//! Catalog and profile invariants for deterministic integration tests.

mod harness;

use harness::nzb_fixture::FixtureCatalog;
use harness::{HarnessBuilder, ServerProfile};
use nzb_nntp::testutil::MockConfig;

type ProfileFactory = fn(ServerProfile) -> HarnessBuilder;

#[tokio::test]
async fn catalog_cases_are_reproducible_and_profiles_are_explicit() {
    let first = FixtureCatalog::single();
    let second = FixtureCatalog::single();
    assert_eq!(first.name, second.name);
    assert_eq!(first.xml, second.xml);
    assert_eq!(first.articles, second.articles);
    assert_eq!(first.articles.len(), 1);

    let multi = FixtureCatalog::multi_segment();
    assert_eq!(multi.articles.len(), 3);
    assert!(String::from_utf8_lossy(&multi.xml).contains("catalog.bin"));

    let profiles: [(&str, ProfileFactory); 6] = [
        ("happy", HarnessBuilder::happy_path),
        ("retry", HarnessBuilder::retrying),
        ("pause", HarnessBuilder::pause_resume),
        ("cancel", HarnessBuilder::cancellation),
        ("hopeless", HarnessBuilder::hopeless),
        ("restart", HarnessBuilder::restart_recovery),
    ];
    for (name, profile) in profiles {
        let server = ServerProfile::start(name, MockConfig::default(), 1).await;
        let _ = profile(server);
    }
}
