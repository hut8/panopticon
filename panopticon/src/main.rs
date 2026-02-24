mod api;
mod auth_store;
mod db;
mod email;
mod email_auth;
mod ip_whitelist;
mod middleware;
mod oauth;
mod session;
pub mod utec;

use axum::{
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    Router,
};
use include_dir::{include_dir, Dir};
use mime_guess::from_path;
use sqlx::PgPool;
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::{info, Level};

use auth_store::AuthStore;
use email::Mailer;

// Embed web assets into the binary at compile time
static ASSETS: Dir<'_> = include_dir!("web/build");

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub auth_store: AuthStore,
    pub mailer: Mailer,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env file if present (development); in production, systemd
    // provides environment variables via EnvironmentFile.
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .without_time()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "panopticon=info,tower_http=info".into()),
        )
        .init();

    let db = db::init_pool().await?;
    let auth_store = AuthStore::new()?;
    let mailer = Mailer::new()?;
    let whitelist = ip_whitelist::load_whitelist()?;

    let state = AppState {
        db,
        auth_store,
        mailer,
    };

    let app = Router::new()
        .nest("/api/auth", email_auth::router())
        .nest("/api", api::router())
        .nest("/auth", oauth::router())
        .fallback(handle_static_file)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &axum::http::Request<_>| {
                    let client_ip = request
                        .headers()
                        .get("x-forwarded-for")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.split(',').next())
                        .map(|s| s.trim().to_string())
                        .unwrap_or_else(|| "-".into());
                    tracing::info_span!(
                        "request",
                        method = %request.method(),
                        uri = %request.uri(),
                        client_ip = %client_ip,
                    )
                })
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(axum::middleware::from_fn(move |req, next| {
            ip_whitelist::check(whitelist.clone(), req, next)
        }))
        .with_state(state);

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
