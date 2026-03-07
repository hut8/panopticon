use axum::{
    extract::{Path, State},
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
        .route("/nfc/tokens", get(nfc_list_tokens))
        .route("/nfc/tokens/{id}", delete(nfc_delete_token))
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
        "SELECT u.id, u.email, u.email_confirmed, u.is_approved \
         FROM users u JOIN nfc_tokens n ON u.id = n.user_id \
         WHERE n.serial = $1",
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
    label: Option<String>,
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

    let label = body.label.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty());

    let row: Option<(uuid::Uuid,)> = match sqlx::query_as(
        "INSERT INTO nfc_tokens (user_id, serial, label) VALUES ($1, $2, $3) \
         ON CONFLICT (serial) DO NOTHING \
         RETURNING id",
    )
    .bind(user.id)
    .bind(&serial)
    .bind(label)
    .fetch_optional(&state.db)
    .await
    {
        Ok(row) => row,
        Err(e) => {
            error!("Failed to register NFC token: {e}");
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Registration failed");
        }
    };

    match row {
        Some((id,)) => {
            info!(user_id = %user.id, serial = %serial, "NFC token registered");
            Json(serde_json::json!({
                "id": id,
                "serial": serial,
                "label": label,
            }))
            .into_response()
        }
        None => json_error(
            StatusCode::CONFLICT,
            "This NFC tag is already registered",
        ),
    }
}

// ── NFC List Tokens ──────────────────────────────────────────────────────────

async fn nfc_list_tokens(State(state): State<AppState>, user: AuthUser) -> Response {
    let rows: Vec<(uuid::Uuid, String, Option<String>, chrono::DateTime<chrono::Utc>)> =
        match sqlx::query_as(
            "SELECT id, serial, label, created_at FROM nfc_tokens WHERE user_id = $1 ORDER BY created_at DESC",
        )
        .bind(user.id)
        .fetch_all(&state.db)
        .await
        {
            Ok(rows) => rows,
            Err(e) => {
                error!("Failed to list NFC tokens: {e}");
                return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to list tokens");
            }
        };

    let tokens: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(id, serial, label, created_at)| {
            serde_json::json!({
                "id": id,
                "serial": serial,
                "label": label,
                "created_at": created_at,
            })
        })
        .collect();

    Json(tokens).into_response()
}

// ── NFC Delete Token ─────────────────────────────────────────────────────────

async fn nfc_delete_token(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<uuid::Uuid>,
) -> Response {
    let result = match sqlx::query("DELETE FROM nfc_tokens WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.id)
        .execute(&state.db)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to delete NFC token: {e}");
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to delete token");
        }
    };

    if result.rows_affected() == 0 {
        return json_error(StatusCode::NOT_FOUND, "Token not found");
    }

    info!(user_id = %user.id, token_id = %id, "NFC token removed");
    StatusCode::NO_CONTENT.into_response()
}
