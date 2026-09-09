//! SSRF guard for server-side URL fetches.
//!
//! Any endpoint that fetches a caller-supplied URL (the native
//! `POST /api/queue/add-url`, RSS feed fetches, and the SABnzbd-compatible
//! `mode=addurl`) must route through [`validate_fetch_url`] before issuing the
//! request. The guard rejects non-http(s) schemes and any host that resolves
//! to a private, loopback, or otherwise non-globally-routable address, and
//! [`build_fetch_client`] pins the connection to the exact addresses that were
//! validated so a hostname cannot re-resolve to an internal address between
//! the check and the request (DNS rebinding).

use std::net::{IpAddr, SocketAddr};

use crate::error::ApiError;

#[derive(Debug)]
pub struct FetchUrlPlan {
    pub url: reqwest::Url,
    resolved_addrs: Option<(String, Vec<SocketAddr>)>,
}

impl FetchUrlPlan {
    /// Whether a request for this URL needs a client pinned to the validated
    /// addresses. True when the host was a resolved hostname (guards against
    /// DNS rebinding); false for an IP-literal URL, where a shared pooled
    /// client is safe to reuse.
    pub fn requires_pinned_client(&self) -> bool {
        self.resolved_addrs.is_some()
    }
}

/// Returns `Err` if `raw_url` is not http/https or resolves to a
/// private/reserved address.
pub async fn validate_fetch_url(raw_url: &str) -> Result<FetchUrlPlan, ApiError> {
    let url = reqwest::Url::parse(raw_url)
        .map_err(|e| ApiError::from(anyhow::anyhow!("Invalid URL: {e}")))?;

    match url.scheme() {
        "http" | "https" => {}
        s => {
            return Err(ApiError::from(anyhow::anyhow!(
                "URL scheme '{s}' not allowed (must be http or https)"
            )));
        }
    }

    let host = url
        .host_str()
        .ok_or_else(|| ApiError::from(anyhow::anyhow!("URL has no host")))?
        .to_string();

    // IP literal: validate directly without a DNS round-trip.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if !is_globally_routable(ip) {
            return Err(ApiError::from(anyhow::anyhow!(
                "URL targets a private/reserved address"
            )));
        }
        return Ok(FetchUrlPlan {
            url,
            resolved_addrs: None,
        });
    }

    // Hostname: resolve and check every returned address.
    let port = url.port_or_known_default().unwrap_or(80);
    let addrs: Vec<_> = tokio::net::lookup_host(format!("{host}:{port}"))
        .await
        .map_err(|e| ApiError::from(anyhow::anyhow!("DNS resolution failed for '{host}': {e}")))?
        .collect();

    if addrs.is_empty() {
        return Err(ApiError::from(anyhow::anyhow!(
            "DNS resolution returned no addresses for '{host}'"
        )));
    }

    for addr in &addrs {
        if !is_globally_routable(addr.ip()) {
            return Err(ApiError::from(anyhow::anyhow!(
                "URL resolves to a private/reserved address"
            )));
        }
    }

    Ok(FetchUrlPlan {
        url,
        resolved_addrs: Some((host, addrs)),
    })
}

/// Build a reqwest client pinned to the addresses validated in `plan`, so a
/// hostname cannot re-resolve to an internal address after the check.
pub fn build_fetch_client(plan: &FetchUrlPlan) -> Result<reqwest::Client, ApiError> {
    let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30));
    if let Some((host, addrs)) = &plan.resolved_addrs {
        builder = builder.resolve_to_addrs(host, addrs.as_slice());
    }
    builder
        .build()
        .map_err(|e| ApiError::from(anyhow::anyhow!("Failed to build fetch client: {e}")))
}

/// Read a response body, failing if it exceeds `max_bytes` (avoids unbounded
/// memory use from a hostile or misconfigured URL).
pub async fn read_response_bytes_limited(
    mut response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, ApiError> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| ApiError::from(anyhow::anyhow!("Failed to read response: {e}")))?
    {
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(ApiError::from(anyhow::anyhow!(
                "Fetched body exceeds the {} MB limit",
                max_bytes / 1024 / 1024
            )));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn is_globally_routable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !v4.is_loopback()
                && !v4.is_private()
                && !v4.is_link_local()
                && !v4.is_broadcast()
                && !v4.is_unspecified()
                && !v4.is_documentation()
        }
        IpAddr::V6(v6) => {
            !v6.is_loopback() && !v6.is_unspecified() && !v6.is_multicast() && !v6.is_unique_local()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn validate_fetch_url_rejects_private_ip_literals() {
        let err = validate_fetch_url("http://127.0.0.1/file.nzb")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("private/reserved"));
    }

    #[tokio::test]
    async fn validate_fetch_url_rejects_localhost_hostname() {
        let err = validate_fetch_url("http://localhost/file.nzb")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("private/reserved"));
    }

    #[tokio::test]
    async fn validate_fetch_url_rejects_link_local_metadata() {
        // 169.254.169.254 is the cloud-metadata endpoint; must be refused.
        let err = validate_fetch_url("http://169.254.169.254/latest/meta-data/")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("private/reserved"));
    }

    #[tokio::test]
    async fn validate_fetch_url_rejects_non_http_scheme() {
        let err = validate_fetch_url("file:///etc/passwd").await.unwrap_err();
        assert!(err.to_string().contains("not allowed"));
    }
}
