//! SABnzbd configuration importer.
//!
//! Parses SABnzbd INI files and API responses into a preview structure
//! that can be reviewed, edited, and applied to rustnzb.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::config::{CategoryConfig, RssFeedConfig, ServerConfig};

// ---------------------------------------------------------------------------
// Public structs
// ---------------------------------------------------------------------------

/// Preview returned by both INI and API import — same shape for both.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SabnzbdImportPreview {
    pub servers: Vec<ImportedServer>,
    pub categories: Vec<CategoryConfig>,
    pub general: ImportedGeneral,
    pub rss_feeds: Vec<RssFeedConfig>,
    /// Warnings about partially-imported features.
    pub warnings: Vec<String>,
    /// Fields/sections that were skipped entirely.
    pub skipped_fields: Vec<String>,
}

/// A server imported from SABnzbd.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedServer {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub ssl: bool,
    pub ssl_verify: bool,
    pub username: Option<String>,
    pub password: Option<String>,
    /// True when the password was masked (imported via API).
    pub password_masked: bool,
    pub connections: u16,
    pub priority: u8,
    pub enabled: bool,
    pub retention: u32,
    pub optional: bool,
}

impl ImportedServer {
    /// Convert to a rustnzb `ServerConfig`, generating a new UUID.
    pub fn to_server_config(&self) -> ServerConfig {
        ServerConfig {
            id: uuid::Uuid::new_v4().to_string(),
            name: self.name.clone(),
            host: self.host.clone(),
            port: self.port,
            ssl: self.ssl,
            ssl_verify: self.ssl_verify,
            username: self.username.clone(),
            password: self.password.clone(),
            connections: self.connections,
            priority: self.priority,
            enabled: self.enabled,
            retention: self.retention,
            pipelining: 1,
            optional: self.optional,
            compress: false,
        }
    }
}

/// General settings imported from SABnzbd.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedGeneral {
    pub api_key: Option<String>,
    pub complete_dir: Option<String>,
    pub incomplete_dir: Option<String>,
    pub speed_limit_bps: u64,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse SABnzbd bandwidth limit string (e.g. "50M", "1G", "500K", "0", "").
pub fn parse_bandwidth_limit(s: &str) -> u64 {
    let s = s.trim().trim_matches('"');
    if s.is_empty() || s == "0" {
        return 0;
    }
    let (num_part, multiplier) = if let Some(n) = s.strip_suffix(['K', 'k']) {
        (n, 1024u64)
    } else if let Some(n) = s.strip_suffix(['M', 'm']) {
        (n, 1024 * 1024)
    } else if let Some(n) = s.strip_suffix(['G', 'g']) {
        (n, 1024 * 1024 * 1024)
    } else {
        // Plain number = bytes/sec in SABnzbd (KB/s)
        (s, 1024u64)
    };
    num_part.trim().parse::<u64>().unwrap_or(0) * multiplier
}

/// Parse SABnzbd-style boolean ("0"/"1").
pub fn parse_ini_bool(s: &str) -> bool {
    matches!(s.trim(), "1" | "yes" | "true" | "True")
}

// ---------------------------------------------------------------------------
// INI Parser
// ---------------------------------------------------------------------------

type SectionMap = HashMap<(String, String), HashMap<String, String>>;

/// Parse a raw SABnzbd INI file into section/subsection key-value maps.
fn parse_ini_sections(content: &str) -> SectionMap {
    let mut sections: SectionMap = HashMap::new();
    let mut current_section = String::new();
    let mut current_subsection = String::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        // [[subsection]]
        if line.starts_with("[[") && line.ends_with("]]") {
            current_subsection = line[2..line.len() - 2].to_string();
            continue;
        }

        // [section]
        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].to_string();
            current_subsection.clear();
            continue;
        }

        // key = value
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim().to_string();
            let value = value.trim();
            // Strip surrounding quotes (SABnzbd uses dir = "" for empty values)
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(value)
                .to_string();
            sections
                .entry((current_section.clone(), current_subsection.clone()))
                .or_default()
                .insert(key, value);
        }
    }

    sections
}

/// Known SABnzbd features we don't import.
const SKIPPED_SECTIONS: &[&str] = &["sorting", "notifications", "schedules"];

