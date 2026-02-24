use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{Duration, Utc};
use rand::Rng;
use serde::Deserialize;
use tracing::{error, info, warn};

use crate::middleware::AuthUser;
use crate::session::{
    clear_session_cookie, create_session, delete_session, extract_session_id_from_cookies,
    set_session_cookie,
};
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/me", get(me))
        .route("/confirm-email", get(confirm_email))
        .route("/resend-confirmation", post(resend_confirmation))
        .route("/forgot-password", post(forgot_password))
        .route("/reset-password", post(reset_password))
}

fn is_secure() -> bool {
    std::env::var("BASE_URL")
        .map(|u| u.starts_with("https://"))
        .unwrap_or(false)
}

fn generate_token() -> String {
    let bytes: [u8; 32] = rand::thread_rng().r#gen();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn json_error(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({"error": msg}))).into_response()
}

// ── Register ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct RegisterRequest {
    email: String,
    password: String,
}

async fn register(State(state): State<AppState>, Json(body): Json<RegisterRequest>) -> Response {
    let email = body.email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        return json_error(StatusCode::BAD_REQUEST, "Invalid email address");
    }
    if body.password.len() < 8 {
        return json_error(
            StatusCode::BAD_REQUEST,
            "Password must be at least 8 characters",
        );
    }

    let password_hash = match hash_password(&body.password) {
        Ok(h) => h,
        Err(e) => {
            error!("Failed to hash password: {e}");
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Registration failed");
        }
    };

    let user_id: Option<(uuid::Uuid,)> = match sqlx::query_as(
        "INSERT INTO users (email, password_hash) VALUES ($1, $2) \
         ON CONFLICT (email) DO NOTHING \
         RETURNING id",
    )
    .bind(&email)
    .bind(&password_hash)
    .fetch_optional(&state.db)
    .await
    {
        Ok(row) => row,
        Err(e) => {
            error!("Database error during registration: {e}");
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Registration failed");
        }
    };

    let user_id = match user_id {
        Some((id,)) => id,
        None => {
            return json_error(
                StatusCode::CONFLICT,
                "An account with this email already exists",
            );
        }
    };

    info!(email = %email, "New user registered");

    // Send confirmation email
    let token = generate_token();
    let expires_at = Utc::now() + Duration::hours(24);
    if let Err(e) = sqlx::query(
        "INSERT INTO email_tokens (id, user_id, token_type, expires_at) VALUES ($1, $2, 'confirmation', $3)",
    )
    .bind(&token)
    .bind(user_id)
    .bind(expires_at)
    .execute(&state.db)
    .await
    {
        error!("Failed to store confirmation token: {e}");
    } else if let Err(e) = state.mailer.send_confirmation_email(&email, &token).await {
        error!("Failed to send confirmation email: {e}");
    }

    // Create session
    let session_id = match create_session(&state.db, user_id).await {
        Ok(id) => id,
        Err(e) => {
            error!("Failed to create session: {e}");
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Registration failed");
        }
    };

    let mut response = Json(serde_json::json!({
        "id": user_id,
        "email": email,
        "email_confirmed": false,
        "is_approved": false,
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

// ── Login ───────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

async fn login(State(state): State<AppState>, Json(body): Json<LoginRequest>) -> Response {
    let email = body.email.trim().to_lowercase();

    let row: Option<(uuid::Uuid, String, bool, bool)> = match sqlx::query_as(
        "SELECT id, password_hash, email_confirmed, is_approved FROM users WHERE email = $1",
    )
    .bind(&email)
    .fetch_optional(&state.db)
    .await
    {
        Ok(row) => row,
        Err(e) => {
            error!("Database error during login: {e}");
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Login failed");
        }
    };

    let (user_id, password_hash, email_confirmed, is_approved) = match row {
        Some(r) => r,
        None => {
            return json_error(StatusCode::UNAUTHORIZED, "Invalid email or password");
        }
    };

    if !verify_password(&body.password, &password_hash) {
        return json_error(StatusCode::UNAUTHORIZED, "Invalid email or password");
    }

    info!(email = %email, "User logged in");

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

// ── Logout ──────────────────────────────────────────────────────────────────

async fn logout(State(state): State<AppState>, headers: axum::http::HeaderMap) -> Response {
    if let Some(cookie_header) = headers.get("cookie").and_then(|v| v.to_str().ok()) {
        if let Some(session_id) = extract_session_id_from_cookies(cookie_header) {
            let _ = delete_session(&state.db, session_id).await;
        }
    }

    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        "set-cookie",
        clear_session_cookie(is_secure()).parse().unwrap(),
    );
    response
}

// ── Me ──────────────────────────────────────────────────────────────────────

async fn me(user: Result<AuthUser, Response>) -> Response {
    match user {
        Ok(user) => Json(serde_json::json!({
            "id": user.id,
            "email": user.email,
            "email_confirmed": user.email_confirmed,
            "is_approved": user.is_approved,
        }))
        .into_response(),
        Err(e) => e,
    }
}

// ── Confirm Email ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ConfirmEmailParams {
    token: String,
}

async fn confirm_email(
    State(state): State<AppState>,
    Query(params): Query<ConfirmEmailParams>,
) -> Response {
    let row: Option<(uuid::Uuid,)> = match sqlx::query_as(
        "UPDATE email_tokens SET used = TRUE \
         WHERE id = $1 AND token_type = 'confirmation' AND expires_at > now() AND used = FALSE \
         RETURNING user_id",
    )
    .bind(&params.token)
    .fetch_optional(&state.db)
    .await
    {
        Ok(row) => row,
        Err(e) => {
            error!("Failed to validate confirmation token: {e}");
            return Redirect::temporary("/?error=confirmation_failed").into_response();
        }
    };

    match row {
        Some((user_id,)) => {
            if let Err(e) = sqlx::query(
                "UPDATE users SET email_confirmed = TRUE, updated_at = now() WHERE id = $1",
            )
            .bind(user_id)
            .execute(&state.db)
            .await
            {
                error!("Failed to confirm email: {e}");
                return Redirect::temporary("/?error=confirmation_failed").into_response();
            }
            info!(%user_id, "Email confirmed");
            Redirect::temporary("/?confirmed=true").into_response()
        }
        None => {
            warn!(token = %params.token, "Invalid or expired confirmation token");
            Redirect::temporary("/?error=invalid_token").into_response()
        }
    }
}

// ── Resend Confirmation ─────────────────────────────────────────────────────

async fn resend_confirmation(State(state): State<AppState>, user: AuthUser) -> Response {
    if user.email_confirmed {
        return json_error(StatusCode::BAD_REQUEST, "Email already confirmed");
    }

    let token = generate_token();
    let expires_at = Utc::now() + Duration::hours(24);

    if let Err(e) = sqlx::query(
        "INSERT INTO email_tokens (id, user_id, token_type, expires_at) VALUES ($1, $2, 'confirmation', $3)",
    )
    .bind(&token)
    .bind(user.id)
    .bind(expires_at)
    .execute(&state.db)
    .await
    {
        error!("Failed to store confirmation token: {e}");
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to send confirmation email");
    }

    if let Err(e) = state
        .mailer
        .send_confirmation_email(&user.email, &token)
        .await
    {
        error!("Failed to send confirmation email: {e}");
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to send confirmation email",
        );
    }

    Json(serde_json::json!({"message": "Confirmation email sent"})).into_response()
}

// ── Forgot Password ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ForgotPasswordRequest {
    email: String,
}

