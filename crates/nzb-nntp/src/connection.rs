//! NNTP connection state machine.
//!
//! Implements RFC 3977 (Network News Transfer Protocol) over async TCP/TLS.
//!
//! Connection lifecycle:
//! 1. TCP connect -> receive welcome (200/201)
//! 2. AUTH: USER/PASS if credentials provided
//! 3. ARTICLE <message-id> -> receive article data
//! 4. STAT <message-id> -> check article existence
//! 5. QUIT -> close

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tracing::{debug, trace};

use nzb_core::config::ServerConfig;

use crate::error::{NntpError, NntpResult};

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

/// NNTP response: status code + message, optionally with multi-line body.
#[derive(Debug, Clone)]
pub struct NntpResponse {
    /// Three-digit numeric status code (e.g. 200, 220, 430).
    pub code: u16,
    /// Human-readable message from the first response line.
    pub message: String,
    /// Multi-line body data, if any. Dot-stuffing has been undone.
    pub data: Option<Vec<u8>>,
}

impl NntpResponse {
    /// Returns `true` if the response indicates success (2xx).
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.code)
    }

    /// Returns `true` if the response indicates the server wants auth (480).
    pub fn needs_auth(&self) -> bool {
        self.code == 480
    }
}

// ---------------------------------------------------------------------------
// Connection state enum
// ---------------------------------------------------------------------------

/// Current state of an NNTP connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Not connected to any server.
    Disconnected,
    /// TCP/TLS handshake in progress.
    Connecting,
    /// Performing USER/PASS authentication.
    Authenticating,
    /// Authenticated and idle, ready for commands.
    Ready,
    /// Currently sending/receiving article data.
    Busy,
    /// An unrecoverable error occurred; reconnection required.
    Error,
}

// ---------------------------------------------------------------------------
// Transport abstraction
// ---------------------------------------------------------------------------

