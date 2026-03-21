use serde::Serialize;

pub const MB: u64 = 1024 * 1024;
pub const GB: u64 = 1024 * MB;

pub const ARTICLE_SIZE: u64 = 750_000;
pub const NNTP_GROUP: &str = "alt.binaries.test";
pub const MSG_ID_DOMAIN: &str = "benchnzb";

pub const SABNZBD_API: &str = "http://sabnzbd:8080";
pub const SABNZBD_API_KEY: &str = "benchnzb0123456789abcdef01234567";
pub const RUSTNZBD_API: &str = "http://rustnzbd:9090";

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
