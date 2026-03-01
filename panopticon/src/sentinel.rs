use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::middleware::AuthUser;
use crate::ws::WsEvent;
use crate::AppState;

type ApiError = (StatusCode, &'static str);

// ── Request / response types ────────────────────────────────────────────────

#[derive(Deserialize)]
struct ScanRequest {
    tag_id: String,
    secret: String,
}

#[derive(Serialize)]
struct ScanResponse {
    action: String,
}

#[derive(Serialize)]
struct ModeResponse {
    mode: String,
}

#[derive(Deserialize)]
struct SetModeRequest {
    mode: String,
}

#[derive(Serialize)]
struct CardResponse {
    id: Uuid,
    tag_id: String,
    label: Option<String>,
    created_at: String,
}

#[derive(Serialize)]
struct ScanLogEntry {
    id: Uuid,
    tag_id: String,
    action: String,
    created_at: String,
}

#[derive(Serialize)]
pub struct SentinelResponse {
    pub id: Uuid,
    pub name: String,
    pub connected: bool,
    pub last_connected_at: Option<String>,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct SentinelLogEntry {
    pub id: Uuid,
    pub sentinel_id: Uuid,
    pub message: String,
    pub created_at: String,
}

#[derive(Deserialize)]
struct LogsQuery {
    limit: Option<i64>,
}

// ── Router ──────────────────────────────────────────────────────────────────

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/scan", post(handle_scan))
        .route("/mode", get(get_mode))
        .route("/mode", post(set_mode))
        .route("/cards", get(list_cards))
        .route("/cards/{id}", delete(remove_card))
        .route("/scan-log", get(scan_log))
        .route("/sentinels", get(list_sentinels))
        .route("/sentinels/{id}/logs", get(sentinel_logs))
}

// ── Tag ID validation ───────────────────────────────────────────────────────

/// Validate tag_id format: 5 colon-separated uppercase hex bytes (e.g. "80:00:48:23:4C")
pub fn is_valid_tag_id(tag_id: &str) -> bool {
    let parts: Vec<&str> = tag_id.split(':').collect();
    if parts.len() != 5 {
        return false;
    }
    parts.iter().all(|part| {
        part.len() == 2
            && part
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase())
    })
}

// ── Shared scan logic ───────────────────────────────────────────────────────

/// Core scan processing logic shared by both the HTTP handler and TCP handler.
/// Returns the action string ("enrolled", "granted", or "denied").
pub async fn process_scan(state: &AppState, tag_id: &str) -> Result<String, String> {
    // Read current mode
    let mode: String =
        sqlx::query_scalar("SELECT value FROM system_config WHERE key = 'sentinel_mode'")
            .fetch_one(&state.db)
            .await
            .map_err(|e| {
                error!("Failed to read sentinel mode: {e:#}");
                "Database error".to_string()
            })?;

    let action = match mode.as_str() {
        "enroll" => {
            sqlx::query("INSERT INTO access_cards (tag_id) VALUES ($1) ON CONFLICT DO NOTHING")
                .bind(tag_id)
                .execute(&state.db)
                .await
                .map_err(|e| {
                    error!("Failed to enroll card: {e:#}");
                    "Database error".to_string()
                })?;

            info!(tag_id = %tag_id, "Card enrolled");
            "enrolled"
        }
        _ => {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM access_cards WHERE tag_id = $1)")
                    .bind(tag_id)
                    .fetch_one(&state.db)
                    .await
                    .map_err(|e| {
                        error!("Failed to check card: {e:#}");
                        "Database error".to_string()
                    })?;

            if exists {
                // Attempt to unlock via U-Tec
                if let Some(client) = state.auth_store.client().await {
                    match client.discover_locks().await {
                        Ok(locks) => {
                            if let Some(lock) = locks.first() {
                                match client.unlock(lock).await {
                                    Ok(_) => {
                                        info!(tag_id = %tag_id, lock = %lock.name, "Door unlocked")
                                    }
                                    Err(e) => {
                                        error!(tag_id = %tag_id, "Failed to unlock: {e:#}")
                                    }
                                }
                            } else {
                                warn!("No locks found on U-Tec account");
                            }
                        }
                        Err(e) => error!("Failed to discover locks: {e:#}"),
                    }
                } else {
                    warn!("U-Tec not connected — cannot unlock");
                }

                info!(tag_id = %tag_id, "Access granted");
                "granted"
            } else {
                warn!(tag_id = %tag_id, "Access denied");
                "denied"
            }
        }
    };

    // Log to scan_log
    let scan_row: (Uuid, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
        "INSERT INTO scan_log (tag_id, action) VALUES ($1, $2) RETURNING id, created_at",
    )
    .bind(tag_id)
    .bind(action)
    .fetch_one(&state.db)
    .await
    .map_err(|e| {
        error!("Failed to log scan: {e:#}");
        "Database error".to_string()
    })?;

    let _ = state.events.send(WsEvent::Scan {
        tag_id: tag_id.to_string(),
        action: action.to_string(),
        created_at: scan_row.1.to_rfc3339(),
    });

    // If enrolled, also broadcast the new card
    if action == "enrolled" {
        let card: Option<(Uuid, String, Option<String>, chrono::DateTime<chrono::Utc>)> =
            sqlx::query_as(
                "SELECT id, tag_id, label, created_at FROM access_cards WHERE tag_id = $1",
            )
            .bind(tag_id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();

        if let Some((id, tag_id, label, created_at)) = card {
            let _ = state.events.send(WsEvent::CardAdded {
                id,
                tag_id,
                label,
                created_at: created_at.to_rfc3339(),
            });
        }
    }

    Ok(action.to_string())
}

