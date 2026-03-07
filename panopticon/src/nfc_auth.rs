use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use tracing::{error, info, warn};

use crate::middleware::AuthUser;
use crate::session::{create_session, set_session_cookie};
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/nfc/login", post(nfc_login))
        .route("/nfc/register", post(nfc_register))
        .route("/nfc/serial", get(nfc_get_serial))
        .route("/nfc/serial", delete(nfc_unregister))
}

fn is_secure() -> bool {
    std::env::var("BASE_URL")
        .map(|u| u.starts_with("https://"))
        .unwrap_or(false)
}

fn json_error(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({"error": msg}))).into_response()
}

// ── NFC Login ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct NfcLoginRequest {
    serial: String,
}

async fn nfc_login(State(state): State<AppState>, Json(body): Json<NfcLoginRequest>) -> Response {
    let serial = body.serial.trim().to_lowercase();
    if serial.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "Missing NFC serial number");
    }

    let row: Option<(uuid::Uuid, String, bool, bool)> = match sqlx::query_as(
        "SELECT id, email, email_confirmed, is_approved \
         FROM users WHERE nfc_serial = $1",
    )
    .bind(&serial)
    .fetch_optional(&state.db)
    .await
    {
        Ok(row) => row,
        Err(e) => {
            error!("Database error during NFC login: {e}");
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Login failed");
        }
    };

    let (user_id, email, email_confirmed, is_approved) = match row {
        Some(r) => r,
        None => {
            warn!(serial = %serial, "NFC login attempt with unregistered tag");
            return json_error(StatusCode::UNAUTHORIZED, "NFC tag not registered");
        }
    };

    info!(email = %email, "User logged in via NFC");

    let session_id = match create_session(&state.db, user_id).await {
        Ok(id) => id,
        Err(e) => {
            error!("Failed to create session: {e}");
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Login failed");
        }
    };

    let mut response = Json(serde_json::json!({
        "id": user_id,
        "email": email,
        "email_confirmed": email_confirmed,
        "is_approved": is_approved,
    }))
    .into_response();

    response.headers_mut().insert(
        "set-cookie",
        set_session_cookie(&session_id, is_secure())
            .parse()
            .unwrap(),
    );

    response
}

// ── NFC Register ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct NfcRegisterRequest {
    serial: String,
}

async fn nfc_register(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<NfcRegisterRequest>,
) -> Response {
    let serial = body.serial.trim().to_lowercase();
    if serial.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "Missing NFC serial number");
    }

    // Check if another user already has this serial
    let existing: Option<(uuid::Uuid,)> = match sqlx::query_as(
        "SELECT id FROM users WHERE nfc_serial = $1 AND id != $2",
    )
    .bind(&serial)
    .bind(user.id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(row) => row,
        Err(e) => {
            error!("Failed to check NFC serial: {e}");
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Registration failed");
        }
    };

    if existing.is_some() {
        return json_error(StatusCode::CONFLICT, "This NFC tag is already registered to another account");
    }

    match sqlx::query("UPDATE users SET nfc_serial = $1 WHERE id = $2")
        .bind(&serial)
        .bind(user.id)
        .execute(&state.db)
        .await
    {
        Ok(_) => {
            info!(user_id = %user.id, serial = %serial, "NFC serial registered");
            Json(serde_json::json!({ "serial": serial })).into_response()
        }
        Err(e) => {
            error!("Failed to register NFC serial: {e}");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "Registration failed")
        }
    }
}

// ── NFC Get Serial ───────────────────────────────────────────────────────────

async fn nfc_get_serial(State(state): State<AppState>, user: AuthUser) -> Response {
    let row: Option<(Option<String>,)> = match sqlx::query_as(
        "SELECT nfc_serial FROM users WHERE id = $1",
    )
    .bind(user.id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(row) => row,
        Err(e) => {
            error!("Failed to get NFC serial: {e}");
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to get NFC serial");
        }
    };

    let serial = row.and_then(|(s,)| s);
    Json(serde_json::json!({ "serial": serial })).into_response()
}

// ── NFC Unregister ───────────────────────────────────────────────────────────

async fn nfc_unregister(State(state): State<AppState>, user: AuthUser) -> Response {
    match sqlx::query("UPDATE users SET nfc_serial = NULL WHERE id = $1")
        .bind(user.id)
        .execute(&state.db)
        .await
    {
        Ok(_) => {
            info!(user_id = %user.id, "NFC serial removed");
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            error!("Failed to remove NFC serial: {e}");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to remove NFC serial")
        }
    }
}
