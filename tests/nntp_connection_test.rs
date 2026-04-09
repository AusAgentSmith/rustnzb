//! Integration test: connect to real NNTP servers with all port/SSL variants.
//!
//! Credentials are read from environment variables:
//!   - NNTP_PRIMARY_HOST, NNTP_PRIMARY_USER, NNTP_PRIMARY_PASS
//!   - NNTP_BACKUP_HOST, NNTP_BACKUP_USER, NNTP_BACKUP_PASS
//!
//! Tests are skipped on CI or when credentials are not set.

use nzb_web::nzb_core::config::ServerConfig;
use nzb_web::nzb_core::nzb_nntp::NntpConnection;

/// Helper to build a ServerConfig for testing.
fn make_config(
    id: &str,
    host: &str,
    port: u16,
    ssl: bool,
    username: &str,
    password: &str,
    connections: u16,
) -> ServerConfig {
    let mut c = ServerConfig::new(id, host);
    c.name = format!("{host}:{port} ({})", if ssl { "SSL" } else { "plain" });
    c.port = port;
    c.ssl = ssl;
    c.username = Some(username.to_string());
    c.password = Some(password.to_string());
    c.connections = connections;
    c.ramp_up_delay_ms = 0;
    c.recv_buffer_size = 0;
    c
}

/// Test a single connection: connect, authenticate, then quit.
async fn test_connection(config: &ServerConfig) -> Result<String, String> {
    let mut conn = NntpConnection::new(config.id.clone());

    match tokio::time::timeout(std::time::Duration::from_secs(15), conn.connect(config)).await {
        Ok(Ok(())) => {
            let msg = format!(
                "OK  {}:{} {} — connected and authenticated",
                config.host,
                config.port,
                if config.ssl { "SSL" } else { "PLAIN" }
            );
            // Graceful disconnect
            let _ = conn.quit().await;
            Ok(msg)
        }
        Ok(Err(e)) => Err(format!(
            "FAIL {}:{} {} — {}",
            config.host,
            config.port,
            if config.ssl { "SSL" } else { "PLAIN" },
            e
        )),
        Err(_) => Err(format!(
            "TIMEOUT {}:{} {} — no response in 15s",
            config.host,
            config.port,
            if config.ssl { "SSL" } else { "PLAIN" },
        )),
    }
}

fn get_primary_creds() -> Option<(String, String, String)> {
    let host = std::env::var("NNTP_PRIMARY_HOST").ok()?;
    let user = std::env::var("NNTP_PRIMARY_USER").ok()?;
    let pass = std::env::var("NNTP_PRIMARY_PASS").ok()?;
    Some((host, user, pass))
}

fn get_backup_creds() -> Option<(String, String, String)> {
    let host = std::env::var("NNTP_BACKUP_HOST").ok()?;
    let user = std::env::var("NNTP_BACKUP_USER").ok()?;
    let pass = std::env::var("NNTP_BACKUP_PASS").ok()?;
    Some((host, user, pass))
}

fn should_skip() -> bool {
    std::env::var("CI").is_ok()
}

#[tokio::test]
async fn test_primary_ssl_563() {
    if should_skip() {
        return;
    }
    let Some((host, user, pass)) = get_primary_creds() else {
        eprintln!("Skipping: NNTP_PRIMARY_* env vars not set");
        return;
    };
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = make_config("primary-ssl-563", &host, 563, true, &user, &pass, 1);
    let result = test_connection(&config).await;
    eprintln!("{}", result.as_ref().unwrap_or_else(|e| e));
    assert!(result.is_ok(), "{}", result.unwrap_err());
}

#[tokio::test]
async fn test_primary_ssl_443() {
    if should_skip() {
        return;
    }
    let Some((host, user, pass)) = get_primary_creds() else {
        eprintln!("Skipping: NNTP_PRIMARY_* env vars not set");
        return;
    };
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = make_config("primary-ssl-443", &host, 443, true, &user, &pass, 1);
    let result = test_connection(&config).await;
    eprintln!("{}", result.as_ref().unwrap_or_else(|e| e));
    assert!(result.is_ok(), "{}", result.unwrap_err());
}

#[tokio::test]
async fn test_primary_plain_119() {
    if should_skip() {
        return;
    }
    let Some((host, user, pass)) = get_primary_creds() else {
        eprintln!("Skipping: NNTP_PRIMARY_* env vars not set");
        return;
    };
    let config = make_config("primary-plain-119", &host, 119, false, &user, &pass, 1);
    let result = test_connection(&config).await;
    eprintln!("{}", result.as_ref().unwrap_or_else(|e| e));
    assert!(result.is_ok(), "{}", result.unwrap_err());
}

#[tokio::test]
async fn test_primary_plain_80() {
    if should_skip() {
        return;
    }
    let Some((host, user, pass)) = get_primary_creds() else {
        eprintln!("Skipping: NNTP_PRIMARY_* env vars not set");
        return;
    };
    let config = make_config("primary-plain-80", &host, 80, false, &user, &pass, 1);
    let result = test_connection(&config).await;
    eprintln!("{}", result.as_ref().unwrap_or_else(|e| e));
    assert!(result.is_ok(), "{}", result.unwrap_err());
}

#[tokio::test]
async fn test_backup_ssl_563() {
    if should_skip() {
        return;
    }
    let Some((host, user, pass)) = get_backup_creds() else {
        eprintln!("Skipping: NNTP_BACKUP_* env vars not set");
        return;
    };
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = make_config("backup-ssl-563", &host, 563, true, &user, &pass, 1);
    let result = test_connection(&config).await;
    eprintln!("{}", result.as_ref().unwrap_or_else(|e| e));
    assert!(result.is_ok(), "{}", result.unwrap_err());
}

/// Run all variants and print a summary table.
#[tokio::test]
async fn test_all_connections_summary() {
    if should_skip() {
        return;
    }
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut configs = Vec::new();

    if let Some((host, user, pass)) = get_primary_creds() {
        for (id, port, ssl) in [
            ("primary-ssl-563", 563u16, true),
            ("primary-ssl-443", 443, true),
            ("primary-plain-119", 119, false),
            ("primary-plain-80", 80, false),
        ] {
            configs.push(make_config(id, &host, port, ssl, &user, &pass, 1));
        }
    }

    if let Some((host, user, pass)) = get_backup_creds() {
        configs.push(make_config(
            "backup-ssl-563",
            &host,
            563,
            true,
            &user,
            &pass,
            1,
        ));
    }

    if configs.is_empty() {
        eprintln!("Skipping: no NNTP credentials configured");
        return;
    }

    eprintln!("\n============================================================");
    eprintln!("  NNTP Connection Test Summary");
    eprintln!("============================================================");

    let mut pass = 0;
    let mut fail = 0;

    for config in &configs {
        let result = test_connection(config).await;
        match &result {
            Ok(msg) => {
                eprintln!("  {msg}");
                pass += 1;
            }
            Err(msg) => {
                eprintln!("  {msg}");
                fail += 1;
            }
        }
    }

    eprintln!("============================================================");
    eprintln!("  Results: {pass} passed, {fail} failed");
    eprintln!("============================================================\n");
}
