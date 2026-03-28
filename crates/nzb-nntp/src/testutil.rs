//! Test utilities: in-process mock NNTP server.
//!
//! Provides a configurable NNTP server that runs as a tokio task for unit testing.
//! Supports: AUTH, GROUP, XOVER, BODY, ARTICLE, STAT, QUIT with configurable
//! responses and error injection.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

use nzb_core::config::ServerConfig;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the mock NNTP server.
#[derive(Clone)]
pub struct MockConfig {
    /// Welcome banner code (200 = posting allowed, 201 = read-only).
    pub welcome_code: u16,
    /// Welcome banner message.
    pub welcome_message: String,
    /// Whether auth is required before commands.
    pub auth_required: bool,
    /// Valid credentials. None = accept any credentials.
    pub valid_credentials: Option<(String, String)>,
    /// If true, authentication always fails (482 on USER).
    pub fail_auth: bool,
    /// If true, server sends 502 on connect.
    pub service_unavailable: bool,
    /// Groups: name -> (count, first, last).
    pub groups: HashMap<String, (u64, u64, u64)>,
    /// Articles: message-id (without angle brackets) -> body bytes.
    pub articles: HashMap<String, Vec<u8>>,
    /// XOVER entries as pre-formatted tab-delimited lines.
    pub xover_entries: Vec<String>,
}

impl Default for MockConfig {
    fn default() -> Self {
        Self {
            welcome_code: 200,
            welcome_message: "Mock NNTP Ready".into(),
            auth_required: false,
            valid_credentials: None,
            fail_auth: false,
            service_unavailable: false,
            groups: HashMap::new(),
            articles: HashMap::new(),
            xover_entries: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Mock server
// ---------------------------------------------------------------------------

/// An in-process mock NNTP server for testing.
pub struct MockNntpServer {
    pub addr: SocketAddr,
    _shutdown: tokio::sync::watch::Sender<bool>,
}

impl MockNntpServer {
    /// Start the mock server on a random port.
    pub async fn start(config: MockConfig) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let config = Arc::new(config);
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        if let Ok((stream, _)) = result {
                            let cfg = config.clone();
                            tokio::spawn(handle_connection(stream, cfg));
                        }
                    }
                    _ = shutdown_rx.changed() => break,
                }
            }
        });

        Self {
            addr,
            _shutdown: shutdown_tx,
        }
    }

    /// The port the mock server is listening on.
    pub fn port(&self) -> u16 {
        self.addr.port()
    }
}

// ---------------------------------------------------------------------------
// Test config helpers
// ---------------------------------------------------------------------------

/// Create a plain-TCP ServerConfig pointing at localhost on the given port.
pub fn test_config(port: u16) -> ServerConfig {
    ServerConfig {
        id: "test-server".into(),
        name: "Test Server".into(),
        host: "127.0.0.1".into(),
        port,
        ssl: false,
        ssl_verify: false,
        username: None,
        password: None,
        connections: 4,
        priority: 0,
        enabled: true,
        retention: 0,
        pipelining: 1,
        optional: false,
        compress: false,
    }
}

/// Create a plain-TCP ServerConfig with authentication credentials.
pub fn test_config_with_auth(port: u16, user: &str, pass: &str) -> ServerConfig {
    let mut config = test_config(port);
    config.username = Some(user.to_string());
    config.password = Some(pass.to_string());
    config
}

// ---------------------------------------------------------------------------
// Connection handler
// ---------------------------------------------------------------------------

