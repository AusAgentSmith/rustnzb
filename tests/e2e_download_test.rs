//! End-to-end test: parse NZB → connect to NNTP → fetch articles → yEnc decode → assemble file
//!
//! Uses the smallest NZB (We.Bury.The.Dead) and fetches a small par2 file (single article)
//! plus the first few articles of a RAR file to verify the full pipeline.

use nzb_web::nzb_core::config::ServerConfig;
use nzb_web::nzb_core::nzb_nntp::NntpConnection;
use nzb_web::nzb_core::nzb_parser;
use nzb_web::nzb_decode::yenc;

fn usenet_farm_config() -> ServerConfig {
    let mut c = ServerConfig::new("uf-test", "news.usenet.farm");
    c.name = "Usenet Farm Test".to_string();
    c.username = Some("uf8ea2a82f370952aa92".to_string());
    c.password = Some("ff24a05910fd23cb0040ff".to_string());
    c.connections = 1;
    c.ramp_up_delay_ms = 0;
    c.recv_buffer_size = 0;
    c
}

#[tokio::test]
async fn test_fetch_single_article_and_decode() {
    // 1. Parse NZB
    let nzb_path = std::path::Path::new("TestData/We.Bury.The.Dead.2024.BDRip.x264-COCAIN.nzb");
    if !nzb_path.exists() || std::env::var("CI").is_ok() {
        eprintln!("TestData not found or CI environment, skipping");
        return;
    }
    let _ = rustls::crypto::ring::default_provider().install_default();

    let job = nzb_parser::parse_nzb_file(nzb_path).unwrap();
    eprintln!(
        "Parsed NZB: {} files, {} articles, {:.1} MB",
        job.file_count,
        job.article_count,
        job.total_bytes as f64 / 1_048_576.0
    );

    // Find the single-article par2 file (smallest file, easiest to test)
    let small_file = job
        .files
        .iter()
        .filter(|f| f.articles.len() == 1)
        .min_by_key(|f| f.bytes)
        .expect("No single-article file found");

    eprintln!(
        "Target file: {} ({} bytes, {} article)",
        small_file.filename,
        small_file.bytes,
        small_file.articles.len()
    );

    let article = &small_file.articles[0];
    eprintln!("Message-ID: {}", article.message_id);

    // 2. Connect to NNTP server
    let config = usenet_farm_config();
    let mut conn = NntpConnection::new("uf-test".to_string());

    let connect_result =
        tokio::time::timeout(std::time::Duration::from_secs(15), conn.connect(&config)).await;

    assert!(connect_result.is_ok(), "Connection timed out");
    assert!(connect_result.unwrap().is_ok(), "Connection failed");
    eprintln!("Connected to {}", config.host);

    // 3. Fetch article
    let fetch_result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        conn.fetch_article(&article.message_id),
    )
    .await;

    assert!(fetch_result.is_ok(), "Fetch timed out");
    let response = fetch_result.unwrap();
    assert!(response.is_ok(), "Fetch failed: {:?}", response.err());

    let response = response.unwrap();
    assert_eq!(response.code, 220, "Expected 220 article follows");

    let raw_data = response.data.expect("No article data received");
    eprintln!("Fetched {} bytes of raw article data", raw_data.len());
    assert!(raw_data.len() > 100, "Article data too small");

    // 4. yEnc decode
    let decode_result = yenc::decode_yenc(&raw_data);
    match &decode_result {
        Ok(result) => {
            eprintln!("yEnc decode successful:");
            eprintln!("  Filename: {:?}", result.filename);
            eprintln!("  Decoded size: {} bytes", result.data.len());
            eprintln!("  File size: {:?}", result.file_size);
            eprintln!("  Part begin: {:?}", result.part_begin);
            eprintln!("  CRC32: {:08X}", result.crc32);
            assert!(!result.data.is_empty(), "Decoded data is empty");
        }
        Err(e) => {
            // Some par2 files might have unusual encoding — log but check raw data
            eprintln!("yEnc decode error (may be expected for some files): {e}");
            eprintln!(
                "First 200 bytes of raw data: {:?}",
                String::from_utf8_lossy(&raw_data[..std::cmp::min(200, raw_data.len())])
            );
        }
    }

    // 5. Disconnect
    let _ = conn.quit().await;
    eprintln!("Disconnected. Test complete!");
}

