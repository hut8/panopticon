use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, error, warn};
use uuid::Uuid;

use crate::middleware::AuthUser;
use crate::utec::{Device, DeviceWithStates, LockUser, UTec};
use crate::ws::WsEvent;
use crate::AppState;

type ApiError = (StatusCode, &'static str);

#[derive(Serialize)]
struct DeviceResponse {
    id: String,
    name: String,
    lock_state: Option<String>,
    battery_level: Option<u64>,
    online: bool,
}

#[derive(Serialize)]
struct LockActionResponse {
    success: bool,
    lock_state: Option<String>,
}

#[derive(Serialize)]
struct NotificationPrefs {
    email: bool,
}

#[derive(Deserialize)]
struct UpdateNotificationPrefs {
    email: bool,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/devices", get(list_devices))
        .route("/devices/{id}/lock", post(lock_device))
        .route("/devices/{id}/unlock", post(unlock_device))
        .route("/devices/{id}/users", get(list_lock_users))
        .route(
            "/notifications",
            get(get_notifications).put(update_notifications),
        )
        .route("/admin/pending-users", get(list_pending_users))
        .route("/admin/users/{id}/approve", post(approve_user))
        .route("/admin/users/{id}", delete(delete_user))
}

async fn get_client(state: &AppState) -> Result<UTec, ApiError> {
    state
        .auth_store
        .client()
        .await
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "U-Tec not connected"))
}

