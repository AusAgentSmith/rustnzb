//! Integration tests for server-priority dispatch.
//!
//! These tests spin up multiple `MockNntpServer` instances at different
//! priorities, submit a real NZB job, and assert that articles flow through
//! the servers the way the priority model prescribes:
//!
//! - Fresh articles ONLY go to the highest-priority healthy server.
//! - When a primary returns 430 (ArticleNotFound), the article cascades to
//!   the next-priority server (matches SABnzbd `search_new_server`).
//! - When a primary is circuit-broken / unreachable, backups take over
//!   immediately (no 60 s idle stall).
//! - Backup workers sitting idle while the primary serves everything are NOT
//!   evicted by the idle watchdog (Bug 2 fix).
//! - Same-priority peers share work.
//!
//! The MockNntpServer has no per-server request counter, so "this server
//! served article X" is inferred from `MockConfig.articles` — only the server
//! that actually holds article X's yEnc body can succeed on it. Combine that
//! with `article_response_overrides: {msg_id → 430}` to force selective
//! failover without changing the job's article count.

mod harness;

use std::collections::HashMap;
use std::time::Duration;

use harness::nzb_fixture::NzbFixture;
use harness::{HarnessBuilder, ServerProfile, yenc_articles};
use nzb_nntp::testutil::MockConfig;
use nzb_web::nzb_core::models::{JobStatus, Priority};

// ---------------------------------------------------------------------------
// Fixture helpers — return fully owned data so tests don't fight the borrow
// checker over fixture-internal references.
// ---------------------------------------------------------------------------

/// Builds an N-segment single-file NZB. Returns the owned XML bytes, a map
/// of yEnc-encoded article bodies keyed by message-id (ready to splice into
/// any subset for a per-server MockConfig), and the ordered list of message
/// ids so tests can reference individual articles (e.g. to 430-override).
fn make_fixture(prefix: &str, n: usize) -> (Vec<u8>, HashMap<String, Vec<u8>>, Vec<String>) {
    let mids: Vec<String> = (1..=n).map(|i| format!("{prefix}-{i}@test")).collect();
    let bodies: Vec<Vec<u8>> = (1..=n).map(|i| format!("body-{i}").into_bytes()).collect();
    let segs: Vec<(&str, &[u8])> = mids
        .iter()
        .zip(bodies.iter())
        .map(|(m, b)| (m.as_str(), b.as_slice()))
        .collect();
    let file_name = format!("{prefix}.bin");
    let built = NzbFixture::new(prefix).add_file(&file_name, &segs).build();
    let triples: Vec<(&str, &[u8], &str)> = built
        .articles
        .iter()
        .map(|(m, b, f)| (*m, *b, f.as_str()))
        .collect();
    let yenc = yenc_articles(&triples);
    (built.xml, yenc, mids)
}