/// Parse a SABnzbd INI file into an import preview.
pub fn parse_sabnzbd_ini(content: &str) -> SabnzbdImportPreview {
    let sections = parse_ini_sections(content);
    let mut warnings = Vec::new();
    let mut skipped_fields = Vec::new();

    // --- General (from [misc]) ---
    let misc = sections
        .get(&("misc".into(), String::new()))
        .cloned()
        .unwrap_or_default();

    let general = ImportedGeneral {
        api_key: misc.get("api_key").cloned().filter(|s| !s.is_empty()),
        complete_dir: misc.get("complete_dir").cloned().filter(|s| !s.is_empty()),
        incomplete_dir: misc.get("download_dir").cloned().filter(|s| !s.is_empty()),
        speed_limit_bps: misc
            .get("bandwidth_limit")
            .map(|s| parse_bandwidth_limit(s))
            .unwrap_or(0),
    };

    // --- Servers (from [servers] → [[name]]) ---
    let servers: Vec<ImportedServer> = sections
        .iter()
        .filter(|((section, subsection), _)| section == "servers" && !subsection.is_empty())
        .map(|((_, _), kv)| build_imported_server(kv, false))
        .collect();

    // --- Categories (from [categories] → [[name]]) ---
    let categories: Vec<CategoryConfig> = sections
        .iter()
        .filter(|((section, subsection), _)| section == "categories" && !subsection.is_empty())
        .map(|((_, _), kv)| {
            let name = kv.get("name").map(|s| s.as_str()).unwrap_or("*");
            let name = if name == "*" { "Default" } else { name };

            // Check for scripts
            if let Some(script) = kv.get("script")
                && script != "Default"
                && !script.is_empty()
            {
                warnings.push(format!(
                    "Category '{}': script '{}' not imported (rustnzb doesn't support scripts)",
                    name, script
                ));
            }

            CategoryConfig {
                name: name.to_string(),
                output_dir: kv
                    .get("dir")
                    .filter(|s| !s.is_empty())
                    .map(std::path::PathBuf::from),
                post_processing: kv.get("pp").and_then(|s| s.parse().ok()).unwrap_or(3),
            }
        })
        .collect();

    // --- RSS feeds (from [rss] → [[name]]) ---
    let rss_feeds: Vec<RssFeedConfig> = sections
        .iter()
        .filter(|((section, subsection), _)| section == "rss" && !subsection.is_empty())
        .filter_map(|((_, subsection), kv)| {
            let url = kv
                .get("uri")
                .or_else(|| kv.get("url"))
                .cloned()
                .filter(|s| !s.is_empty())?;

            let filter_regex = kv
                .get("filter")
                .or_else(|| kv.get("filters"))
                .cloned()
                .filter(|s| !s.is_empty());

            if filter_regex.is_some() {
                warnings.push(format!(
                    "RSS feed '{}': complex filter simplified to first include pattern",
                    subsection
                ));
            }

            Some(RssFeedConfig {
                name: subsection.clone(),
                url,
                poll_interval_secs: 900,
                category: kv.get("cat").cloned().filter(|s| !s.is_empty() && s != "*"),
                filter_regex,
                enabled: kv.get("enable").map(|s| parse_ini_bool(s)).unwrap_or(true),
                auto_download: false,
            })
        })
        .collect();

    // --- Skipped fields ---
    for &section in SKIPPED_SECTIONS {
        if sections.keys().any(|(s, _)| s == section) {
            skipped_fields.push(format!("[{section}] — not supported by rustnzb"));
        }
    }

    // Check for duplicate-detection settings
    if misc.get("no_dupes").is_some_and(|v| v != "0") {
        skipped_fields.push("Duplicate detection settings — not yet supported".into());
    }

    SabnzbdImportPreview {
        servers,
        categories,
        general,
        rss_feeds,
        warnings,
        skipped_fields,
    }
}

// ---------------------------------------------------------------------------
// API Response Parser
// ---------------------------------------------------------------------------

