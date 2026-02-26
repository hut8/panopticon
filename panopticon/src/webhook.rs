//! Webhook receiver for U-Tec device event notifications.
//!
//! U-Tec pushes device state changes (lock/unlock, battery, online/offline)
//! to a registered webhook URL. The notification payload uses the same
//! envelope format as API responses, so we reuse `DeviceWithStates` for
//! parsing.
//!
//! Authentication: U-Tec echoes back the `access_token` we provided during
//! registration as a query parameter. We validate it against the stored
//! notification token in `AuthStore`.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::post,
    Json, Router,
};
use serde::Deserialize;
use tracing::{info, warn};

use crate::utec::DeviceWithStates;
use crate::ws::WsEvent;
use crate::AppState;

#[derive(Deserialize)]
struct WebhookParams {
    access_token: Option<String>,
}

/// The notification payload — same envelope as U-Tec API responses.
#[derive(Deserialize)]
struct NotificationBody {
    #[allow(dead_code)]
    header: Option<serde_json::Value>,
    payload: NotificationPayload,
}

#[derive(Deserialize)]
struct NotificationPayload {
    devices: Vec<DeviceWithStates>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/utec", post(handle_utec_notification))
}

async fn handle_utec_notification(
    State(state): State<AppState>,
    Query(params): Query<WebhookParams>,
    Json(body): Json<NotificationBody>,
) -> StatusCode {
    // Validate notification token
    let expected = match state.auth_store.notification_token().await {
        Some(t) => t,
        None => {
            warn!("Webhook received but no notification token configured");
            return StatusCode::UNAUTHORIZED;
        }
    };

    let provided = params.access_token.unwrap_or_default();
    if provided != expected {
        warn!("Webhook received with invalid token");
        return StatusCode::UNAUTHORIZED;
    }

    // Process each device's state changes
    for device in &body.payload.devices {
        if let Some(lock_state) = device.lock_state() {
            info!(
                device_id = %device.id,
                lock_state = %lock_state,
                "Webhook: lock state change"
            );
            let _ = state.events.send(WsEvent::LockState {
                device_id: device.id.clone(),
                lock_state,
            });
        }
    }

    StatusCode::OK
}