async fn handle_connection(stream: tokio::net::TcpStream, config: Arc<MockConfig>) {
    let mut stream = BufReader::new(stream);

    // Send welcome banner
    if config.service_unavailable {
        let _ = stream
            .get_mut()
            .write_all(b"502 Service unavailable\r\n")
            .await;
        let _ = stream.get_mut().flush().await;
        return;
    }

    let welcome = format!("{} {}\r\n", config.welcome_code, config.welcome_message);
    let _ = stream.get_mut().write_all(welcome.as_bytes()).await;
    let _ = stream.get_mut().flush().await;

    let mut authenticated = !config.auth_required;
    let mut selected_group: Option<String> = None;
    let mut line = String::new();

    loop {
        line.clear();
        match stream.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parts: Vec<&str> = trimmed.splitn(3, ' ').collect();
        let cmd = parts[0].to_uppercase();

        match cmd.as_str() {
            "QUIT" => {
                let _ = stream.get_mut().write_all(b"205 Goodbye\r\n").await;
                let _ = stream.get_mut().flush().await;
                break;
            }

            "AUTHINFO" => {
                let sub = parts.get(1).map(|s| s.to_uppercase()).unwrap_or_default();
                match sub.as_str() {
                    "USER" => {
                        if config.fail_auth {
                            let _ = stream
                                .get_mut()
                                .write_all(b"482 Authentication rejected\r\n")
                                .await;
                        } else {
                            let _ = stream
                                .get_mut()
                                .write_all(b"381 Password required\r\n")
                                .await;
                        }
                    }
                    "PASS" => {
                        if config.fail_auth {
                            let _ = stream
                                .get_mut()
                                .write_all(b"481 Authentication failed\r\n")
                                .await;
                        } else if let Some((_, ref valid_pass)) = config.valid_credentials {
                            let given = parts.get(2).unwrap_or(&"");
                            if *given == valid_pass.as_str() {
                                authenticated = true;
                                let _ = stream
                                    .get_mut()
                                    .write_all(b"281 Authentication accepted\r\n")
                                    .await;
                            } else {
                                let _ = stream
                                    .get_mut()
                                    .write_all(b"481 Authentication failed\r\n")
                                    .await;
                            }
                        } else {
                            // No specific credentials — accept anything
                            authenticated = true;
                            let _ = stream
                                .get_mut()
                                .write_all(b"281 Authentication accepted\r\n")
                                .await;
                        }
                    }
                    _ => {
                        let _ = stream
                            .get_mut()
                            .write_all(b"500 Unknown AUTHINFO subcommand\r\n")
                            .await;
                    }
                }
            }

            "GROUP" => {
                if !authenticated {
                    let _ = stream
                        .get_mut()
                        .write_all(b"480 Authentication required\r\n")
                        .await;
                } else {
                    let name = parts.get(1).unwrap_or(&"");
                    if let Some(&(count, first, last)) = config.groups.get(*name) {
                        selected_group = Some(name.to_string());
                        let resp = format!("211 {} {} {} {}\r\n", count, first, last, name);
                        let _ = stream.get_mut().write_all(resp.as_bytes()).await;
                    } else {
                        let _ = stream.get_mut().write_all(b"411 No such group\r\n").await;
                    }
                }
            }

            "XOVER" => {
                if !authenticated {
                    let _ = stream
                        .get_mut()
                        .write_all(b"480 Authentication required\r\n")
                        .await;
                } else if selected_group.is_none() {
                    let _ = stream
                        .get_mut()
                        .write_all(b"412 No newsgroup selected\r\n")
                        .await;
                } else if config.xover_entries.is_empty() {
                    let _ = stream
                        .get_mut()
                        .write_all(b"420 No articles in range\r\n")
                        .await;
                } else {
                    let _ = stream
                        .get_mut()
                        .write_all(b"224 Overview data follows\r\n")
                        .await;
                    for entry in &config.xover_entries {
                        let _ = stream.get_mut().write_all(entry.as_bytes()).await;
                        let _ = stream.get_mut().write_all(b"\r\n").await;
                    }
                    let _ = stream.get_mut().write_all(b".\r\n").await;
                }
            }

            "ARTICLE" => {
                if !authenticated {
                    let _ = stream
                        .get_mut()
                        .write_all(b"480 Authentication required\r\n")
                        .await;
                } else {
                    let mid = parts
                        .get(1)
                        .unwrap_or(&"")
                        .trim_matches(|c| c == '<' || c == '>');
                    if let Some(data) = config.articles.get(mid) {
                        let header = format!("220 0 <{}>\r\n", mid);
                        let _ = stream.get_mut().write_all(header.as_bytes()).await;
                        write_multiline_body(stream.get_mut(), data).await;
                    } else {
                        let resp = format!("430 No article: <{}>\r\n", mid);
                        let _ = stream.get_mut().write_all(resp.as_bytes()).await;
                    }
                }
            }

            "BODY" => {
                if !authenticated {
                    let _ = stream
                        .get_mut()
                        .write_all(b"480 Authentication required\r\n")
                        .await;
                } else {
                    let mid = parts
                        .get(1)
                        .unwrap_or(&"")
                        .trim_matches(|c| c == '<' || c == '>');
                    if let Some(data) = config.articles.get(mid) {
                        let header = format!("222 0 <{}>\r\n", mid);
                        let _ = stream.get_mut().write_all(header.as_bytes()).await;
                        write_multiline_body(stream.get_mut(), data).await;
                    } else {
                        let resp = format!("430 No article: <{}>\r\n", mid);
                        let _ = stream.get_mut().write_all(resp.as_bytes()).await;
                    }
                }
            }

            "STAT" => {
                if !authenticated {
                    let _ = stream
                        .get_mut()
                        .write_all(b"480 Authentication required\r\n")
                        .await;
                } else {
                    let mid = parts
                        .get(1)
                        .unwrap_or(&"")
                        .trim_matches(|c| c == '<' || c == '>');
                    if config.articles.contains_key(mid) {
                        let resp = format!("223 0 <{}>\r\n", mid);
                        let _ = stream.get_mut().write_all(resp.as_bytes()).await;
                    } else {
                        let resp = format!("430 No article: <{}>\r\n", mid);
                        let _ = stream.get_mut().write_all(resp.as_bytes()).await;
                    }
                }
            }

            _ => {
                let resp = format!("500 Unknown command: {}\r\n", cmd);
                let _ = stream.get_mut().write_all(resp.as_bytes()).await;
            }
        }

        let _ = stream.get_mut().flush().await;
    }
}

/// Write a multiline body with dot-stuffing and `.\r\n` terminator.
async fn write_multiline_body(writer: &mut tokio::net::TcpStream, data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    for line in text.lines() {
        if line.starts_with('.') {
            let _ = writer.write_all(b".").await; // dot-stuff
        }
        let _ = writer.write_all(line.as_bytes()).await;
        let _ = writer.write_all(b"\r\n").await;
    }
    // If data was empty, lines() yields nothing — just write terminator
    if data.is_empty() {
        let _ = writer.write_all(b"\r\n").await;
    }
    let _ = writer.write_all(b".\r\n").await;
    let _ = writer.flush().await;
}