/// Parse the JSON response from SABnzbd's `get_config` API mode.
pub fn parse_sabnzbd_api_response(json: &serde_json::Value) -> SabnzbdImportPreview {
    let config = &json["config"];
    let misc = &config["misc"];
    let mut warnings = Vec::new();
    let mut skipped_fields = Vec::new();

    // --- General ---
    let general = ImportedGeneral {
        api_key: misc["api_key"]
            .as_str()
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty()),
        complete_dir: misc["complete_dir"]
            .as_str()
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty()),
        incomplete_dir: misc["download_dir"]
            .as_str()
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty()),
        speed_limit_bps: misc["bandwidth_limit"]
            .as_str()
            .map(parse_bandwidth_limit)
            .or_else(|| misc["bandwidth_limit"].as_u64())
            .unwrap_or(0),
    };

    // --- Servers ---
    let servers: Vec<ImportedServer> = config["servers"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|s| {
                    let password = s["password"].as_str().map(|p| p.to_string());
                    let password_masked = password
                        .as_ref()
                        .is_some_and(|p| p.contains('*'));

                    if password_masked {
                        let name = s["displayname"]
                            .as_str()
                            .or(s["name"].as_str())
                            .unwrap_or("unknown");
                        warnings.push(format!(
                            "Server '{}': password is masked (***) — you'll need to enter it manually",
                            name
                        ));
                    }

                    ImportedServer {
                        name: s["displayname"]
                            .as_str()
                            .or(s["name"].as_str())
                            .unwrap_or("")
                            .to_string(),
                        host: s["host"].as_str().unwrap_or("").to_string(),
                        port: s["port"]
                            .as_u64()
                            .or_else(|| s["port"].as_str().and_then(|p| p.parse().ok()))
                            .unwrap_or(563) as u16,
                        ssl: s["ssl"].as_u64().unwrap_or(0) != 0
                            || s["ssl"].as_bool().unwrap_or(false),
                        ssl_verify: s["ssl_verify"].as_u64().unwrap_or(0) != 0
                            || s["ssl_verify"].as_bool().unwrap_or(false),
                        username: s["username"]
                            .as_str()
                            .map(|u| u.to_string())
                            .filter(|u| !u.is_empty()),
                        password: password.filter(|p| !p.is_empty()),
                        password_masked,
                        connections: s["connections"]
                            .as_u64()
                            .or_else(|| s["connections"].as_str().and_then(|c| c.parse().ok()))
                            .unwrap_or(8) as u16,
                        priority: s["priority"]
                            .as_u64()
                            .or_else(|| s["priority"].as_str().and_then(|p| p.parse().ok()))
                            .unwrap_or(0) as u8,
                        enabled: s["enable"].as_u64().unwrap_or(1) != 0
                            || s["enable"].as_bool().unwrap_or(true),
                        retention: s["retention"]
                            .as_u64()
                            .or_else(|| s["retention"].as_str().and_then(|r| r.parse().ok()))
                            .unwrap_or(0) as u32,
                        optional: s["optional"].as_u64().unwrap_or(0) != 0
                            || s["optional"].as_bool().unwrap_or(false),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // --- Categories ---
    let categories: Vec<CategoryConfig> = config["categories"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|c| {
                    let name = c["name"].as_str().unwrap_or("*");
                    let name = if name == "*" { "Default" } else { name };

                    if let Some(script) = c["script"].as_str()
                        && script != "Default"
                        && !script.is_empty()
                    {
                        warnings.push(format!(
                            "Category '{}': script '{}' not imported",
                            name, script
                        ));
                    }

                    CategoryConfig {
                        name: name.to_string(),
                        output_dir: c["dir"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .map(std::path::PathBuf::from),
                        post_processing: c["pp"]
                            .as_u64()
                            .or_else(|| c["pp"].as_str().and_then(|p| p.parse().ok()))
                            .unwrap_or(3) as u8,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // --- Skipped ---
    for &section in SKIPPED_SECTIONS {
        if config[section].is_object() || config[section].is_array() {
            skipped_fields.push(format!("[{section}] — not supported by rustnzb"));
        }
    }

    SabnzbdImportPreview {
        servers,
        categories,
        general,
        rss_feeds: Vec::new(), // SABnzbd API doesn't return RSS config in get_config
        warnings,
        skipped_fields,
    }
}

// ---------------------------------------------------------------------------
// Helpers for building servers from INI key-value maps
// ---------------------------------------------------------------------------

fn build_imported_server(kv: &HashMap<String, String>, from_api: bool) -> ImportedServer {
    let password = kv.get("password").cloned().filter(|s| !s.is_empty());
    let password_masked = from_api && password.as_ref().is_some_and(|p| p.contains('*'));

    ImportedServer {
        name: kv
            .get("displayname")
            .or(kv.get("name"))
            .cloned()
            .unwrap_or_default(),
        host: kv.get("host").cloned().unwrap_or_default(),
        port: kv.get("port").and_then(|s| s.parse().ok()).unwrap_or(563),
        ssl: kv.get("ssl").map(|s| parse_ini_bool(s)).unwrap_or(false),
        ssl_verify: kv
            .get("ssl_verify")
            .map(|s| parse_ini_bool(s))
            .unwrap_or(false),
        username: kv.get("username").cloned().filter(|s| !s.is_empty()),
        password: password.clone(),
        password_masked,
        connections: kv
            .get("connections")
            .and_then(|s| s.parse().ok())
            .unwrap_or(8),
        priority: kv.get("priority").and_then(|s| s.parse().ok()).unwrap_or(0),
        enabled: kv.get("enable").map(|s| parse_ini_bool(s)).unwrap_or(true),
        retention: kv
            .get("retention")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        optional: kv
            .get("optional")
            .map(|s| parse_ini_bool(s))
            .unwrap_or(false),
    }
}
