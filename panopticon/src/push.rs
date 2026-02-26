use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio::sync::broadcast;
use tracing::{error, info, warn};
use web_push::{
    ContentEncoding, PartialVapidSignatureBuilder, SubscriptionInfo, VapidSignatureBuilder,
    WebPushMessageBuilder,
};

use crate::middleware::AuthUser;
use crate::ws::WsEvent;
use crate::AppState;

type ApiError = (StatusCode, &'static str);

// ── Config ──────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct PushConfig {
    vapid_builder: PartialVapidSignatureBuilder,
    vapid_public_key: String,
}

impl PushConfig {
    pub fn new() -> Result<Option<Self>> {
        let key_path = match std::env::var("VAPID_PRIVATE_KEY_PATH") {
            Ok(v) => v,
            Err(_) => {
                info!("VAPID_PRIVATE_KEY_PATH not set, push notifications disabled");
                return Ok(None);
            }
        };
        let public_key = std::env::var("VAPID_PUBLIC_KEY")
            .context("VAPID_PUBLIC_KEY must be set when VAPID_PRIVATE_KEY_PATH is set")?;

        let pem_file =
            std::fs::File::open(&key_path).with_context(|| format!("open {key_path}"))?;
        let vapid_builder = VapidSignatureBuilder::from_pem_no_sub(pem_file)
            .map_err(|e| anyhow::anyhow!("Failed to load VAPID key: {e}"))?;

        info!("Push notifications enabled (VAPID key loaded)");

        Ok(Some(Self {
            vapid_builder,
            vapid_public_key: public_key,
        }))
    }
}

// ── API endpoints ───────────────────────────────────────────────────────────

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/push/vapid-key", get(vapid_key))
        .route("/push/subscribe", post(subscribe))
        .route("/push/unsubscribe", post(unsubscribe))
}

#[derive(Serialize)]
struct VapidKeyResponse {
    key: String,
}

async fn vapid_key(
    _user: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<VapidKeyResponse>, ApiError> {
    let config = state
        .push_config
        .as_ref()
        .ok_or((StatusCode::NOT_FOUND, "Push notifications not configured"))?;

    Ok(Json(VapidKeyResponse {
        key: config.vapid_public_key.clone(),
    }))
}

#[derive(Deserialize)]
struct SubscribeRequest {
    endpoint: String,
    p256dh: String,
    auth: String,
}

async fn subscribe(
    user: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<SubscribeRequest>,
) -> Result<StatusCode, ApiError> {
    state
        .push_config
        .as_ref()
        .ok_or((StatusCode::NOT_FOUND, "Push notifications not configured"))?;

    sqlx::query(
        "INSERT INTO push_subscriptions (user_id, endpoint, p256dh, auth)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (endpoint) DO UPDATE SET user_id = $1, p256dh = $3, auth = $4",
    )
    .bind(user.id)
    .bind(&body.endpoint)
    .bind(&body.p256dh)
    .bind(&body.auth)
    .execute(&state.db)
    .await
    .map_err(|e| {
        error!("Failed to save push subscription: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to save subscription",
        )
    })?;

    info!(user_id = %user.id, "Push subscription saved");
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct UnsubscribeRequest {
    endpoint: String,
}

async fn unsubscribe(
    user: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<UnsubscribeRequest>,
) -> Result<StatusCode, ApiError> {
    sqlx::query("DELETE FROM push_subscriptions WHERE endpoint = $1 AND user_id = $2")
        .bind(&body.endpoint)
        .bind(user.id)
        .execute(&state.db)
        .await
        .map_err(|e| {
            error!("Failed to delete push subscription: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to remove subscription",
            )
        })?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Background notifier ─────────────────────────────────────────────────────

struct PushSubscriptionRow {
    id: uuid::Uuid,
    endpoint: String,
    p256dh: String,
    auth: String,
}

impl PushSubscriptionRow {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            id: row.try_get("id")?,
            endpoint: row.try_get("endpoint")?,
            p256dh: row.try_get("p256dh")?,
            auth: row.try_get("auth")?,
        })
    }
}

