mod oauth;

use axum::{
    Router,
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use include_dir::{Dir, include_dir};
use mime_guess::from_path;
use tracing::info;

// Embed web assets into the binary at compile time
static ASSETS: Dir<'_> = include_dir!("web/build");

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "panopticon=info,tower_http=info".into()),
        )
        .init();

    let app = Router::new()
        .nest("/auth", oauth::router())
        .fallback(handle_static_file);

    let addr = "0.0.0.0:1337";
    info!("Panopticon listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn handle_static_file(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    // Handle root path
    if path.is_empty() || path == "index.html" {
        if let Some(index_file) = ASSETS.get_file("index.html") {
            let mut headers = HeaderMap::new();
            headers.insert("content-type", "text/html".parse().unwrap());
            headers.insert(
                "cache-control",
                "public, max-age=0, must-revalidate".parse().unwrap(),
            );
            return (StatusCode::OK, headers, index_file.contents()).into_response();
        }
    }

    // Try to find the file in embedded assets
    if let Some(file) = ASSETS.get_file(path) {
        let mut headers = HeaderMap::new();
        let content_type = from_path(path).first_or_octet_stream();
        headers.insert("content-type", content_type.as_ref().parse().unwrap());

        // Hashed assets get long cache, others get short cache
        if path.starts_with("_app/") || path.starts_with("assets/") {
            headers.insert(
                "cache-control",
                "public, max-age=31536000, immutable".parse().unwrap(),
            );
        } else {
            headers.insert(
                "cache-control",
                "public, max-age=3600, must-revalidate".parse().unwrap(),
            );
        }

        return (StatusCode::OK, headers, file.contents()).into_response();
    }

    // Client-side routing fallback: serve index.html for non-file paths
    if !path.contains('.') && path != "favicon.ico" {
        if let Some(index_file) = ASSETS.get_file("index.html") {
            let mut headers = HeaderMap::new();
            headers.insert("content-type", "text/html".parse().unwrap());
            headers.insert(
                "cache-control",
                "public, max-age=0, must-revalidate".parse().unwrap(),
            );
            return (StatusCode::OK, headers, index_file.contents()).into_response();
        }
    }

    (StatusCode::NOT_FOUND, "Not Found").into_response()
}