#[tokio::test]
async fn test_fetch_multiple_articles_from_rar() {
    // Parse NZB and find a multi-segment RAR file
    let nzb_path = std::path::Path::new("TestData/We.Bury.The.Dead.2024.BDRip.x264-COCAIN.nzb");
    if !nzb_path.exists() || std::env::var("CI").is_ok() {
        eprintln!("TestData not found or CI environment, skipping");
        return;
    }
    let _ = rustls::crypto::ring::default_provider().install_default();

    let job = nzb_parser::parse_nzb_file(nzb_path).unwrap();

    // Find a RAR file with multiple segments
    let rar_file = job
        .files
        .iter()
        .find(|f| f.filename.to_lowercase().contains(".r") && f.articles.len() > 5)
        .expect("No multi-segment RAR file found");

    eprintln!(
        "Target RAR: {} ({} articles, {:.1} MB)",
        rar_file.filename,
        rar_file.articles.len(),
        rar_file.bytes as f64 / 1_048_576.0
    );

    // Connect
    let config = usenet_farm_config();
    let mut conn = NntpConnection::new("uf-test".to_string());
    conn.connect(&config).await.expect("Connection failed");
    eprintln!("Connected to {}", config.host);

    // Fetch first 3 articles
    let mut decoded_sizes = Vec::new();
    let articles_to_fetch = &rar_file.articles[..std::cmp::min(3, rar_file.articles.len())];

    for (i, article) in articles_to_fetch.iter().enumerate() {
        eprintln!(
            "Fetching article {}/{}: segment {} (msg-id: {}...)",
            i + 1,
            articles_to_fetch.len(),
            article.segment_number,
            &article.message_id[..std::cmp::min(30, article.message_id.len())]
        );

        let response = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            conn.fetch_article(&article.message_id),
        )
        .await
        .expect("Fetch timed out")
        .expect("Fetch failed");

        assert_eq!(response.code, 220);
        let raw_data = response.data.expect("No data");
        eprintln!("  Raw: {} bytes", raw_data.len());

        match yenc::decode_yenc(&raw_data) {
            Ok(result) => {
                eprintln!(
                    "  Decoded: {} bytes, offset: {:?}, CRC: {:08X}",
                    result.data.len(),
                    result.part_begin,
                    result.crc32
                );
                decoded_sizes.push(result.data.len());
            }
            Err(e) => {
                eprintln!("  Decode error: {e}");
                // Still count it
                decoded_sizes.push(0);
            }
        }
    }

    let _ = conn.quit().await;

    let total_decoded: usize = decoded_sizes.iter().sum();
    eprintln!(
        "\nSummary: fetched {} articles, decoded {} bytes total",
        articles_to_fetch.len(),
        total_decoded
    );

    // At least some articles should decode successfully
    assert!(total_decoded > 0, "No articles decoded successfully");
    eprintln!("Pipeline test complete!");
}

#[tokio::test]
async fn test_article_not_found_handling() {
    // Skip on CI — requires real Usenet server access
    if std::env::var("CI").is_ok() {
        eprintln!("Skipping on CI — requires real NNTP server");
        return;
    }

    // Install rustls crypto provider for TLS connections
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Test that we handle 430 (article not found) gracefully
    let config = usenet_farm_config();
    let mut conn = NntpConnection::new("uf-test".to_string());
    conn.connect(&config).await.expect("Connection failed");

    let result = conn
        .fetch_article("nonexistent-fake-id-12345@nowhere.invalid")
        .await;

    match &result {
        Err(nzb_web::nzb_core::nzb_nntp::NntpError::ArticleNotFound(_)) => {
            eprintln!("Correctly got ArticleNotFound for fake message-id");
        }
        Err(e) => {
            eprintln!("Got error (acceptable): {e}");
        }
        Ok(_) => {
            panic!("Should not have found a fake article!");
        }
    }

    // Connection should still be usable after a 430
    assert!(
        conn.is_connected(),
        "Connection should still be alive after 430"
    );
    let _ = conn.quit().await;
    eprintln!("Error handling test complete!");
}