/// Select a subset of `all_yenc` into a fresh map — lets one server serve
/// some message-ids while another serves the rest.
fn subset(all_yenc: &HashMap<String, Vec<u8>>, which: &[&str]) -> HashMap<String, Vec<u8>> {
    which
        .iter()
        .filter_map(|m| all_yenc.get(*m).map(|v| (m.to_string(), v.clone())))
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The primary serves every article. The backup's article map is EMPTY —
/// meaning if any article were dispatched to the backup it would 430 and
/// ultimately fail the job. We assert the job succeeds 100%, which proves
/// the priority gate actually kept the backup out of the dispatch path.
///
/// Secondarily, we confirm the Bug 2 idle-eviction fix: with `max_worker_idle`
/// set short (3 s) and the test running ~6 s, backup workers would previously
/// be evicted at least once. Assert `worker_eviction_count()` stays at 0.
#[tokio::test]
async fn backup_stays_idle_when_primary_serves_all() {
    let (xml, yenc, _mids) = make_fixture("primserve", 6);

    let primary = ServerProfile::start(
        "primary",
        MockConfig {
            articles: yenc.clone(),
            ..Default::default()
        },
        3,
    )
    .await
    .with_priority(0);

    // Backup has NOTHING. Any article sent here would 430 → job fails.
    let backup = ServerProfile::start(
        "backup",
        MockConfig {
            articles: HashMap::new(),
            ..Default::default()
        },
        3,
    )
    .await
    .with_priority(1);

    let engine = HarnessBuilder::new()
        .with_server(primary)
        .with_server(backup)
        .article_timeout(10)
        .build();

    // Aggressive idle threshold: without the Bug 2 fix, backup workers
    // would be evicted at 3 s, respawn, evict again, etc.
    engine
        .queue_manager
        .set_max_worker_idle(Duration::from_secs(3));

    let job_id = engine.submit_nzb_xml("primserve", xml).expect("submit nzb");

    // All 6 articles must download cleanly (only primary has them).
    let done = engine
        .wait_for(Duration::from_secs(20), |snap| {
            snap.job(&job_id)
                .map(|j| j.articles_downloaded == 6)
                .unwrap_or(false)
        })
        .await;

    let view = engine.job(&job_id).expect("job present");
    assert!(
        done,
        "primary did not complete the job: status={} downloaded={} failed={}",
        view.status, view.articles_downloaded, view.articles_failed
    );
    assert_eq!(
        view.articles_failed, 0,
        "no articles should have failed — backup should have stayed out of dispatch"
    );

    // Bug 2: the backup's 3 workers sit idle the whole time (no workable
    // items since primary handled everything). They must NOT be evicted.
    let evictions = engine.queue_manager.worker_eviction_count();
    assert_eq!(
        evictions, 0,
        "idle backup workers were evicted ({evictions}x) — Bug 2 regressed"
    );
}

/// Primary is launched with `service_unavailable: true` so it sends 502 on
/// connect. The backup is healthy and has every article. Before the fix,
/// backup workers would never get work until the primary was tried per-article
/// and circuit-broken. After the fix, the circuit-broken primary is excluded
/// from `higher_priority_servers` and the backup takes over immediately.
#[tokio::test]
async fn backup_takes_over_when_primary_unreachable() {
    let (xml, yenc, _mids) = make_fixture("deadprim", 4);

    let primary = ServerProfile::start(
        "dead-primary",
        MockConfig {
            service_unavailable: true,
            ..Default::default()
        },
        2,
    )
    .await
    .with_priority(0);

    let backup = ServerProfile::start(
        "backup",
        MockConfig {
            articles: yenc,
            ..Default::default()
        },
        2,
    )
    .await
    .with_priority(1);

    let engine = HarnessBuilder::new()
        .with_server(primary)
        .with_server(backup)
        .article_timeout(5)
        .build();

    let job_id = engine.submit_nzb_xml("deadprim", xml).expect("submit nzb");

    // Backup should pick up all 4 articles — primary is 502 on every connect.
    let done = engine
        .wait_for(Duration::from_secs(25), |snap| {
            snap.job(&job_id)
                .map(|j| j.articles_downloaded == 4)
                .unwrap_or(false)
        })
        .await;

    let view = engine.job(&job_id).expect("job present");
    assert!(
        done,
        "backup did not take over from unreachable primary: status={} downloaded={} failed={}",
        view.status, view.articles_downloaded, view.articles_failed
    );
}

/// Primary returns 430 for half the articles; has the other half. Backup has
/// every article. Assert the job completes 100% — primary serves what it can,
/// backup picks up the failures.
#[tokio::test]
async fn backup_picks_up_primary_430_failures() {
    let (xml, yenc, mids) = make_fixture("mixed", 4);

    // Primary fails on the first two, serves the last two.
    let mut overrides = HashMap::new();
    overrides.insert(mids[0].clone(), 430u16);
    overrides.insert(mids[1].clone(), 430u16);
    let primary_articles = subset(&yenc, &[mids[2].as_str(), mids[3].as_str()]);

    let primary = ServerProfile::start(
        "primary",
        MockConfig {
            articles: primary_articles,
            article_response_overrides: overrides,
            ..Default::default()
        },
        2,
    )
    .await
    .with_priority(0);

    let backup = ServerProfile::start(
        "backup",
        MockConfig {
            articles: yenc,
            ..Default::default()
        },
        2,
    )
    .await
    .with_priority(1);

    let engine = HarnessBuilder::new()
        .with_server(primary)
        .with_server(backup)
        .article_timeout(10)
        .build();

    let job_id = engine.submit_nzb_xml("mixed", xml).expect("submit nzb");

    let done = engine
        .wait_for(Duration::from_secs(20), |snap| {
            snap.job(&job_id)
                .map(|j| j.articles_downloaded == 4)
                .unwrap_or(false)
        })
        .await;

    let view = engine.job(&job_id).expect("job present");
    assert!(
        done,
        "job did not complete via cascade: status={} downloaded={} failed={}",
        view.status, view.articles_downloaded, view.articles_failed
    );
    assert_eq!(
        view.articles_failed, 0,
        "expected 0 failures after failover"
    );
}

/// Three tiers (priority 0, 1, 2). Priority 0 and 1 both 430 on all articles.
/// Priority 2 has everything. Verifies multi-tier cascade — priority 2 only
/// gets items after both 0 and 1 have tried.
#[tokio::test]
async fn three_tier_priority_cascade() {
    let (xml, yenc, mids) = make_fixture("tier", 3);

    let all_430: HashMap<String, u16> = mids.iter().map(|m| (m.clone(), 430u16)).collect();

    let tier0 = ServerProfile::start(
        "tier0",
        MockConfig {
            article_response_overrides: all_430.clone(),
            ..Default::default()
        },
        2,
    )
    .await
    .with_priority(0);

    let tier1 = ServerProfile::start(
        "tier1",
        MockConfig {
            article_response_overrides: all_430,
            ..Default::default()
        },
        2,
    )
    .await
    .with_priority(1);

    let tier2 = ServerProfile::start(
        "tier2",
        MockConfig {
            articles: yenc,
            ..Default::default()
        },
        2,
    )
    .await
    .with_priority(2);

    let engine = HarnessBuilder::new()
        .with_server(tier0)
        .with_server(tier1)
        .with_server(tier2)
        .article_timeout(10)
        .build();

    let job_id = engine.submit_nzb_xml("tier", xml).expect("submit nzb");

    let done = engine
        .wait_for(Duration::from_secs(25), |snap| {
            snap.job(&job_id)
                .map(|j| j.articles_downloaded == 3)
                .unwrap_or(false)
        })
        .await;

    let view = engine.job(&job_id).expect("job present");
    assert!(
        done,
        "tier-2 did not complete cascade: status={} downloaded={} failed={}",
        view.status, view.articles_downloaded, view.articles_failed
    );
}

/// Two servers both at priority 0. Neither gets a priority-gate block; both
/// can compete for every fresh article. We just assert the job completes —
/// this is the regression-guard case (priority 0 = priority 0 means no
/// starvation, same as the pre-patch baseline).
#[tokio::test]
async fn same_priority_peers_both_serve_job() {
    let (xml, yenc, _mids) = make_fixture("same", 6);

    let a = ServerProfile::start(
        "peer-a",
        MockConfig {
            articles: yenc.clone(),
            ..Default::default()
        },
        2,
    )
    .await
    .with_priority(0);

    let b = ServerProfile::start(
        "peer-b",
        MockConfig {
            articles: yenc,
            ..Default::default()
        },
        2,
    )
    .await
    .with_priority(0);

    let engine = HarnessBuilder::new()
        .with_server(a)
        .with_server(b)
        .article_timeout(10)
        .build();

    let job_id = engine.submit_nzb_xml("same", xml).expect("submit nzb");

    let done = engine
        .wait_for(Duration::from_secs(15), |snap| {
            snap.job(&job_id)
                .map(|j| j.articles_downloaded == 6)
                .unwrap_or(false)
        })
        .await;

    let view = engine.job(&job_id).expect("job present");
    assert!(
        done,
        "same-priority peers didn't complete job: status={} downloaded={} failed={}",
        view.status, view.articles_downloaded, view.articles_failed
    );
}

/// Reordering within the same numeric priority must still have immediate
/// scheduling effect. Moving a queued job above the active one should pause
/// the active download and start the reordered job right away.
#[tokio::test]
async fn moving_queued_job_to_top_preempts_active_download() {
    let (xml_a, yenc_a, _mids_a) = make_fixture("drag-a", 12);
    let (xml_b, yenc_b, _mids_b) = make_fixture("drag-b", 12);

    let primary = ServerProfile::start(
        "primary",
        MockConfig {
            articles: yenc_a.into_iter().chain(yenc_b).collect(),
            response_delay: Some(Duration::from_millis(120)),
            ..Default::default()
        },
        2,
    )
    .await
    .with_priority(0);

    let engine = HarnessBuilder::new()
        .with_server(primary)
        .max_active_downloads(1)
        .article_timeout(10)
        .build();

    let first_id = engine
        .submit_nzb_xml("drag-a", xml_a)
        .expect("submit first nzb");
    let first_started = engine
        .wait_for(Duration::from_secs(5), |snap| {
            snap.job(&first_id)
                .map(|j| j.status == JobStatus::Downloading)
                .unwrap_or(false)
        })
        .await;
    assert!(first_started, "first job never entered downloading state");

    let second_id = engine
        .submit_nzb_xml("drag-b", xml_b)
        .expect("submit second nzb");
    let second_queued = engine
        .wait_for(Duration::from_secs(5), |snap| {
            let first = snap.job(&first_id);
            let second = snap.job(&second_id);
            matches!(
                (first.map(|j| j.status), second.map(|j| j.status)),
                (Some(JobStatus::Downloading), Some(JobStatus::Queued))
            )
        })
        .await;
    assert!(
        second_queued,
        "expected first job downloading and second queued before reorder"
    );

    engine
        .queue_manager
        .move_job(&second_id, 0)
        .expect("move second job to top");

    let preempted = engine
        .wait_for(Duration::from_secs(5), |snap| {
            let first = snap.job(&first_id);
            let second = snap.job(&second_id);
            matches!(
                (first.map(|j| j.status), second.map(|j| j.status)),
                (Some(JobStatus::Paused), Some(JobStatus::Downloading))
            )
        })
        .await;

    let first = engine.job(&first_id).expect("first job present");
    let second = engine.job(&second_id).expect("second job present");
    assert!(
        preempted,
        "reorder did not preempt immediately: first={} second={}",
        first.status, second.status
    );
}

/// A priority change must reorder only — it must never pause an
/// actively-downloading job (GH #124). With a single download slot, raising a
/// queued job to `Force` leaves the running job downloading; the new order
/// takes effect when the slot frees. (Contrast with `move_job`, an explicit
/// drag-to-top, which does preempt — see the test above.)
#[tokio::test]
async fn raising_queued_priority_does_not_preempt_active_download() {
    let (xml_a, yenc_a, _mids_a) = make_fixture("prio-a", 12);
    let (xml_b, yenc_b, _mids_b) = make_fixture("prio-b", 12);

    let primary = ServerProfile::start(
        "primary",
        MockConfig {
            articles: yenc_a.into_iter().chain(yenc_b).collect(),
            response_delay: Some(Duration::from_millis(120)),
            ..Default::default()
        },
        2,
    )
    .await
    .with_priority(0);

    let engine = HarnessBuilder::new()
        .with_server(primary)
        .max_active_downloads(1)
        .article_timeout(10)
        .build();

    let first_id = engine
        .submit_nzb_xml("prio-a", xml_a)
        .expect("submit first nzb");
    let first_started = engine
        .wait_for(Duration::from_secs(5), |snap| {
            snap.job(&first_id)
                .map(|j| j.status == JobStatus::Downloading)
                .unwrap_or(false)
        })
        .await;
    assert!(first_started, "first job never entered downloading state");

    let second_id = engine
        .submit_nzb_xml("prio-b", xml_b)
        .expect("submit second nzb");
    let second_queued = engine
        .wait_for(Duration::from_secs(5), |snap| {
            matches!(
                (
                    snap.job(&first_id).map(|j| j.status),
                    snap.job(&second_id).map(|j| j.status)
                ),
                (Some(JobStatus::Downloading), Some(JobStatus::Queued))
            )
        })
        .await;
    assert!(
        second_queued,
        "expected first job downloading and second queued before reprioritise"
    );

    // Raise the queued job to the highest priority via a priority change.
    engine
        .queue_manager
        .set_job_priority(&second_id, Priority::Force)
        .expect("raise second job to Force");

    // The active job must NOT be preempted. Give any (incorrect) preemption a
    // window to occur and assert it never does.
    let preempted = engine
        .wait_for(Duration::from_secs(2), |snap| {
            snap.job(&first_id)
                .map(|j| j.status == JobStatus::Paused)
                .unwrap_or(false)
        })
        .await;
    assert!(
        !preempted,
        "priority change must not pause the active download"
    );

    let first = engine.job(&first_id).expect("first job present");
    let second = engine.job(&second_id).expect("second job present");
    assert_eq!(
        first.status,
        JobStatus::Downloading,
        "active job should keep downloading after a priority change"
    );
    assert_eq!(
        second.status,
        JobStatus::Queued,
        "reprioritised job waits for a free slot instead of preempting"
    );
}

/// Sanity: job must reach a terminal state (Completed/Failed) after
/// submission when the only priority-0 server is unreachable. Guards against
/// the failure mode where backup workers get starvation-logged and never
/// retire. With `service_unavailable: true`, primary 502s on every connect
/// → primary drops out of higher_priority_servers → backup picks up all work
/// → job must complete.
#[tokio::test]
async fn unreachable_primary_does_not_hang_backup() {
    let (xml, yenc, _mids) = make_fixture("unreach", 2);

    // Primary: immediate 502 on every connect.
    let primary = ServerProfile::start(
        "dead",
        MockConfig {
            service_unavailable: true,
            ..Default::default()
        },
        1,
    )
    .await
    .with_priority(0);

    // Backup: has everything.
    let backup = ServerProfile::start(
        "live",
        MockConfig {
            articles: yenc,
            ..Default::default()
        },
        2,
    )
    .await
    .with_priority(1);

    let engine = HarnessBuilder::new()
        .with_server(primary)
        .with_server(backup)
        .article_timeout(5)
        .build();

    let job_id = engine.submit_nzb_xml("unreach", xml).expect("submit nzb");

    // Must reach a terminal state — never hang in Downloading forever.
    let settled = engine
        .wait_for_status(
            &job_id,
            Duration::from_secs(25),
            &[JobStatus::Completed, JobStatus::Failed],
        )
        .await;

    let view = engine.job(&job_id).expect("job present");
    assert!(
        settled,
        "job never left Downloading when primary was unreachable: status={} downloaded={} failed={}",
        view.status, view.articles_downloaded, view.articles_failed
    );
}
