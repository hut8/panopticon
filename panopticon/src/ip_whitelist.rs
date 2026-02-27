use std::net::IpAddr;
use std::sync::Arc;

use axum::{
    extract::Request,
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{Html, IntoResponse, Response},
};
use ipnet::IpNet;
use tracing::{info, warn};

use crate::auth_store::resolve_auth_path;
use crate::geo_access::GeoAccess;

/// Load the IP whitelist from `ip_whitelist.txt` in the same directory as `auth.json`.
pub fn load_whitelist() -> anyhow::Result<Arc<Vec<IpNet>>> {
    let path = resolve_auth_path().with_file_name("ip-whitelist.txt");

    let contents = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {e}", path.display()))?;

    let mut entries = Vec::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let net: IpNet = if line.contains('/') {
            line.parse()
                .map_err(|e| anyhow::anyhow!("Invalid CIDR '{line}': {e}"))?
        } else {
            // Bare IP — parse as a single-host network
            let addr: IpAddr = line
                .parse()
                .map_err(|e| anyhow::anyhow!("Invalid IP '{line}': {e}"))?;
            IpNet::from(addr)
        };
        entries.push(net);
    }

    info!(
        path = %path.display(),
        count = entries.len(),
        "Loaded IP whitelist"
    );

    Ok(Arc::new(entries))
}

/// Middleware that rejects requests from IPs not in the whitelist or geo radius.
pub async fn check(
    whitelist: Arc<Vec<IpNet>>,
    geo: GeoAccess,
    req: Request,
    next: Next,
) -> Response {
    let client_ip = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "-".into());

    if let Ok(addr) = client_ip.parse::<IpAddr>() {
        // Fast path: IP whitelist check.
        if whitelist.iter().any(|net| net.contains(&addr)) {
            return next.run(req).await;
        }

        // Fallback: geo-proximity check.
        if geo.is_within_radius(addr).await {
            info!(client_ip = %client_ip, "Allowed by geo proximity");
            return next.run(req).await;
        }
    }

    warn!(client_ip = %client_ip, "Blocked by IP whitelist and geo check");

    (
        StatusCode::FORBIDDEN,
        HeaderMap::from_iter([(
            axum::http::header::CONTENT_TYPE,
            "text/html".parse().unwrap(),
        )]),
        Html(FORBIDDEN_HTML),
    )
        .into_response()
}

static FORBIDDEN_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head><meta charset="utf-8"><title>403 Unauthorized</title></head>
<body style="font-family:sans-serif;display:flex;justify-content:center;align-items:center;height:100vh;margin:0;background:#111;color:#c33">
<h1>Unauthorized</h1>
</body>
</html>"#;