async fn forgot_password(
    State(state): State<AppState>,
    Json(body): Json<ForgotPasswordRequest>,
) -> Response {
    let email = body.email.trim().to_lowercase();

    // Always return 200 to prevent email enumeration
    let row: Option<(uuid::Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE email = $1")
        .bind(&email)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);

    if let Some((user_id,)) = row {
        let token = generate_token();
        let expires_at = Utc::now() + Duration::hours(1);

        if let Err(e) = sqlx::query(
            "INSERT INTO email_tokens (id, user_id, token_type, expires_at) VALUES ($1, $2, 'password_reset', $3)",
        )
        .bind(&token)
        .bind(user_id)
        .bind(expires_at)
        .execute(&state.db)
        .await
        {
            error!("Failed to store reset token: {e}");
        } else if let Err(e) = state.mailer.send_password_reset_email(&email, &token).await {
            error!("Failed to send password reset email: {e}");
        }
    }

    Json(serde_json::json!({"message": "If that email exists, a reset link has been sent"}))
        .into_response()
}

// ── Reset Password ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ResetPasswordRequest {
    token: String,
    password: String,
}

async fn reset_password(
    State(state): State<AppState>,
    Json(body): Json<ResetPasswordRequest>,
) -> Response {
    if body.password.len() < 8 {
        return json_error(
            StatusCode::BAD_REQUEST,
            "Password must be at least 8 characters",
        );
    }

    let row: Option<(uuid::Uuid,)> = match sqlx::query_as(
        "UPDATE email_tokens SET used = TRUE \
         WHERE id = $1 AND token_type = 'password_reset' AND expires_at > now() AND used = FALSE \
         RETURNING user_id",
    )
    .bind(&body.token)
    .fetch_optional(&state.db)
    .await
    {
        Ok(row) => row,
        Err(e) => {
            error!("Failed to validate reset token: {e}");
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Password reset failed");
        }
    };

    let user_id = match row {
        Some((id,)) => id,
        None => {
            return json_error(StatusCode::BAD_REQUEST, "Invalid or expired reset token");
        }
    };

    let password_hash = match hash_password(&body.password) {
        Ok(h) => h,
        Err(e) => {
            error!("Failed to hash password: {e}");
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Password reset failed");
        }
    };

    if let Err(e) =
        sqlx::query("UPDATE users SET password_hash = $1, updated_at = now() WHERE id = $2")
            .bind(&password_hash)
            .bind(user_id)
            .execute(&state.db)
            .await
    {
        error!("Failed to update password: {e}");
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "Password reset failed");
    }

    // Invalidate all existing sessions for this user
    let _ = sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(user_id)
        .execute(&state.db)
        .await;

    info!(%user_id, "Password reset");

    Json(serde_json::json!({"message": "Password has been reset"})).into_response()
}

// ── Password Hashing ────────────────────────────────────────────────────────

fn hash_password(password: &str) -> anyhow::Result<String> {
    use argon2::{
        password_hash::{rand_core::OsRng, SaltString},
        Argon2, PasswordHasher,
    };

    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("Password hashing failed: {e}"))?;

    Ok(hash.to_string())
}

fn verify_password(password: &str, hash: &str) -> bool {
    use argon2::{password_hash::PasswordHash, Argon2, PasswordVerifier};

    let parsed_hash = match PasswordHash::new(hash) {
        Ok(h) => h,
        Err(_) => return false,
    };

    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
}
