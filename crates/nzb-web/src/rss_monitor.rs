use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use tracing::{info, warn};

use nzb_core::config::RssFeedConfig;
use nzb_core::models::{Priority, RssItem};

use crate::queue_manager::QueueManager;

/// Background RSS feed monitor that polls configured feeds for NZB links,
/// persists all discovered items to the database, and automatically enqueues
/// items that match download rules.
pub struct RssMonitor {
    feeds: Vec<RssFeedConfig>,
    queue_manager: Arc<QueueManager>,
    data_dir: PathBuf,
    /// Max RSS items to keep (None = unlimited).
    rss_history_limit: Option<usize>,
}

impl RssMonitor {
    pub fn new(
        feeds: Vec<RssFeedConfig>,
        queue_manager: Arc<QueueManager>,
        data_dir: PathBuf,
        rss_history_limit: Option<usize>,
    ) -> Self {
        Self {
            feeds,
            queue_manager,
            data_dir,
            rss_history_limit,
        }
    }

    /// Migrate legacy rss_seen.json entries into the database on first run.
    fn migrate_seen_json(&self) {
        let seen_file = self.data_dir.join("rss_seen.json");
        if !seen_file.exists() {
            return;
        }

        let seen: HashSet<String> = std::fs::read_to_string(&seen_file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        if seen.is_empty() {
            let _ = std::fs::remove_file(&seen_file);
            return;
        }

        info!(
            count = seen.len(),
            "Migrating legacy rss_seen.json to database"
        );

        for id in &seen {
            let item = RssItem {
                id: id.clone(),
                feed_name: "migrated".into(),
                title: id.clone(),
                url: None,
                published_at: None,
                first_seen_at: Utc::now(),
                downloaded: true,
                downloaded_at: Some(Utc::now()),
                category: None,
                size_bytes: 0,
            };
            let _ = self.queue_manager.rss_item_upsert(&item);
        }

        // Remove the legacy file after migration
        let _ = std::fs::remove_file(&seen_file);
        info!("Legacy rss_seen.json migrated and removed");
    }

    /// Run the monitor loop forever, polling feeds at their configured intervals.
    pub async fn run(self) {
        info!("Starting RSS monitor with {} feed(s)", self.feeds.len());

        // Migrate legacy seen file on first run
        self.migrate_seen_json();

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        loop {
            for feed in &self.feeds {
                if !feed.enabled {
                    continue;
                }

                if let Err(e) = self.check_feed(&client, feed).await {
                    warn!(feed = %feed.name, error = %e, "RSS feed check failed");
                }
            }

            // Prune old items based on config
            self.prune_items();

            // Use the minimum poll interval across all enabled feeds, defaulting to 15 min
            let interval = self
                .feeds
                .iter()
                .filter(|f| f.enabled)
                .map(|f| f.poll_interval_secs)
                .min()
                .unwrap_or(900);

            tokio::time::sleep(tokio::time::Duration::from_secs(interval)).await;
        }
    }

    /// Prune RSS items to stay within the configured history limit.
    fn prune_items(&self) {
        // Use the rss_history_limit from config, default 500
        let limit = self.rss_history_limit.unwrap_or(500);
        if let Ok(count) = self.queue_manager.rss_item_count() {
            if count > limit {
                if let Ok(pruned) = self.queue_manager.rss_items_prune(limit) {
                    if pruned > 0 {
                        info!(pruned, "Pruned old RSS items");
                    }
                }
            }
        }
    }

    async fn check_feed(
        &self,
        client: &reqwest::Client,
        feed: &RssFeedConfig,
    ) -> anyhow::Result<()> {
        info!(feed = %feed.name, url = %feed.url, "Checking RSS feed");

        let response = client.get(&feed.url).send().await?;
        let body = response.bytes().await?;
        let parsed = feed_rs::parser::parse(&body[..])?;

        // Compile filter regex if provided
        let filter = feed
            .filter_regex
            .as_ref()
            .and_then(|r| regex::Regex::new(r).ok());

        // Load download rules for this feed
        let rules = self
            .queue_manager
            .rss_rule_list()
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.enabled && r.feed_names.iter().any(|n| n == &feed.name))
            .collect::<Vec<_>>();

        let mut new_items = 0;

        for entry in &parsed.entries {
            let title = entry
                .title
                .as_ref()
                .map(|t| t.content.clone())
                .unwrap_or_default();
            let entry_id = entry.id.clone();

            // Find NZB URL from links or media content
            let nzb_url = Self::extract_nzb_url(entry);

            // Extract size from enclosure/media if available
            let size_bytes = entry
                .media
                .iter()
                .flat_map(|m| &m.content)
                .filter_map(|c| c.size)
                .next()
                .unwrap_or(0);

            // Extract published date
            let published_at = entry.published.or(entry.updated);

            // Persist ALL items to DB (regardless of filter)
            let item = RssItem {
                id: entry_id.clone(),
                feed_name: feed.name.clone(),
                title: title.clone(),
                url: nzb_url.clone(),
                published_at,
                first_seen_at: Utc::now(),
                downloaded: false,
                downloaded_at: None,
                category: feed.category.clone(),
                size_bytes: size_bytes as u64,
            };

            // Check if already in DB (dedup)
            let already_exists = self
                .queue_manager
                .rss_item_exists(&entry_id)
                .unwrap_or(false);

            if !already_exists {
                let _ = self.queue_manager.rss_item_upsert(&item);
                new_items += 1;
            } else {
                continue; // Already processed
            }

            let Some(ref url) = nzb_url else { continue };

            // Check if this item should be auto-downloaded:
            // 1. Feed-level filter must pass (if set)
            let passes_filter = match filter {
                Some(ref re) => re.is_match(&title),
                None => true,
            };

            if !passes_filter {
                continue;
            }

            // 2. Check download rules (applied to pre-filtered items)
            let matched_rule = rules.iter().find(|r| {
                regex::Regex::new(&r.match_regex)
                    .map(|re| re.is_match(&title))
                    .unwrap_or(false)
            });

            // If there are rules for this feed, only download if a rule matches.
            // If there are no rules, use the feed's existing auto-download behavior.
            let (should_download, category, priority) = if !rules.is_empty() {
                if let Some(rule) = matched_rule {
                    (
                        true,
                        rule.category.clone().or_else(|| feed.category.clone()),
                        rule.priority,
                    )
                } else {
                    (false, None, 1)
                }
            } else {
                // No rules: auto-download everything that passes the filter (legacy behavior)
                (true, feed.category.clone(), 1)
            };

            if !should_download {
                continue;
            }

            info!(feed = %feed.name, title = %title, url = %url, "Auto-downloading RSS item");

            match self
                .fetch_and_enqueue(client, url, &title, feed, category.as_deref(), priority)
                .await
            {
                Ok(()) => {
                    let _ = self
                        .queue_manager
                        .rss_item_mark_downloaded(&entry_id, category.as_deref());
                    info!(title = %title, "RSS item enqueued successfully");
                }
                Err(e) => {
                    warn!(title = %title, error = %e, "Failed to enqueue RSS item");
                }
            }
        }

        if new_items > 0 {
            info!(feed = %feed.name, new_items, "RSS feed check complete");
        }

        Ok(())
    }