// ── Handlers ────────────────────────────────────────────────────────────────

async fn handle_scan(
    State(state): State<AppState>,
    Json(req): Json<ScanRequest>,
) -> Result<Json<ScanResponse>, ApiError> {
    if req.secret != state.sentinel_secret {
        return Err((StatusCode::UNAUTHORIZED, "Invalid secret"));
    }

    if !is_valid_tag_id(&req.tag_id) {
        return Err((StatusCode::BAD_REQUEST, "Invalid tag_id format"));
    }

    let action = process_scan(&state, &req.tag_id)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    Ok(Json(ScanResponse { action }))
}

async fn get_mode(
    _user: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<ModeResponse>, ApiError> {
    let mode: String =
        sqlx::query_scalar("SELECT value FROM system_config WHERE key = 'sentinel_mode'")
            .fetch_one(&state.db)
            .await
            .map_err(|e| {
                error!("Failed to read mode: {e:#}");
                (StatusCode::INTERNAL_SERVER_ERROR, "Database error")
            })?;

    Ok(Json(ModeResponse { mode }))
}

async fn set_mode(
    _user: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<SetModeRequest>,
) -> Result<Json<ModeResponse>, ApiError> {
    if req.mode != "guard" && req.mode != "enroll" {
        return Err((StatusCode::BAD_REQUEST, "Mode must be 'guard' or 'enroll'"));
    }

    sqlx::query("UPDATE system_config SET value = $1 WHERE key = 'sentinel_mode'")
        .bind(&req.mode)
        .execute(&state.db)
        .await
        .map_err(|e| {
            error!("Failed to set mode: {e:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error")
        })?;

    info!(mode = %req.mode, "Sentinel mode changed");

    let _ = state.events.send(WsEvent::ModeChanged {
        mode: req.mode.clone(),
    });

    Ok(Json(ModeResponse { mode: req.mode }))
}

async fn list_cards(
    _user: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<CardResponse>>, ApiError> {
    let rows: Vec<(Uuid, String, Option<String>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, tag_id, label, created_at FROM access_cards ORDER BY created_at DESC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| {
        error!("Failed to list cards: {e:#}");
        (StatusCode::INTERNAL_SERVER_ERROR, "Database error")
    })?;

    let cards = rows
        .into_iter()
        .map(|(id, tag_id, label, created_at)| CardResponse {
            id,
            tag_id,
            label,
            created_at: created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(cards))
}

async fn remove_card(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let result = sqlx::query("DELETE FROM access_cards WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(|e| {
            error!("Failed to remove card: {e:#}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error")
        })?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Card not found"));
    }

    info!(%id, "Card removed");

    let _ = state.events.send(WsEvent::CardRemoved { id });

    Ok(StatusCode::NO_CONTENT)
}

async fn scan_log(
    _user: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<ScanLogEntry>>, ApiError> {
    let rows: Vec<(Uuid, String, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, tag_id, action, created_at FROM scan_log ORDER BY created_at DESC LIMIT 50",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| {
        error!("Failed to read scan log: {e:#}");
        (StatusCode::INTERNAL_SERVER_ERROR, "Database error")
    })?;

    let entries = rows
        .into_iter()
        .map(|(id, tag_id, action, created_at)| ScanLogEntry {
            id,
            tag_id,
            action,
            created_at: created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(entries))
}

// ── Sentinel endpoints ──────────────────────────────────────────────────────

type SentinelRow = (
    Uuid,
    String,
    bool,
    Option<chrono::DateTime<chrono::Utc>>,
    chrono::DateTime<chrono::Utc>,
);

async fn list_sentinels(
    _user: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<SentinelResponse>>, ApiError> {
    let rows: Vec<SentinelRow> = sqlx::query_as(
        "SELECT id, name, connected, last_connected_at, created_at FROM sentinels ORDER BY created_at",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| {
        error!("Failed to list sentinels: {e:#}");
        (StatusCode::INTERNAL_SERVER_ERROR, "Database error")
    })?;

    let sentinels = rows
        .into_iter()
        .map(
            |(id, name, connected, last_connected_at, created_at)| SentinelResponse {
                id,
                name,
                connected,
                last_connected_at: last_connected_at.map(|t| t.to_rfc3339()),
                created_at: created_at.to_rfc3339(),
            },
        )
        .collect();

    Ok(Json(sentinels))
}

async fn sentinel_logs(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<LogsQuery>,
) -> Result<Json<Vec<SentinelLogEntry>>, ApiError> {
    let limit = query.limit.unwrap_or(200).min(1000);

    let rows: Vec<(Uuid, Uuid, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, sentinel_id, message, created_at FROM sentinel_logs \
         WHERE sentinel_id = $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(id)
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .map_err(|e| {
        error!("Failed to read sentinel logs: {e:#}");
        (StatusCode::INTERNAL_SERVER_ERROR, "Database error")
    })?;

    let entries = rows
        .into_iter()
        .map(|(id, sentinel_id, message, created_at)| SentinelLogEntry {
            id,
            sentinel_id,
            message,
            created_at: created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(entries))
}
