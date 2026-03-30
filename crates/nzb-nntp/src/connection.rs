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
use tokio_socks::tcp::Socks5Stream;
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
// GROUP response
// ---------------------------------------------------------------------------

/// Response from the GROUP command (RFC 3977 Section 6.1.1).
#[derive(Debug, Clone)]
pub struct GroupResponse {
    /// Estimated number of articles in the group.
    pub count: u64,
    /// Lowest article number.
    pub first: u64,
    /// Highest article number.
    pub last: u64,
    /// Group name (echoed back by server).
    pub name: String,
}

// ---------------------------------------------------------------------------
// XOVER entry
// ---------------------------------------------------------------------------

/// A single entry from an XOVER/OVER response.
/// Fields correspond to the overview.fmt standard (RFC 2980 Section 3.1.1):
/// article_num \t subject \t from \t date \t message-id \t references \t bytes \t lines
#[derive(Debug, Clone)]
pub struct XoverEntry {
    pub article_num: u64,
    pub subject: String,
    pub from: String,
    pub date: String,
    pub message_id: String,
    pub references: String,
    pub bytes: u64,
    pub lines: u64,
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
    Tls(Box<BufReader<tokio_rustls::client::TlsStream<TcpStream>>>),
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
    /// Whether XFEATURE COMPRESS GZIP is active on this connection.
    compress_enabled: bool,
}

impl NntpConnection {
    /// Create a new, disconnected connection for the given server.
    pub fn new(server_id: String) -> Self {
        Self {
            server_id,
            state: ConnectionState::Disconnected,
            transport: None,
            compress_enabled: false,
        }
    }

