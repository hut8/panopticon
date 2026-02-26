use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::session::extract_session_id_from_cookies;
use crate::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct AuthUser {
    pub id: Uuid,
    pub email: String,
    pub email_confirmed: bool,
    pub is_approved: bool,
}

impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);
        let pool = &app_state.db;

        let cookie_header = parts
            .headers
            .get("cookie")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"error": "Not authenticated"})),
                )
                    .into_response()
            })?;

        let session_id = extract_session_id_from_cookies(cookie_header).ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Not authenticated"})),
            )
                .into_response()
        })?;

        let user = get_user_by_session(pool, session_id).await.ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Session expired"})),
            )
                .into_response()
        })?;

        Ok(user)
    }
}

/// Helper trait to extract AppState from state (mirrors axum's FromRef pattern).
pub trait FromRef<T> {
    fn from_ref(input: &T) -> Self;
}

impl FromRef<AppState> for AppState {
    fn from_ref(input: &AppState) -> Self {
        input.clone()
    }
}

async fn get_user_by_session(pool: &PgPool, session_id: &str) -> Option<AuthUser> {
    let row: Option<(Uuid, String, bool, bool)> = sqlx::query_as(
        "SELECT u.id, u.email, u.email_confirmed, u.is_approved \
         FROM users u JOIN sessions s ON u.id = s.user_id \
         WHERE s.id = $1 AND s.expires_at > now()",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .ok()?;

    row.map(|(id, email, email_confirmed, is_approved)| AuthUser {
        id,
        email,
        email_confirmed,
        is_approved,
    })
}
