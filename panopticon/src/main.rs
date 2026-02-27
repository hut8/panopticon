mod api;
mod auth_store;
mod db;
mod email;
mod email_auth;
mod geo_access;
mod ip_whitelist;
mod middleware;
mod oauth;
mod push;
mod sentinel;
mod session;
pub mod utec;
mod webhook;
mod ws;

use axum::{
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    Router,
};
use include_dir::{include_dir, Dir};
use mime_guess::from_path;
use sqlx::PgPool;
use tokio::sync::broadcast;
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::{info, Level};

use auth_store::AuthStore;
use email::Mailer;
use push::PushConfig;

// Embed web assets into the binary at compile time
static ASSETS: Dir<'_> = include_dir!("web/build");

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub auth_store: AuthStore,
    pub mailer: Mailer,
    pub push_config: Option<PushConfig>,
    pub sentinel_secret: String,
    pub events: broadcast::Sender<ws::WsEvent>,
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
    let push_config = PushConfig::new()?;
    let whitelist = ip_whitelist::load_whitelist()?;
    let geo = geo_access::GeoAccess::init().await;
    if geo.is_enabled() {
        geo.spawn_gpsd_task();
    }

    let sentinel_secret =
        std::env::var("SENTINEL_SECRET").unwrap_or_else(|_| "changeme".to_string());

    let (events_tx, _) = broadcast::channel::<ws::WsEvent>(64);

    // Spawn email notifier on access events
    let email_rx = events_tx.subscribe();
    tokio::spawn(email::spawn_email_notifier(
        email_rx,
        db.clone(),
        mailer.clone(),
    ));

    // Spawn push notifier if VAPID keys are configured
    if let Some(ref pc) = push_config {
        let push_rx = events_tx.subscribe();
        tokio::spawn(push::spawn_push_notifier(push_rx, db.clone(), pc.clone()));
    }

    let state = AppState {
        db,
        auth_store,
        mailer,
        push_config,
        sentinel_secret,
        events: events_tx,
    };

    // Routes behind the IP whitelist (all normal app routes)
    let protected = Router::new()
        .nest("/api/auth", email_auth::router())
        .nest("/api/sentinel", sentinel::router())
        .nest("/api", push::router())
        .nest("/api", api::router())
        .nest("/api", ws::router())
        .nest("/auth", oauth::router())
        .fallback(handle_static_file)
        .layer(axum::middleware::from_fn(move |req, next| {
            ip_whitelist::check(whitelist.clone(), geo.clone(), req, next)
        }));

    // Webhook routes are outside the IP whitelist — they authenticate
    // via a notification token instead.
    let app = Router::new()
        .nest("/api/webhooks", webhook::router())
        .merge(protected)
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
                    // Log only the path (no query string) to avoid leaking
                    // the webhook notification token.
                    let path = request.uri().path();
                    tracing::info_span!(
                        "request",
                        method = %request.method(),
                        uri = %path,
                        client_ip = %client_ip,
                    )
                })
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .with_state(state);

    let addr = "127.0.0.1:1337";
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
