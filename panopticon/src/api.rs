use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tracing::error;

use crate::middleware::AuthUser;
use crate::utec::UTec;
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
        .route(
            "/notifications",
            get(get_notifications).put(update_notifications),
        )
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

    let lock_state = results
        .iter()
        .find(|s| s.id == id)
        .and_then(|s| s.lock_state());

    if let Some(ref ls) = lock_state {
        let _ = state.events.send(WsEvent::LockState {
            device_id: id,
            lock_state: ls.clone(),
        });
    }

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

    let lock_state = results
        .iter()
        .find(|s| s.id == id)
        .and_then(|s| s.lock_state());

    if let Some(ref ls) = lock_state {
        let _ = state.events.send(WsEvent::LockState {
            device_id: id,
            lock_state: ls.clone(),
        });
    }

    Ok(Json(LockActionResponse {
        success: true,
        lock_state,
    }))
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
