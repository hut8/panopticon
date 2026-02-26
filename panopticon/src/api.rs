use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use tracing::error;

use crate::middleware::AuthUser;
use crate::utec::UTec;
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

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/devices", get(list_devices))
        .route("/devices/{id}/lock", post(lock_device))
        .route("/devices/{id}/unlock", post(unlock_device))
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
            DeviceResponse {
                id: lock.id.clone(),
                name: lock.name.clone(),
                lock_state: device_states.and_then(|s| s.lock_state()),
                battery_level: device_states.and_then(|s| s.battery_level()),
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

    Ok(Json(LockActionResponse {
        success: true,
        lock_state,
    }))
}
