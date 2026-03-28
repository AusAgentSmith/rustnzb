use serde::Serialize;

pub const MB: u64 = 1024 * 1024;
pub const GB: u64 = 1024 * MB;

pub const ARTICLE_SIZE: u64 = 750_000;
pub const NNTP_GROUP: &str = "alt.binaries.test";
pub const MSG_ID_DOMAIN: &str = "benchnzb";

pub const SABNZBD_API: &str = "http://sabnzbd:8080";
pub const SABNZBD_API_KEY: &str = "benchnzb0123456789abcdef01234567";
pub const RUSTNZB_API: &str = "http://rustnzb:9090";

pub const POLL_INTERVAL_MS: u64 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum TestType {
    Raw,
    Par2,
    Unpack,
}

impl std::fmt::Display for TestType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TestType::Raw => write!(f, "raw"),
            TestType::Par2 => write!(f, "par2"),
            TestType::Unpack => write!(f, "unpack"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Scenario {
    pub name: String,
    pub description: String,
    pub total_size: u64,
    pub test_type: TestType,
    pub missing_pct: f64,
    pub redundancy_pct: f64,
    pub timeout_secs: u64,
}

pub fn size_label(size: u64) -> String {
    if size >= GB {
        format!("{}gb", size / GB)
    } else {
        format!("{}mb", size / MB)
    }
}

fn make_scenario(size: u64, test_type: TestType) -> Scenario {
    let label = size_label(size);
    let type_str = test_type.to_string();
    let name = format!("sz{}_{}", label, type_str);

    let missing_pct = if test_type == TestType::Par2 {
        3.0
    } else {
        0.0
    };
    let redundancy_pct = if matches!(test_type, TestType::Par2 | TestType::Unpack) {
        8.0
    } else {
        0.0
    };

    let base_timeout = std::cmp::max(600, (size / GB) * 120);
    let type_bonus = match test_type {
        TestType::Raw => 0,
        TestType::Par2 => 600,
        TestType::Unpack => 900,
    };
    let timeout_secs = base_timeout + type_bonus;

    let desc = match test_type {
        TestType::Raw => format!("{} GB raw download", size / GB),
        TestType::Par2 => format!(
            "{} GB download + par2 repair ({:.0}% missing)",
            size / GB,
            missing_pct
        ),
        TestType::Unpack => format!("{} GB download + 7z extraction", size / GB),
    };

    Scenario {
        name,
        description: desc,
        total_size: size,
        test_type,
        missing_pct,
        redundancy_pct,
        timeout_secs,
    }
}

fn generate_all() -> Vec<Scenario> {
    let sizes = vec![5 * GB, 10 * GB, 50 * GB];
    let types = vec![TestType::Raw, TestType::Par2, TestType::Unpack];
    let mut scenarios = Vec::new();
    for &size in &sizes {
        for &tt in &types {
            scenarios.push(make_scenario(size, tt));
        }
    }
    scenarios
}

/// Window size for stress test analysis (seconds).
pub const STRESS_WINDOW_SECS: u64 = 300;

/// Parse a human-readable duration string like "1h", "30m", "2h30m", "4h15m".
pub fn parse_duration(s: &str) -> anyhow::Result<std::time::Duration> {
    let s = s.trim().to_lowercase();
    let mut total_secs: u64 = 0;
    let mut num_buf = String::new();

    for c in s.chars() {
        if c.is_ascii_digit() {
            num_buf.push(c);
        } else {
            let n: u64 = num_buf
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid duration: {s}"))?;
            num_buf.clear();
            match c {
                'h' => total_secs += n * 3600,
                'm' => total_secs += n * 60,
                's' => total_secs += n,
                _ => anyhow::bail!("Unknown duration unit '{c}' in: {s}"),
            }
        }
    }

    // Handle bare number (no unit) — treat as seconds
    if !num_buf.is_empty() {
        let n: u64 = num_buf
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid duration: {s}"))?;
        total_secs += n;
    }

    if total_secs == 0 {
        anyhow::bail!("Duration must be > 0: {s}");
    }

    Ok(std::time::Duration::from_secs(total_secs))
}

/// Parse a human-readable size string like "1gb", "500mb", "10gb".
pub fn parse_size(s: &str) -> anyhow::Result<u64> {
    let s = s.trim().to_lowercase();

    if let Some(n) = s.strip_suffix("gb") {
        let v: u64 = n.parse().map_err(|_| anyhow::anyhow!("Invalid size: {s}"))?;
        Ok(v * GB)
    } else if let Some(n) = s.strip_suffix("mb") {
        let v: u64 = n.parse().map_err(|_| anyhow::anyhow!("Invalid size: {s}"))?;
        Ok(v * MB)
    } else {
        // Try as raw bytes
        let v: u64 = s.parse().map_err(|_| anyhow::anyhow!("Invalid size: {s}"))?;
        Ok(v)
    }
}

/// Format bytes as a human-readable size string.
pub fn format_size(bytes: u64) -> String {
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Format a Duration as a human-readable string like "2h 30m 15s".
pub fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h {m:02}m {s:02}s")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

pub fn resolve_scenarios(selector: &str) -> Vec<Scenario> {
    let all = generate_all();
    let sel = selector.trim().to_lowercase();

    match sel.as_str() {
        "all" | "full" => all,
        "quick" => all
            .iter()
            .filter(|s| s.name == "sz5gb_raw")
            .cloned()
            .collect(),
        "medium" => all
            .iter()
            .filter(|s| s.total_size <= 10 * GB)
            .cloned()
            .collect(),
        "speed" => all
            .iter()
            .filter(|s| s.test_type == TestType::Raw)
            .cloned()
            .collect(),
        "postproc" => all
            .iter()
            .filter(|s| s.test_type != TestType::Raw && s.total_size <= 10 * GB)
            .cloned()
            .collect(),
        _ => {
            let names: Vec<&str> = sel.split(',').map(|s| s.trim()).collect();
            let matched: Vec<Scenario> = all
                .iter()
                .filter(|s| names.contains(&s.name.as_str()))
                .cloned()
                .collect();
            if matched.is_empty() {
                tracing::error!("No scenarios matched: {selector}");
                tracing::info!("Groups: quick, medium, speed, postproc, full");
            }
            matched
        }
    }
}
