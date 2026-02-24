use anyhow::Result;
use chrono::{Duration, Utc};
use rand::Rng;
use sqlx::PgPool;

const SESSION_COOKIE: &str = "panopticon_session";
const SESSION_MAX_AGE_DAYS: i64 = 30;

pub fn generate_session_id() -> String {
    let bytes: [u8; 32] = rand::thread_rng().r#gen();
    hex::encode(&bytes)
}

pub async fn create_session(pool: &PgPool, user_id: uuid::Uuid) -> Result<String> {
    let session_id = generate_session_id();
    let expires_at = Utc::now() + Duration::days(SESSION_MAX_AGE_DAYS);

    sqlx::query("INSERT INTO sessions (id, user_id, expires_at) VALUES ($1, $2, $3)")
        .bind(&session_id)
        .bind(user_id)
        .bind(expires_at)
        .execute(pool)
        .await?;

    Ok(session_id)
}

pub async fn delete_session(pool: &PgPool, session_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM sessions WHERE id = $1")
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub fn set_session_cookie(session_id: &str, secure: bool) -> String {
    let max_age = SESSION_MAX_AGE_DAYS * 24 * 60 * 60;
    let secure_flag = if secure { "; Secure" } else { "" };
    format!(
        "{SESSION_COOKIE}={session_id}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age}{secure_flag}"
    )
}

pub fn clear_session_cookie(secure: bool) -> String {
    let secure_flag = if secure { "; Secure" } else { "" };
    format!("{SESSION_COOKIE}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0{secure_flag}")
}

pub fn extract_session_id_from_cookies(cookie_header: &str) -> Option<&str> {
    cookie_header
        .split(';')
        .map(|s| s.trim())
        .find(|s| s.starts_with(&format!("{SESSION_COOKIE}=")))
        .map(|s| &s[SESSION_COOKIE.len() + 1..])
}

/// Encode bytes as hex (avoids adding a hex crate dependency).
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}
