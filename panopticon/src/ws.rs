use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::get,
    Router,
};
use serde::Serialize;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::session::{extract_session_id_from_cookies, get_user_by_session};
use crate::AppState;

// ── Event types ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum WsEvent {
    Scan {
        tag_id: String,
        action: String,
        created_at: String,
    },
    ModeChanged {
        mode: String,
    },
    CardAdded {
        id: Uuid,
        tag_id: String,
        label: Option<String>,
        created_at: String,
    },
    CardRemoved {
        id: Uuid,
    },
    LockState {
        device_id: String,
        lock_state: String,
    },
}

impl WsEvent {
    pub fn to_message(&self) -> Message {
        Message::text(serde_json::to_string(self).unwrap())
    }
}

// ── Router ──────────────────────────────────────────────────────────────────

pub fn router() -> Router<AppState> {
    Router::new().route("/ws", get(ws_handler))
}

// ── Handler ─────────────────────────────────────────────────────────────────

async fn ws_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, StatusCode> {
    // Validate session before upgrading
    let cookie_header = headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let session_id =
        extract_session_id_from_cookies(cookie_header).ok_or(StatusCode::UNAUTHORIZED)?;

    get_user_by_session(&state.db, session_id)
        .await
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let rx = state.events.subscribe();
    Ok(ws.on_upgrade(move |socket| handle_socket(socket, rx)))
}

async fn handle_socket(mut socket: WebSocket, mut rx: broadcast::Receiver<WsEvent>) {
    loop {
        tokio::select! {
            // Forward broadcast events to client
            result = rx.recv() => {
                match result {
                    Ok(event) => {
                        if socket.send(event.to_message()).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("WebSocket client lagged, skipped {n} events");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            // Handle incoming frames (ping/pong/close)
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        if socket.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(_)) => {} // ignore text/binary from client
                    Some(Err(_)) => break,
                }
            }
        }
    }
}