    /// Extract NZB URL from a feed entry's links or media content.
    fn extract_nzb_url(entry: &feed_rs::model::Entry) -> Option<String> {
        entry
            .links
            .iter()
            .find(|l| {
                l.href.ends_with(".nzb")
                    || l.media_type
                        .as_deref()
                        .is_some_and(|mt| mt == "application/x-nzb")
            })
            .map(|l| l.href.clone())
            .or_else(|| {
                // Check media content for NZB URLs
                entry
                    .media
                    .iter()
                    .flat_map(|m| &m.content)
                    .find(|c| {
                        c.url
                            .as_ref()
                            .is_some_and(|u| u.as_str().ends_with(".nzb"))
                    })
                    .and_then(|c| c.url.as_ref().map(|u| u.to_string()))
            })
            .or_else(|| {
                // Fall back to first link
                entry.links.first().map(|l| l.href.clone())
            })
    }

    async fn fetch_and_enqueue(
        &self,
        client: &reqwest::Client,
        url: &str,
        name: &str,
        feed: &RssFeedConfig,
        category: Option<&str>,
        priority: i32,
    ) -> anyhow::Result<()> {
        let response = client.get(url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("HTTP {}", response.status());
        }
        let data = response.bytes().await?;

        let mut job = nzb_core::nzb_parser::parse_nzb(name, &data)?;

        if let Some(cat) = category {
            job.category = cat.to_string();
        } else if let Some(ref cat) = feed.category {
            job.category = cat.clone();
        }

        job.priority = match priority {
            0 => Priority::Low,
            2 => Priority::High,
            3 => Priority::Force,
            _ => Priority::Normal,
        };

        job.work_dir = self.queue_manager.incomplete_dir().join(&job.id);
        job.output_dir = if !job.category.is_empty() && job.category != "Default" {
            self.queue_manager.complete_dir().join(&job.category)
        } else {
            self.queue_manager.complete_dir().to_path_buf()
        };

        std::fs::create_dir_all(&job.work_dir)?;

        self.queue_manager.add_job(job, Some(data.to_vec()))?;
        Ok(())
    }
}