/// A transport is either a plain TCP stream or a TLS-wrapped stream.
/// We box-erase so `NntpConnection` has a single concrete type.
enum Transport {
    Plain(BufReader<TcpStream>),
    Tls(BufReader<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl Transport {
    /// Read a single `\r\n`-terminated line into `buf`, returning the number
    /// of bytes read (including the delimiter).
    async fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize> {
        match self {
            Transport::Plain(r) => r.read_line(buf).await,
            Transport::Tls(r) => r.read_line(buf).await,
        }
    }

    /// Read bytes until `\r\n` into a `Vec<u8>`. Returns number of bytes.
    async fn read_line_bytes(&mut self, buf: &mut Vec<u8>) -> std::io::Result<usize> {
        match self {
            Transport::Plain(r) => r.read_until(b'\n', buf).await,
            Transport::Tls(r) => r.read_until(b'\n', buf).await,
        }
    }

    /// Write all bytes and flush.
    async fn write_all(&mut self, data: &[u8]) -> std::io::Result<()> {
        match self {
            Transport::Plain(r) => {
                r.get_mut().write_all(data).await?;
                r.get_mut().flush().await
            }
            Transport::Tls(r) => {
                r.get_mut().write_all(data).await?;
                r.get_mut().flush().await
            }
        }
    }

    /// Shut down the write half.
    async fn shutdown(&mut self) -> std::io::Result<()> {
        match self {
            Transport::Plain(r) => r.get_mut().shutdown().await,
            Transport::Tls(r) => r.get_mut().shutdown().await,
        }
    }
}

// ---------------------------------------------------------------------------
// NntpConnection
// ---------------------------------------------------------------------------

/// A single NNTP connection to one server.
pub struct NntpConnection {
    /// Server identifier (matches `ServerConfig::id`).
    pub server_id: String,
    /// Current connection state.
    pub state: ConnectionState,
    /// Underlying transport (set after connect).
    transport: Option<Transport>,
}

impl NntpConnection {
    /// Create a new, disconnected connection for the given server.
    pub fn new(server_id: String) -> Self {
        Self {
            server_id,
            state: ConnectionState::Disconnected,
            transport: None,
        }
    }

    // ------------------------------------------------------------------
    // Connect
    // ------------------------------------------------------------------

    /// Connect to the NNTP server described by `config`.
    ///
    /// This performs TCP connection, optional TLS upgrade, reads the welcome
    /// banner, and authenticates if credentials are configured.
    pub async fn connect(&mut self, config: &ServerConfig) -> NntpResult<()> {
        self.state = ConnectionState::Connecting;

        let addr = format!("{}:{}", config.host, config.port);
        debug!(server = %self.server_id, %addr, ssl = config.ssl, "Connecting");

        // 1. TCP connect
        let tcp = TcpStream::connect(&addr).await.map_err(|e| {
            self.state = ConnectionState::Error;
            NntpError::Connection(format!("TCP connect to {addr}: {e}"))
        })?;
        tcp.set_nodelay(true).ok();

        // 2. Optional TLS
        if config.ssl {
            let tls_config = build_tls_config(config.ssl_verify)?;
            let connector = TlsConnector::from(Arc::new(tls_config));

            let server_name = rustls_pki_types::ServerName::try_from(config.host.clone())
                .map_err(|e| NntpError::Tls(format!("Invalid server name '{}': {e}", config.host)))?;

            let tls_stream = connector.connect(server_name, tcp).await.map_err(|e| {
                self.state = ConnectionState::Error;
                NntpError::Tls(format!("TLS handshake with {addr}: {e}"))
            })?;

            self.transport = Some(Transport::Tls(BufReader::with_capacity(256 * 1024, tls_stream)));
        } else {
            self.transport = Some(Transport::Plain(BufReader::with_capacity(256 * 1024, tcp)));
        }

        // 3. Read welcome banner
        let welcome = self.read_response_line().await?;
        debug!(server = %self.server_id, code = welcome.code, msg = %welcome.message, "Welcome");

        match welcome.code {
            200 | 201 => {} // posting allowed / posting not allowed — both fine
            502 => {
                self.state = ConnectionState::Error;
                return Err(NntpError::ServiceUnavailable(welcome.message));
            }
            _ => {
                self.state = ConnectionState::Error;
                return Err(NntpError::Protocol(format!(
                    "Unexpected welcome code {}: {}",
                    welcome.code, welcome.message
                )));
            }
        }

        // 4. Authenticate if credentials are provided
        if config.username.is_some() {
            self.authenticate(config).await?;
        } else {
            self.state = ConnectionState::Ready;
        }

        debug!(server = %self.server_id, "Connection ready");
        Ok(())
    }

    // ------------------------------------------------------------------
    // Authentication
    // ------------------------------------------------------------------

    /// Perform USER/PASS authentication.
    async fn authenticate(&mut self, config: &ServerConfig) -> NntpResult<()> {
        self.state = ConnectionState::Authenticating;

        let username = config
            .username
            .as_deref()
            .ok_or_else(|| NntpError::Auth("No username configured".into()))?;

        // Try AUTHINFO USER first (RFC 4643), fall back to USER (RFC 2980)
        self.send_command(&format!("AUTHINFO USER {username}")).await?;
        let resp = self.read_response_line().await?;

        match resp.code {
            281 => {
                // Authenticated with just username (unusual but valid)
                self.state = ConnectionState::Ready;
                return Ok(());
            }
            381 | 480 => {
                // 381 = password required (standard)
                // 480 = authentication required (some servers send this to mean "continue")
            }
            482 | 502 => {
                self.state = ConnectionState::Error;
                return Err(NntpError::Auth(format!(
                    "USER rejected ({}): {}",
                    resp.code, resp.message
                )));
            }
            _ => {
                self.state = ConnectionState::Error;
                return Err(NntpError::Protocol(format!(
                    "Unexpected USER response {}: {}",
                    resp.code, resp.message
                )));
            }
        }

        // Send PASS
        let password = config
            .password
            .as_deref()
            .ok_or_else(|| NntpError::Auth("Server requires password but none configured".into()))?;

        self.send_command(&format!("AUTHINFO PASS {password}")).await?;
        let resp = self.read_response_line().await?;

        match resp.code {
            281 => {
                self.state = ConnectionState::Ready;
                Ok(())
            }
            481 => {
                self.state = ConnectionState::Error;
                Err(NntpError::Auth(format!(
                    "Authentication failed: {}",
                    resp.message
                )))
            }
            502 => {
                self.state = ConnectionState::Error;
                Err(NntpError::ServiceUnavailable(resp.message))
            }
            _ => {
                self.state = ConnectionState::Error;
                Err(NntpError::Protocol(format!(
                    "Unexpected PASS response {}: {}",
                    resp.code, resp.message
                )))
            }
        }
    }

    // ------------------------------------------------------------------
    // ARTICLE command
    // ------------------------------------------------------------------

    /// Fetch a complete article by message-id.
    ///
    /// Sends `ARTICLE <message-id>` and reads the multi-line response.
    /// Returns the raw article data (headers + blank line + body).
    pub async fn fetch_article(&mut self, message_id: &str) -> NntpResult<NntpResponse> {
        if self.state != ConnectionState::Ready {
            return Err(NntpError::Protocol(format!(
                "Cannot fetch article in state {:?}",
                self.state
            )));
        }
        self.state = ConnectionState::Busy;

        let mid = normalize_message_id(message_id);
        self.send_command(&format!("ARTICLE {mid}")).await?;

        let status = self.read_response_line().await?;

        match status.code {
            220 => {
                // Article follows — read multi-line body
                let data = self.read_multiline_body().await?;
                self.state = ConnectionState::Ready;
                Ok(NntpResponse {
                    code: status.code,
                    message: status.message,
                    data: Some(data),
                })
            }
            430 => {
                self.state = ConnectionState::Ready;
                Err(NntpError::ArticleNotFound(mid))
            }
            411 => {
                self.state = ConnectionState::Ready;
                Err(NntpError::NoSuchGroup(status.message))
            }
            412 | 420 => {
                self.state = ConnectionState::Ready;
                Err(NntpError::NoArticleSelected(status.message))
            }
            480 => {
                self.state = ConnectionState::Error;
                Err(NntpError::AuthRequired(status.message))
            }
            502 => {
                self.state = ConnectionState::Error;
                Err(NntpError::ServiceUnavailable(status.message))
            }
            _ => {
                self.state = ConnectionState::Error;
                Err(NntpError::Protocol(format!(
                    "Unexpected ARTICLE response {}: {}",
                    status.code, status.message
                )))
            }
        }
    }

    // ------------------------------------------------------------------
    // STAT command (pre-check)
    // ------------------------------------------------------------------

    /// Check if an article exists on the server without downloading it.
    ///
    /// Sends `STAT <message-id>`. Returns `Ok(response)` with code 223 if
    /// the article exists, or an appropriate error.
    pub async fn stat_article(&mut self, message_id: &str) -> NntpResult<NntpResponse> {
        if self.state != ConnectionState::Ready {
            return Err(NntpError::Protocol(format!(
                "Cannot STAT in state {:?}",
                self.state
            )));
        }
        self.state = ConnectionState::Busy;

        let mid = normalize_message_id(message_id);
        self.send_command(&format!("STAT {mid}")).await?;

        let resp = self.read_response_line().await?;
        self.state = ConnectionState::Ready;

        match resp.code {
            223 => Ok(resp),
            430 => Err(NntpError::ArticleNotFound(mid)),
            480 => {
                self.state = ConnectionState::Error;
                Err(NntpError::AuthRequired(resp.message))
            }
            _ => Err(NntpError::Protocol(format!(
                "Unexpected STAT response {}: {}",
                resp.code, resp.message
            ))),
        }
    }

    // ------------------------------------------------------------------
    // QUIT
    // ------------------------------------------------------------------

    /// Send QUIT and close the connection gracefully.
    pub async fn quit(&mut self) -> NntpResult<()> {
        if self.transport.is_some() {
            // Best-effort: send QUIT, ignore errors
            if let Err(e) = self.send_command("QUIT").await {
                debug!(server = %self.server_id, "QUIT send failed (ignored): {e}");
            } else {
                // Try to read the 205 response
                match self.read_response_line().await {
                    Ok(resp) => {
                        trace!(server = %self.server_id, code = resp.code, "QUIT response");
                    }
                    Err(e) => {
                        debug!(server = %self.server_id, "QUIT response read failed (ignored): {e}");
                    }
                }
            }

            // Shut down the socket
            if let Some(ref mut transport) = self.transport {
                let _ = transport.shutdown().await;
            }
        }

        self.transport = None;
        self.state = ConnectionState::Disconnected;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Send a raw ARTICLE command and read status line only (for pipeline)
    // ------------------------------------------------------------------

    /// Send a raw NNTP command followed by `\r\n`. Does not read the response.
    pub(crate) async fn send_command(&mut self, cmd: &str) -> NntpResult<()> {
        let transport = self
            .transport
            .as_mut()
            .ok_or(NntpError::Connection("Not connected".into()))?;

        trace!(server = %self.server_id, cmd = %cmd.split_whitespace().next().unwrap_or(""), ">> NNTP");

        let mut line = cmd.to_string();
        line.push_str("\r\n");
        transport
            .write_all(line.as_bytes())
            .await
            .map_err(|e| NntpError::Io(e))?;
        Ok(())
    }

    /// Read a single response line (status code + message). Public for pipeline use.
    pub(crate) async fn read_response_line(&mut self) -> NntpResult<NntpResponse> {
        let transport = self
            .transport
            .as_mut()
            .ok_or(NntpError::Connection("Not connected".into()))?;

        let mut line = String::with_capacity(256);
        let n = transport
            .read_line(&mut line)
            .await
            .map_err(|e| NntpError::Io(e))?;

        if n == 0 {
            return Err(NntpError::Connection("Server closed connection".into()));
        }

        parse_response_line(&line)
    }

    /// Read a multi-line body terminated by `.\r\n`. Un-does dot-stuffing.
    /// Public for pipeline use.
    pub(crate) async fn read_multiline_body(&mut self) -> NntpResult<Vec<u8>> {
        let transport = self
            .transport
            .as_mut()
            .ok_or(NntpError::Connection("Not connected".into()))?;

        let mut body = Vec::with_capacity(1024 * 1024);
        let mut line_buf: Vec<u8> = Vec::with_capacity(16 * 1024);

        loop {
            line_buf.clear();
            let n = transport
                .read_line_bytes(&mut line_buf)
                .await
                .map_err(|e| NntpError::Io(e))?;

            if n == 0 {
                return Err(NntpError::Connection(
                    "Server closed connection during multi-line read".into(),
                ));
            }

            // Check for termination: a lone dot followed by CRLF
            if line_buf == b".\r\n" || line_buf == b".\n" {
                break;
            }

            // Dot-unstuffing: if a line starts with "..", remove the first dot
            if line_buf.starts_with(b"..") {
                body.extend_from_slice(&line_buf[1..]);
            } else {
                body.extend_from_slice(&line_buf);
            }
        }

        Ok(body)
    }

    /// Returns `true` if the connection has an active transport.
    pub fn is_connected(&self) -> bool {
        self.transport.is_some() && self.state != ConnectionState::Disconnected
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a single NNTP response line into code + message.
fn parse_response_line(line: &str) -> NntpResult<NntpResponse> {
    let trimmed = line.trim_end_matches(['\r', '\n']);
    if trimmed.len() < 3 {
        return Err(NntpError::Protocol(format!(
            "Response line too short: {trimmed:?}"
        )));
    }

    let code: u16 = trimmed[..3].parse().map_err(|_| {
        NntpError::Protocol(format!("Invalid response code in: {trimmed:?}"))
    })?;

    let message = if trimmed.len() > 4 {
        trimmed[4..].to_string()
    } else {
        String::new()
    };

    Ok(NntpResponse {
        code,
        message,
        data: None,
    })
}

/// Ensure message-id is wrapped in angle brackets.
fn normalize_message_id(mid: &str) -> String {
    if mid.starts_with('<') && mid.ends_with('>') {
        mid.to_string()
    } else {
        format!("<{mid}>")
    }
}

/// Build a `rustls::ClientConfig` for NNTP TLS connections.
fn build_tls_config(verify_certs: bool) -> NntpResult<rustls::ClientConfig> {
    if verify_certs {
        let mut root_store = rustls::RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        let config = rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        Ok(config)
    } else {
        // Dangerous: skip certificate verification (user opted out)
        let config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerifier))
            .with_no_client_auth();
        Ok(config)
    }
}

/// A certificate verifier that accepts any certificate (for `ssl_verify: false`).
#[derive(Debug)]
struct NoVerifier;

impl rustls::client::danger::ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls_pki_types::CertificateDer<'_>,
        _intermediates: &[rustls_pki_types::CertificateDer<'_>],
        _server_name: &rustls_pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls_pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::aws_lc_rs::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_response_line() {
        let resp = parse_response_line("200 NNTP Service Ready\r\n").unwrap();
        assert_eq!(resp.code, 200);
        assert_eq!(resp.message, "NNTP Service Ready");
    }

    #[test]
    fn test_parse_response_line_no_message() {
        let resp = parse_response_line("200\r\n").unwrap();
        assert_eq!(resp.code, 200);
        assert_eq!(resp.message, "");
    }

    #[test]
    fn test_normalize_message_id() {
        assert_eq!(
            normalize_message_id("abc@example.com"),
            "<abc@example.com>"
        );
        assert_eq!(
            normalize_message_id("<abc@example.com>"),
            "<abc@example.com>"
        );
    }
}