async fn list_devices(
    _user: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<DeviceResponse>>, ApiError> {
    let client = get_client(&state).await?;

    let locks = client.discover_locks().await.map_err(|e| {
        error!("Failed to discover locks: {e:#}");
        (StatusCode::BAD_GATEWAY, "Failed to discover locks")
    })?;

    let lock_refs: Vec<&_> = locks.iter().collect();
    let states = client.query_devices(&lock_refs).await.map_err(|e| {
        error!("Failed to query device states: {e:#}");
        (StatusCode::BAD_GATEWAY, "Failed to query device states")
    })?;

    let devices: Vec<DeviceResponse> = locks
        .iter()
        .map(|lock| {
            let device_states = states.iter().find(|s| s.id == lock.id);
            let battery_level = device_states.and_then(|s| s.battery_level()).map(|raw| {
                // Normalize battery level to 0-100% using the device's
                // batteryLevelRange from discovery (e.g. min=1, max=5).
                let (min, max) = lock
                    .attributes
                    .as_ref()
                    .and_then(|a| a.get("batteryLevelRange"))
                    .map(|r| {
                        let min = r.get("min").and_then(|v| v.as_u64()).unwrap_or(0);
                        let max = r.get("max").and_then(|v| v.as_u64()).unwrap_or(100);
                        (min, max)
                    })
                    .unwrap_or((0, 100));
                if max <= min {
                    return raw;
                }
                ((raw.saturating_sub(min)) * 100) / (max - min)
            });
            DeviceResponse {
                id: lock.id.clone(),
                name: lock.name.clone(),
                lock_state: device_states.and_then(|s| s.lock_state()),
                battery_level,
                online: device_states.is_some_and(|s| s.is_online()),
            }
        })
        .collect();

    Ok(Json(devices))
}

async fn lock_device(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<LockActionResponse>, ApiError> {
    let client = get_client(&state).await?;

    let locks = client.discover_locks().await.map_err(|e| {
        error!("Failed to discover locks: {e:#}");
        (StatusCode::BAD_GATEWAY, "Failed to discover locks")
    })?;

    let device = locks
        .iter()
        .find(|d| d.id == id)
        .ok_or((StatusCode::NOT_FOUND, "Device not found"))?;

    let results = client.lock(device).await.map_err(|e| {
        error!("Failed to lock device {id}: {e:#}");
        (StatusCode::BAD_GATEWAY, "Failed to lock device")
    })?;

    let lock_state = handle_lock_response(&state, &id, device, &results);

    Ok(Json(LockActionResponse {
        success: true,
        lock_state,
    }))
}

async fn unlock_device(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<LockActionResponse>, ApiError> {
    let client = get_client(&state).await?;

    let locks = client.discover_locks().await.map_err(|e| {
        error!("Failed to discover locks: {e:#}");
        (StatusCode::BAD_GATEWAY, "Failed to discover locks")
    })?;

    let device = locks
        .iter()
        .find(|d| d.id == id)
        .ok_or((StatusCode::NOT_FOUND, "Device not found"))?;

    let results = client.unlock(device).await.map_err(|e| {
        error!("Failed to unlock device {id}: {e:#}");
        (StatusCode::BAD_GATEWAY, "Failed to unlock device")
    })?;

    let lock_state = handle_lock_response(&state, &id, device, &results);

    Ok(Json(LockActionResponse {
        success: true,
        lock_state,
    }))
}

/// Maximum seconds we'll wait for a deferred lock response before giving up.
const MAX_DEFERRED_WAIT_SECS: u64 = 60;

/// Handle a lock/unlock command response: if the lock state is immediately
/// available, broadcast it via WebSocket. If the API returns a deferred
/// response (st.deferredResponse), spawn a background task that waits the
/// indicated number of seconds, then queries the device and broadcasts the
/// resulting lock state.
fn handle_lock_response(
    state: &AppState,
    device_id: &str,
    device: &Device,
    results: &[DeviceWithStates],
) -> Option<String> {
    let device_result = results.iter().find(|s| s.id == device_id);
    let lock_state = device_result.and_then(|s| s.lock_state());

    if let Some(ref ls) = lock_state {
        let _ = state.events.send(WsEvent::LockState {
            device_id: device_id.to_string(),
            lock_state: ls.clone(),
        });
    } else if let Some(seconds) = device_result
        .and_then(|s| s.get_state("st.deferredResponse", "seconds"))
        .and_then(|s| s.value.as_u64())
    {
        let seconds = if seconds > MAX_DEFERRED_WAIT_SECS {
            warn!(
                device_id,
                seconds, "Deferred wait exceeds maximum, capping at {MAX_DEFERRED_WAIT_SECS}s"
            );
            MAX_DEFERRED_WAIT_SECS
        } else {
            seconds
        };

        let state = state.clone();
        let device_id = device_id.to_string();
        let device = device.clone();
        tokio::spawn(async move {
            debug!(device_id, seconds, "Waiting for deferred lock response");
            tokio::time::sleep(Duration::from_secs(seconds)).await;

            let Some(client) = state.auth_store.client().await else {
                error!(device_id, "No U-Tec client available for deferred poll");
                return;
            };
            match client.query_device(&device).await {
                Ok(device_states) => {
                    if let Some(ls) = device_states.lock_state() {
                        debug!(device_id, lock_state = %ls, "Deferred lock state resolved");
                        let _ = state.events.send(WsEvent::LockState {
                            device_id,
                            lock_state: ls,
                        });
                    } else {
                        warn!(device_id, "Deferred query returned no lock state");
                    }
                }
                Err(e) => {
                    error!(
                        device_id,
                        "Failed to query device after deferred wait: {e:#}"
                    );
                }
            }
        });
    } else {
        warn!(
            device_id,
            "Lock command response contained neither lock state nor deferred response"
        );
    }

    lock_state
}

async fn list_lock_users(
    _user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<LockUser>>, ApiError> {
    let client = get_client(&state).await?;

    let locks = client.discover_locks().await.map_err(|e| {
        error!("Failed to discover locks: {e:#}");
        (StatusCode::BAD_GATEWAY, "Failed to discover locks")
    })?;

    let device = locks
        .iter()
        .find(|d| d.id == id)
        .ok_or((StatusCode::NOT_FOUND, "Device not found"))?;

    let users = client.list_lock_users(device).await.map_err(|e| {
        error!("Failed to list lock users for device {id}: {e:#}");
        (StatusCode::BAD_GATEWAY, "Failed to list lock users")
    })?;

    Ok(Json(users))
}

async fn get_notifications(
    user: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<NotificationPrefs>, ApiError> {
    let email: bool = sqlx::query_scalar("SELECT notify_email FROM users WHERE id = $1")
        .bind(user.id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| {
            error!("Failed to fetch notification prefs: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to fetch preferences",
            )
        })?;

    Ok(Json(NotificationPrefs { email }))
}

async fn update_notifications(
    user: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<UpdateNotificationPrefs>,
) -> Result<Json<NotificationPrefs>, ApiError> {
    sqlx::query("UPDATE users SET notify_email = $1 WHERE id = $2")
        .bind(body.email)
        .bind(user.id)
        .execute(&state.db)
        .await
        .map_err(|e| {
            error!("Failed to update notification prefs: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to update preferences",
            )
        })?;

    Ok(Json(NotificationPrefs { email: body.email }))
}

// ── Admin: pending users ─────────────────────────────────────────────

#[derive(Serialize)]
struct PendingUser {
    id: Uuid,
    email: String,
    email_confirmed: bool,
    created_at: String,
}

fn require_approved(user: &AuthUser) -> Result<(), ApiError> {
    if !user.is_approved {
        return Err((StatusCode::FORBIDDEN, "Not authorized"));
    }
    Ok(())
}

async fn list_pending_users(
    user: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<PendingUser>>, ApiError> {
    require_approved(&user)?;

    let rows: Vec<(Uuid, String, bool, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, email, email_confirmed, created_at FROM users WHERE is_approved = FALSE",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| {
        error!("Failed to fetch pending users: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to fetch pending users",
        )
    })?;

    let users = rows
        .into_iter()
        .map(|(id, email, email_confirmed, created_at)| PendingUser {
            id,
            email,
            email_confirmed,
            created_at: created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(users))
}

async fn approve_user(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_approved(&user)?;

    let email: Option<String> = sqlx::query_scalar(
        "UPDATE users SET is_approved = TRUE WHERE id = $1 AND is_approved = FALSE RETURNING email",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        error!("Failed to approve user: {e}");
        (StatusCode::INTERNAL_SERVER_ERROR, "Failed to approve user")
    })?;

    let email = email.ok_or((StatusCode::NOT_FOUND, "User not found or already approved"))?;

    if let Err(e) = state.mailer.send_approval_email(&email).await {
        error!(to = %email, "Failed to send approval email: {e}");
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn delete_user(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_approved(&user)?;

    // Delete sessions first (foreign key), then the user
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(|e| {
            error!("Failed to delete user sessions: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to delete user")
        })?;

    let result = sqlx::query("DELETE FROM users WHERE id = $1 AND is_approved = FALSE")
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(|e| {
            error!("Failed to delete user: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to delete user")
        })?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "User not found or already approved"));
    }

    Ok(StatusCode::NO_CONTENT)
}