    /// Returns `true` if gzip compression was negotiated on this connection.
    pub fn is_compress_enabled(&self) -> bool {
        self.compress_enabled
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

        // 1. TCP connect (optionally through SOCKS5 proxy)
        let tcp = if let Some(proxy_url) = &config.proxy_url {
            let proxy = parse_socks5_url(proxy_url).map_err(|e| {
                self.state = ConnectionState::Error;
                NntpError::Connection(format!("Invalid proxy URL: {e}"))
            })?;
            debug!(server = %self.server_id, proxy = %proxy.addr, "Connecting via SOCKS5 proxy");
            let stream = if let Some((user, pass)) = &proxy.auth {
                Socks5Stream::connect_with_password(proxy.addr.as_str(), addr.as_str(), user, pass)
                    .await
            } else {
                Socks5Stream::connect(proxy.addr.as_str(), addr.as_str()).await
            };
            stream
                .map_err(|e| {
                    self.state = ConnectionState::Error;
                    NntpError::Connection(format!("SOCKS5 connect to {addr} via proxy: {e}"))
                })?
                .into_inner()
        } else {
            TcpStream::connect(&addr).await.map_err(|e| {
                self.state = ConnectionState::Error;
                NntpError::Connection(format!("TCP connect to {addr}: {e}"))
            })?
        };
        tcp.set_nodelay(true).ok();

        // 2. Optional TLS
        if config.ssl {
            let tls_config = build_tls_config(config.ssl_verify)?;
            let connector = TlsConnector::from(Arc::new(tls_config));

            let server_name =
                rustls_pki_types::ServerName::try_from(config.host.clone()).map_err(|e| {
                    NntpError::Tls(format!("Invalid server name '{}': {e}", config.host))
                })?;

            let tls_stream = connector.connect(server_name, tcp).await.map_err(|e| {
                self.state = ConnectionState::Error;
                NntpError::Tls(format!("TLS handshake with {addr}: {e}"))
            })?;

            self.transport = Some(Transport::Tls(Box::new(BufReader::with_capacity(
                256 * 1024,
                tls_stream,
            ))));
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

        // 5. Negotiate compression if configured
        if config.compress
            && let Err(e) = self.negotiate_compression().await
        {
            debug!(server = %self.server_id, error = %e, "Compression negotiation failed, continuing without");
        }

        debug!(server = %self.server_id, compress = self.compress_enabled, "Connection ready");
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
        self.send_command(&format!("AUTHINFO USER {username}"))
            .await?;
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
            481 | 482 => {
                // 481 = credentials rejected (RFC 4643)
                // 482 = non-standard but used by providers for block/account exhausted
                self.state = ConnectionState::Error;
                return Err(NntpError::Auth(format!(
                    "USER rejected ({}): {}",
                    resp.code, resp.message
                )));
            }
            502 => {
                self.state = ConnectionState::Error;
                return Err(NntpError::ServiceUnavailable(resp.message));
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
        let password = config.password.as_deref().ok_or_else(|| {
            NntpError::Auth("Server requires password but none configured".into())
        })?;

        self.send_command(&format!("AUTHINFO PASS {password}"))
            .await?;
        let resp = self.read_response_line().await?;

        match resp.code {
            281 => {
                self.state = ConnectionState::Ready;
                Ok(())
            }
            481 | 482 => {
                // 481 = credentials rejected (RFC 4643)
                // 482 = non-standard but used by providers for block/account exhausted
                self.state = ConnectionState::Error;
                Err(NntpError::Auth(format!(
                    "PASS rejected ({}): {}",
                    resp.code, resp.message
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
    // XFEATURE COMPRESS GZIP negotiation
    // ------------------------------------------------------------------

    /// Negotiate XFEATURE COMPRESS GZIP with the server.
    ///
    /// Sends LIST EXTENSIONS to check support, then enables compression
    /// if the server advertises it. Sets `compress_enabled` on success.
    async fn negotiate_compression(&mut self) -> NntpResult<()> {
        // Check server capabilities via LIST EXTENSIONS
        self.send_command("LIST EXTENSIONS").await?;
        let resp = self.read_response_line().await?;

        if resp.code == 202 {
            let data = self.read_multiline_body().await?;
            let text = String::from_utf8_lossy(&data);
            let supports_compress = text
                .lines()
                .any(|line| line.trim().eq_ignore_ascii_case("XFEATURE COMPRESS GZIP"));

            if !supports_compress {
                debug!(server = %self.server_id, "Server does not advertise XFEATURE COMPRESS GZIP");
                return Ok(());
            }
        } else {
            debug!(server = %self.server_id, code = resp.code, "LIST EXTENSIONS not supported");
            return Ok(());
        }

        // Server advertises support — enable it
        self.send_command("XFEATURE COMPRESS GZIP").await?;
        let resp = self.read_response_line().await?;

        if resp.code == 290 {
            self.compress_enabled = true;
            debug!(server = %self.server_id, "GZIP compression enabled");
        } else {
            debug!(server = %self.server_id, code = resp.code, "XFEATURE COMPRESS GZIP rejected");
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // Decompression helper
    // ------------------------------------------------------------------

    /// Read a multi-line body, decompressing if gzip compression is active.
    ///
    /// Detects gzip data by checking for the magic bytes (0x1f, 0x8b).
    /// Falls back to raw data if decompression fails (some servers only
    /// compress certain response types).
    async fn read_multiline_body_maybe_decompress(&mut self) -> NntpResult<Vec<u8>> {
        let raw = self.read_multiline_body().await?;

        if self.compress_enabled && raw.len() >= 2 && raw[0] == 0x1f && raw[1] == 0x8b {
            use flate2::read::GzDecoder;
            use std::io::Read;

            let mut decoder = GzDecoder::new(&raw[..]);
            let mut decompressed = Vec::with_capacity(raw.len() * 4);
            match decoder.read_to_end(&mut decompressed) {
                Ok(_) => {
                    trace!(
                        server = %self.server_id,
                        compressed = raw.len(),
                        decompressed = decompressed.len(),
                        "Decompressed gzip response"
                    );
                    Ok(decompressed)
                }
                Err(e) => {
                    debug!(
                        server = %self.server_id,
                        error = %e,
                        "Gzip decode failed, using raw data"
                    );
                    Ok(raw)
                }
            }
        } else {
            Ok(raw)
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
                let data = self.read_multiline_body_maybe_decompress().await?;
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
            481 | 482 => {
                self.state = ConnectionState::Error;
                Err(NntpError::Auth(format!(
                    "ARTICLE rejected ({}): {}",
                    status.code, status.message
                )))
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
            481 | 482 => {
                self.state = ConnectionState::Error;
                Err(NntpError::Auth(format!(
                    "STAT rejected ({}): {}",
                    resp.code, resp.message
                )))
            }
            _ => Err(NntpError::Protocol(format!(
                "Unexpected STAT response {}: {}",
                resp.code, resp.message
            ))),
        }
    }

    // ------------------------------------------------------------------
    // GROUP command (RFC 3977 Section 6.1.1)
    // ------------------------------------------------------------------

    /// Select a newsgroup and return its article range.
    ///
    /// Sends `GROUP <name>` and parses the `211` response:
    /// `211 count first last name`
    pub async fn group(&mut self, name: &str) -> NntpResult<GroupResponse> {
        if self.state != ConnectionState::Ready {
            return Err(NntpError::Protocol(format!(
                "Cannot GROUP in state {:?}",
                self.state
            )));
        }
        self.state = ConnectionState::Busy;

        self.send_command(&format!("GROUP {name}")).await?;
        let resp = self.read_response_line().await?;

        self.state = ConnectionState::Ready;

        match resp.code {
            211 => {
                let parts: Vec<&str> = resp.message.split_whitespace().collect();
                if parts.len() < 3 {
                    return Err(NntpError::Protocol(format!(
                        "Malformed GROUP response: {}",
                        resp.message
                    )));
                }
                Ok(GroupResponse {
                    count: parts[0].parse().unwrap_or(0),
                    first: parts[1].parse().unwrap_or(0),
                    last: parts[2].parse().unwrap_or(0),
                    name: parts.get(3).unwrap_or(&name).to_string(),
                })
            }
            411 => Err(NntpError::NoSuchGroup(name.to_string())),
            480 => {
                self.state = ConnectionState::Error;
                Err(NntpError::AuthRequired(resp.message))
            }
            481 | 482 => {
                self.state = ConnectionState::Error;
                Err(NntpError::Auth(format!(
                    "GROUP rejected ({}): {}",
                    resp.code, resp.message
                )))
            }
            502 => {
                self.state = ConnectionState::Error;
                Err(NntpError::ServiceUnavailable(resp.message))
            }
            _ => {
                self.state = ConnectionState::Error;
                Err(NntpError::Protocol(format!(
                    "Unexpected GROUP response {}: {}",
                    resp.code, resp.message
                )))
            }
        }
    }

    // ------------------------------------------------------------------
    // XOVER command (RFC 2980 Section 2.8)
    // ------------------------------------------------------------------

    /// Fetch overview data for a range of article numbers.
    ///
    /// Sends `XOVER start-end` and parses the tab-delimited multi-line response.
    /// Response code 224 means overview data follows (dot-terminated).
    pub async fn xover(&mut self, start: u64, end: u64) -> NntpResult<Vec<XoverEntry>> {
        if self.state != ConnectionState::Ready {
            return Err(NntpError::Protocol(format!(
                "Cannot XOVER in state {:?}",
                self.state
            )));
        }
        self.state = ConnectionState::Busy;

        self.send_command(&format!("XOVER {start}-{end}")).await?;
        let status = self.read_response_line().await?;

        match status.code {
            224 => {
                let data = self.read_multiline_body_maybe_decompress().await?;
                self.state = ConnectionState::Ready;
                Ok(parse_xover_data(&data))
            }
            420 => {
                self.state = ConnectionState::Ready;
                Ok(Vec::new()) // No articles in range
            }
            412 => {
                self.state = ConnectionState::Ready;
                Err(NntpError::NoSuchGroup(
                    "No newsgroup selected (send GROUP first)".into(),
                ))
            }
            481 | 482 => {
                self.state = ConnectionState::Error;
                Err(NntpError::Auth(format!(
                    "XOVER rejected ({}): {}",
                    status.code, status.message
                )))
            }
            502 => {
                self.state = ConnectionState::Error;
                Err(NntpError::ServiceUnavailable(status.message))
            }
            _ => {
                self.state = ConnectionState::Error;
                Err(NntpError::Protocol(format!(
                    "Unexpected XOVER response {}: {}",
                    status.code, status.message
                )))
            }
        }
    }

    // ------------------------------------------------------------------
    // BODY command
    // ------------------------------------------------------------------

    /// Fetch article body by message-id (headers excluded).
    ///
    /// Sends `BODY <message-id>` and returns the raw body data.
    pub async fn fetch_body(&mut self, message_id: &str) -> NntpResult<NntpResponse> {
        if self.state != ConnectionState::Ready {
            return Err(NntpError::Protocol(format!(
                "Cannot BODY in state {:?}",
                self.state
            )));
        }
        self.state = ConnectionState::Busy;

        let mid = normalize_message_id(message_id);
        self.send_command(&format!("BODY {mid}")).await?;
        let status = self.read_response_line().await?;

        match status.code {
            222 => {
                let data = self.read_multiline_body_maybe_decompress().await?;
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
            412 | 420 => {
                self.state = ConnectionState::Ready;
                Err(NntpError::NoArticleSelected(status.message))
            }
            480 => {
                self.state = ConnectionState::Error;
                Err(NntpError::AuthRequired(status.message))
            }
            481 | 482 => {
                self.state = ConnectionState::Error;
                Err(NntpError::Auth(format!(
                    "BODY rejected ({}): {}",
                    status.code, status.message
                )))
            }
            502 => {
                self.state = ConnectionState::Error;
                Err(NntpError::ServiceUnavailable(status.message))
            }
            _ => {
                self.state = ConnectionState::Error;
                Err(NntpError::Protocol(format!(
                    "Unexpected BODY response {}: {}",
                    status.code, status.message
                )))
            }
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
            .map_err(NntpError::Io)?;
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
            .map_err(NntpError::Io)?;

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
                .map_err(NntpError::Io)?;

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

    let code: u16 = trimmed[..3]
        .parse()
        .map_err(|_| NntpError::Protocol(format!("Invalid response code in: {trimmed:?}")))?;

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

/// Parse XOVER multi-line body into structured entries.
/// Each line is tab-delimited:
/// article_num \t subject \t from \t date \t message-id \t references \t bytes \t lines
fn parse_xover_data(data: &[u8]) -> Vec<XoverEntry> {
    let text = String::from_utf8_lossy(data);
    let mut entries = Vec::new();

    for line in text.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 8 {
            continue; // Malformed line, skip
        }
        let message_id = parts[4].trim_matches(|c| c == '<' || c == '>').to_string();
        entries.push(XoverEntry {
            article_num: parts[0].parse().unwrap_or(0),
            subject: parts[1].to_string(),
            from: parts[2].to_string(),
            date: parts[3].to_string(),
            message_id,
            references: parts[5].to_string(),
            bytes: parts[6].parse().unwrap_or(0),
            lines: parts[7].trim().parse().unwrap_or(0),
        });
    }

    entries
}

/// Parsed SOCKS5 proxy URL components.
struct Socks5Proxy {
    addr: String,
    auth: Option<(String, String)>,
}

/// Parse a SOCKS5 proxy URL: `socks5://[username:password@]host:port`
fn parse_socks5_url(url: &str) -> Result<Socks5Proxy, String> {
    let rest = url
        .strip_prefix("socks5://")
        .ok_or_else(|| format!("proxy URL must start with socks5://, got: {url}"))?;

    let (auth, host_port) = if let Some(at_pos) = rest.rfind('@') {
        let auth_part = &rest[..at_pos];
        let host_part = &rest[at_pos + 1..];
        let (user, pass) = auth_part
            .split_once(':')
            .ok_or_else(|| "proxy auth must be username:password".to_string())?;
        (
            Some((user.to_string(), pass.to_string())),
            host_part.to_string(),
        )
    } else {
        (None, rest.to_string())
    };

    if host_port.is_empty() {
        return Err("proxy URL must contain host:port".to_string());
    }

    Ok(Socks5Proxy {
        addr: host_port,
        auth,
    })
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
    use crate::testutil::{MockConfig, MockNntpServer, test_config, test_config_with_auth};
    use std::collections::HashMap;

    // -----------------------------------------------------------------------
    // Pure helper function tests (existing)
    // -----------------------------------------------------------------------

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
    fn test_parse_response_line_too_short() {
        let err = parse_response_line("20\r\n");
        assert!(err.is_err());
    }

    #[test]
    fn test_parse_response_line_invalid_code() {
        let err = parse_response_line("ABC some message\r\n");
        assert!(err.is_err());
    }

    #[test]
    fn test_normalize_message_id() {
        assert_eq!(normalize_message_id("abc@example.com"), "<abc@example.com>");
        assert_eq!(
            normalize_message_id("<abc@example.com>"),
            "<abc@example.com>"
        );
    }

    #[test]
    fn test_parse_xover_data() {
        let data = b"123456\tSubject line\tposter@example.com\tMon, 01 Jan 2024 00:00:00 UTC\t<msg-id@host>\t\t768000\t1000\r\n";
        let entries = parse_xover_data(data);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].article_num, 123456);
        assert_eq!(entries[0].subject, "Subject line");
        assert_eq!(entries[0].from, "poster@example.com");
        assert_eq!(entries[0].message_id, "msg-id@host");
        assert_eq!(entries[0].bytes, 768000);
        assert_eq!(entries[0].lines, 1000);
    }

    #[test]
    fn test_parse_xover_strips_angle_brackets() {
        let data = b"1\tSubj\tPoster\tDate\t<abc@def.com>\t\t100\t10\r\n";
        let entries = parse_xover_data(data);
        assert_eq!(entries[0].message_id, "abc@def.com");
    }

    #[test]
    fn test_parse_xover_skips_malformed_lines() {
        let data = b"too\tfew\tfields\r\n123\tSubj\tFrom\tDate\t<mid@x>\t\t500\t50\r\n";
        let entries = parse_xover_data(data);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].article_num, 123);
    }

    #[test]
    fn test_parse_xover_multiple_entries() {
        let data =
            b"100\tS1\tF1\tD1\t<m1@x>\t\t1000\t10\r\n200\tS2\tF2\tD2\t<m2@x>\tref\t2000\t20\r\n";
        let entries = parse_xover_data(data);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].article_num, 100);
        assert_eq!(entries[1].article_num, 200);
        assert_eq!(entries[1].references, "ref");
    }

    #[test]
    fn test_parse_xover_empty() {
        let entries = parse_xover_data(b"");
        assert!(entries.is_empty());
    }

    // -----------------------------------------------------------------------
    // NntpResponse helper tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_response_is_success() {
        assert!(
            NntpResponse {
                code: 200,
                message: "OK".into(),
                data: None
            }
            .is_success()
        );
        assert!(
            NntpResponse {
                code: 220,
                message: "OK".into(),
                data: None
            }
            .is_success()
        );
        assert!(
            NntpResponse {
                code: 281,
                message: "OK".into(),
                data: None
            }
            .is_success()
        );
        assert!(
            !NntpResponse {
                code: 430,
                message: "Not found".into(),
                data: None
            }
            .is_success()
        );
        assert!(
            !NntpResponse {
                code: 502,
                message: "Err".into(),
                data: None
            }
            .is_success()
        );
    }

    #[test]
    fn test_response_needs_auth() {
        assert!(
            NntpResponse {
                code: 480,
                message: "Auth".into(),
                data: None
            }
            .needs_auth()
        );
        assert!(
            !NntpResponse {
                code: 200,
                message: "OK".into(),
                data: None
            }
            .needs_auth()
        );
    }

    // -----------------------------------------------------------------------
    // NntpConnection unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_new_connection_state() {
        let conn = NntpConnection::new("test-1".into());
        assert_eq!(conn.server_id, "test-1");
        assert_eq!(conn.state, ConnectionState::Disconnected);
        assert!(!conn.is_connected());
    }

    // -----------------------------------------------------------------------
    // Mock server integration tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_connect_plain() {
        let server = MockNntpServer::start(MockConfig::default()).await;
        let config = test_config(server.port());
        let mut conn = NntpConnection::new("test".into());

        conn.connect(&config).await.unwrap();
        assert_eq!(conn.state, ConnectionState::Ready);
        assert!(conn.is_connected());
    }

    #[tokio::test]
    async fn test_connect_read_only_server() {
        let server = MockNntpServer::start(MockConfig {
            welcome_code: 201,
            welcome_message: "Read-only".into(),
            ..MockConfig::default()
        })
        .await;
        let config = test_config(server.port());
        let mut conn = NntpConnection::new("test".into());

        conn.connect(&config).await.unwrap();
        assert_eq!(conn.state, ConnectionState::Ready);
    }

    #[tokio::test]
    async fn test_connect_with_auth() {
        let server = MockNntpServer::start(MockConfig {
            auth_required: true,
            valid_credentials: Some(("myuser".into(), "mypass".into())),
            ..MockConfig::default()
        })
        .await;
        let config = test_config_with_auth(server.port(), "myuser", "mypass");
        let mut conn = NntpConnection::new("test".into());

        conn.connect(&config).await.unwrap();
        assert_eq!(conn.state, ConnectionState::Ready);
    }

    #[tokio::test]
    async fn test_connect_auth_wrong_password() {
        let server = MockNntpServer::start(MockConfig {
            auth_required: true,
            valid_credentials: Some(("myuser".into(), "correct".into())),
            ..MockConfig::default()
        })
        .await;
        let config = test_config_with_auth(server.port(), "myuser", "wrong");
        let mut conn = NntpConnection::new("test".into());

        let result = conn.connect(&config).await;
        assert!(result.is_err());
        assert_eq!(conn.state, ConnectionState::Error);
    }

    #[tokio::test]
    async fn test_connect_auth_rejected() {
        let server = MockNntpServer::start(MockConfig {
            auth_required: true,
            fail_auth: true,
            ..MockConfig::default()
        })
        .await;
        let config = test_config_with_auth(server.port(), "user", "pass");
        let mut conn = NntpConnection::new("test".into());

        let result = conn.connect(&config).await;
        assert!(result.is_err());
        assert_eq!(conn.state, ConnectionState::Error);
    }

    #[tokio::test]
    async fn test_connect_service_unavailable() {
        let server = MockNntpServer::start(MockConfig {
            service_unavailable: true,
            ..MockConfig::default()
        })
        .await;
        let config = test_config(server.port());
        let mut conn = NntpConnection::new("test".into());

        let result = conn.connect(&config).await;
        assert!(result.is_err());
        assert_eq!(conn.state, ConnectionState::Error);
    }

    #[tokio::test]
    async fn test_connect_refused() {
        // Connect to a port with nothing listening
        let config = test_config(19999);
        let mut conn = NntpConnection::new("test".into());

        let result = conn.connect(&config).await;
        assert!(result.is_err());
        assert_eq!(conn.state, ConnectionState::Error);
    }

    #[tokio::test]
    async fn test_group_success() {
        let mut groups = HashMap::new();
        groups.insert("alt.binaries.test".into(), (5000u64, 1u64, 5000u64));

        let server = MockNntpServer::start(MockConfig {
            groups,
            ..MockConfig::default()
        })
        .await;
        let config = test_config(server.port());
        let mut conn = NntpConnection::new("test".into());
        conn.connect(&config).await.unwrap();

        let group = conn.group("alt.binaries.test").await.unwrap();
        assert_eq!(group.count, 5000);
        assert_eq!(group.first, 1);
        assert_eq!(group.last, 5000);
        assert_eq!(group.name, "alt.binaries.test");
        assert_eq!(conn.state, ConnectionState::Ready);
    }

    #[tokio::test]
    async fn test_group_not_found() {
        let server = MockNntpServer::start(MockConfig::default()).await;
        let config = test_config(server.port());
        let mut conn = NntpConnection::new("test".into());
        conn.connect(&config).await.unwrap();

        let result = conn.group("nonexistent.group").await;
        assert!(matches!(
            result,
            Err(crate::error::NntpError::NoSuchGroup(_))
        ));
        assert_eq!(conn.state, ConnectionState::Ready);
    }

    #[tokio::test]
    async fn test_xover_success() {
        let mut groups = HashMap::new();
        groups.insert("alt.binaries.test".into(), (100u64, 1u64, 100u64));

        let xover_entries = vec![
            "1\tTest Subject 1\tposter@test.com\tMon, 01 Jan 2024\t<art1@test>\t\t50000\t100"
                .into(),
            "2\tTest Subject 2\tposter@test.com\tMon, 01 Jan 2024\t<art2@test>\t\t60000\t120"
                .into(),
        ];

        let server = MockNntpServer::start(MockConfig {
            groups,
            xover_entries,
            ..MockConfig::default()
        })
        .await;
        let config = test_config(server.port());
        let mut conn = NntpConnection::new("test".into());
        conn.connect(&config).await.unwrap();

        conn.group("alt.binaries.test").await.unwrap();
        let entries = conn.xover(1, 100).await.unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].article_num, 1);
        assert_eq!(entries[0].subject, "Test Subject 1");
        assert_eq!(entries[0].message_id, "art1@test");
        assert_eq!(entries[0].bytes, 50000);
        assert_eq!(entries[1].article_num, 2);
        assert_eq!(entries[1].bytes, 60000);
        assert_eq!(conn.state, ConnectionState::Ready);
    }

    #[tokio::test]
    async fn test_xover_empty_range() {
        let mut groups = HashMap::new();
        groups.insert("alt.binaries.test".into(), (100u64, 1u64, 100u64));

        let server = MockNntpServer::start(MockConfig {
            groups,
            xover_entries: Vec::new(),
            ..MockConfig::default()
        })
        .await;
        let config = test_config(server.port());
        let mut conn = NntpConnection::new("test".into());
        conn.connect(&config).await.unwrap();

        conn.group("alt.binaries.test").await.unwrap();
        let entries = conn.xover(1, 100).await.unwrap();
        assert!(entries.is_empty());
        assert_eq!(conn.state, ConnectionState::Ready);
    }

    #[tokio::test]
    async fn test_xover_no_group_selected() {
        let server = MockNntpServer::start(MockConfig::default()).await;
        let config = test_config(server.port());
        let mut conn = NntpConnection::new("test".into());
        conn.connect(&config).await.unwrap();

        let result = conn.xover(1, 100).await;
        assert!(matches!(
            result,
            Err(crate::error::NntpError::NoSuchGroup(_))
        ));
    }

    #[tokio::test]
    async fn test_fetch_article_success() {
        let mut articles = HashMap::new();
        articles.insert("art1@test".into(), b"This is article body data".to_vec());

        let server = MockNntpServer::start(MockConfig {
            articles,
            ..MockConfig::default()
        })
        .await;
        let config = test_config(server.port());
        let mut conn = NntpConnection::new("test".into());
        conn.connect(&config).await.unwrap();

        let response = conn.fetch_article("art1@test").await.unwrap();
        assert_eq!(response.code, 220);
        let data = response.data.unwrap();
        let body = String::from_utf8_lossy(&data);
        assert!(body.contains("This is article body data"));
        assert_eq!(conn.state, ConnectionState::Ready);
    }

    #[tokio::test]
    async fn test_fetch_article_not_found() {
        let server = MockNntpServer::start(MockConfig::default()).await;
        let config = test_config(server.port());
        let mut conn = NntpConnection::new("test".into());
        conn.connect(&config).await.unwrap();

        let result = conn.fetch_article("nonexistent@test").await;
        assert!(matches!(
            result,
            Err(crate::error::NntpError::ArticleNotFound(_))
        ));
        assert_eq!(conn.state, ConnectionState::Ready);
    }

    #[tokio::test]
    async fn test_fetch_body_success() {
        let mut articles = HashMap::new();
        articles.insert("body1@test".into(), b"Body content here".to_vec());

        let server = MockNntpServer::start(MockConfig {
            articles,
            ..MockConfig::default()
        })
        .await;
        let config = test_config(server.port());
        let mut conn = NntpConnection::new("test".into());
        conn.connect(&config).await.unwrap();

        let response = conn.fetch_body("body1@test").await.unwrap();
        assert_eq!(response.code, 222);
        let data = response.data.unwrap();
        let body = String::from_utf8_lossy(&data);
        assert!(body.contains("Body content here"));
        assert_eq!(conn.state, ConnectionState::Ready);
    }

    #[tokio::test]
    async fn test_fetch_body_not_found() {
        let server = MockNntpServer::start(MockConfig::default()).await;
        let config = test_config(server.port());
        let mut conn = NntpConnection::new("test".into());
        conn.connect(&config).await.unwrap();

        let result = conn.fetch_body("missing@test").await;
        assert!(matches!(
            result,
            Err(crate::error::NntpError::ArticleNotFound(_))
        ));
        assert_eq!(conn.state, ConnectionState::Ready);
    }

    #[tokio::test]
    async fn test_stat_article_exists() {
        let mut articles = HashMap::new();
        articles.insert("stat1@test".into(), b"data".to_vec());

        let server = MockNntpServer::start(MockConfig {
            articles,
            ..MockConfig::default()
        })
        .await;
        let config = test_config(server.port());
        let mut conn = NntpConnection::new("test".into());
        conn.connect(&config).await.unwrap();

        let response = conn.stat_article("stat1@test").await.unwrap();
        assert_eq!(response.code, 223);
        assert_eq!(conn.state, ConnectionState::Ready);
    }

    #[tokio::test]
    async fn test_stat_article_not_found() {
        let server = MockNntpServer::start(MockConfig::default()).await;
        let config = test_config(server.port());
        let mut conn = NntpConnection::new("test".into());
        conn.connect(&config).await.unwrap();

        let result = conn.stat_article("missing@test").await;
        assert!(matches!(
            result,
            Err(crate::error::NntpError::ArticleNotFound(_))
        ));
        assert_eq!(conn.state, ConnectionState::Ready);
    }

    #[tokio::test]
    async fn test_quit_graceful() {
        let server = MockNntpServer::start(MockConfig::default()).await;
        let config = test_config(server.port());
        let mut conn = NntpConnection::new("test".into());
        conn.connect(&config).await.unwrap();
        assert!(conn.is_connected());

        conn.quit().await.unwrap();
        assert_eq!(conn.state, ConnectionState::Disconnected);
        assert!(!conn.is_connected());
    }

    #[tokio::test]
    async fn test_quit_when_not_connected() {
        let mut conn = NntpConnection::new("test".into());
        // Should not error even when not connected
        conn.quit().await.unwrap();
        assert_eq!(conn.state, ConnectionState::Disconnected);
    }

    #[tokio::test]
    async fn test_command_in_wrong_state() {
        let mut conn = NntpConnection::new("test".into());
        // All commands should fail when disconnected (no transport)
        let result = conn.fetch_article("test@msg").await;
        assert!(matches!(result, Err(crate::error::NntpError::Protocol(_))));

        let result = conn.fetch_body("test@msg").await;
        assert!(matches!(result, Err(crate::error::NntpError::Protocol(_))));

        let result = conn.stat_article("test@msg").await;
        assert!(matches!(result, Err(crate::error::NntpError::Protocol(_))));

        let result = conn.group("test.group").await;
        assert!(matches!(result, Err(crate::error::NntpError::Protocol(_))));

        let result = conn.xover(1, 10).await;
        assert!(matches!(result, Err(crate::error::NntpError::Protocol(_))));
    }

    #[test]
    fn test_parse_socks5_url_basic() {
        let proxy = parse_socks5_url("socks5://127.0.0.1:1080").unwrap();
        assert_eq!(proxy.addr, "127.0.0.1:1080");
        assert!(proxy.auth.is_none());
    }

    #[test]
    fn test_parse_socks5_url_with_auth() {
        let proxy = parse_socks5_url("socks5://user:pass@proxy.example.com:8388").unwrap();
        assert_eq!(proxy.addr, "proxy.example.com:8388");
        let (user, pass) = proxy.auth.unwrap();
        assert_eq!(user, "user");
        assert_eq!(pass, "pass");
    }

    #[test]
    fn test_parse_socks5_url_invalid_scheme() {
        assert!(parse_socks5_url("http://127.0.0.1:1080").is_err());
    }

    #[test]
    fn test_parse_socks5_url_empty_host() {
        assert!(parse_socks5_url("socks5://").is_err());
    }

    #[tokio::test]
    async fn test_multiple_commands_sequentially() {
        let mut articles = HashMap::new();
        articles.insert("a1@test".into(), b"data1".to_vec());
        articles.insert("a2@test".into(), b"data2".to_vec());

        let mut groups = HashMap::new();
        groups.insert("alt.test".into(), (100u64, 1u64, 100u64));

        let server = MockNntpServer::start(MockConfig {
            articles,
            groups,
            ..MockConfig::default()
        })
        .await;
        let config = test_config(server.port());
        let mut conn = NntpConnection::new("test".into());
        conn.connect(&config).await.unwrap();

        // GROUP
        let group = conn.group("alt.test").await.unwrap();
        assert_eq!(group.count, 100);

        // STAT
        let stat = conn.stat_article("a1@test").await.unwrap();
        assert_eq!(stat.code, 223);

        // ARTICLE
        let art = conn.fetch_article("a1@test").await.unwrap();
        assert_eq!(art.code, 220);

        // BODY
        let body = conn.fetch_body("a2@test").await.unwrap();
        assert_eq!(body.code, 222);

        // STAT not found
        let result = conn.stat_article("missing@test").await;
        assert!(result.is_err());

        // Connection should still be ready after non-fatal error
        assert_eq!(conn.state, ConnectionState::Ready);

        // QUIT
        conn.quit().await.unwrap();
        assert_eq!(conn.state, ConnectionState::Disconnected);
    }
}