pub async fn spawn_push_notifier(
    mut rx: broadcast::Receiver<WsEvent>,
    pool: PgPool,
    config: PushConfig,
) {
    let http_client = reqwest::Client::new();

    info!("Push notifier started");
    loop {
        let event = match rx.recv().await {
            Ok(e) => e,
            Err(broadcast::error::RecvError::Lagged(n)) => {
                warn!("Push notifier lagged, skipped {n} events");
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => {
                info!("Push notifier shutting down (channel closed)");
                return;
            }
        };

        let (title, body) = match &event {
            WsEvent::Scan { tag_id, action, .. } => {
                let title = format!(
                    "Access {}",
                    if action == "granted" {
                        "Granted"
                    } else {
                        "Denied"
                    }
                );
                let body = format!("Card {} — {}", tag_id, action);
                (title, body)
            }
            WsEvent::LockState {
                device_id,
                lock_state,
            } => {
                let title = format!("Lock {}", lock_state);
                let body = format!("{} is now {}", device_id, lock_state);
                (title, body)
            }
            _ => continue,
        };

        let payload = serde_json::json!({ "title": title, "body": body }).to_string();

        let rows: Vec<PushSubscriptionRow> = match sqlx::query(
            "SELECT ps.id, ps.endpoint, ps.p256dh, ps.auth
             FROM push_subscriptions ps
             JOIN users u ON u.id = ps.user_id
             WHERE u.is_approved = TRUE",
        )
        .fetch_all(&pool)
        .await
        {
            Ok(raw) => {
                let mut out = Vec::with_capacity(raw.len());
                for row in &raw {
                    match PushSubscriptionRow::from_row(row) {
                        Ok(r) => out.push(r),
                        Err(e) => {
                            error!("Failed to parse push subscription row: {e}");
                        }
                    }
                }
                out
            }
            Err(e) => {
                error!("Failed to query push subscriptions: {e}");
                continue;
            }
        };

        for row in rows {
            let sub_info = SubscriptionInfo::new(&row.endpoint, &row.p256dh, &row.auth);

            let sig = match config.vapid_builder.clone().add_sub_info(&sub_info).build() {
                Ok(s) => s,
                Err(e) => {
                    error!(endpoint = %row.endpoint, "VAPID signing failed: {e}");
                    continue;
                }
            };

            let mut builder = WebPushMessageBuilder::new(&sub_info);
            builder.set_payload(ContentEncoding::Aes128Gcm, payload.as_bytes());
            builder.set_vapid_signature(sig);

            let message = match builder.build() {
                Ok(m) => m,
                Err(e) => {
                    error!(endpoint = %row.endpoint, "Failed to build push message: {e}");
                    continue;
                }
            };

            // Build the HTTP request from the WebPushMessage
            let endpoint = message.endpoint.to_string();
            let mut req = http_client.post(&endpoint).header("TTL", message.ttl);

            if let Some(payload) = message.payload {
                req = req
                    .header("Content-Encoding", payload.content_encoding.to_str())
                    .header("Content-Type", "application/octet-stream");

                for (k, v) in &payload.crypto_headers {
                    req = req.header(*k, v);
                }
                req = req.body(payload.content);
            }

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        // ok
                    } else if status == reqwest::StatusCode::GONE
                        || status == reqwest::StatusCode::NOT_FOUND
                    {
                        warn!(endpoint = %row.endpoint, "Push endpoint stale ({status}), removing");
                        let _ = sqlx::query("DELETE FROM push_subscriptions WHERE id = $1")
                            .bind(row.id)
                            .execute(&pool)
                            .await;
                    } else {
                        let body_text = resp.text().await.unwrap_or_default();
                        error!(
                            endpoint = %row.endpoint,
                            status = %status,
                            body = %body_text,
                            "Push delivery failed"
                        );
                    }
                }
                Err(e) => {
                    error!(endpoint = %row.endpoint, "Push HTTP request failed: {e}");
                }
            }
        }
    }
}
