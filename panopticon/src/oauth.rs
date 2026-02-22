//! OAuth2 flow for U-Tec smart lock API.
//!
//! Flow:
//! 1. User visits /auth/login → redirected to U-Tec authorization endpoint
//! 2. U-Tec redirects back to /auth/callback with authorization_code + state
//! 3. We exchange the code for an access token via U-Tec's token endpoint
//! 4. Token is persisted to auth.json and a UTec client is created

use std::sync::LazyLock;

use axum::{
    extract::{Query, State},
    response::{IntoResponse, Redirect, Response},
    routing::{delete, get},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use crate::auth_store::{AuthData, AuthStore};
use crate::utec::UTec;

/// U-Tec OAuth2 endpoints
const AUTHORIZE_URI: &str = "https://oauth.u-tec.com/authorize";
const TOKEN_URI: &str = "https://oauth.u-tec.com/token";

/// Callback host
const REDIRECT_HOST: &str = "https://hut8.tools";

/// OAuth2 credentials loaded from environment variables.
/// Required: UTEC_CLIENT_ID, UTEC_CLIENT_SECRET
/// Optional: UTEC_SCOPE (defaults to "openapi")
static CLIENT_ID: LazyLock<String> =
    LazyLock::new(|| std::env::var("UTEC_CLIENT_ID").expect("UTEC_CLIENT_ID must be set"));
static CLIENT_SECRET: LazyLock<String> = LazyLock::new(|| {
    std::env::var("UTEC_CLIENT_SECRET").expect("UTEC_CLIENT_SECRET must be set")
});
static SCOPE: LazyLock<String> =
    LazyLock::new(|| std::env::var("UTEC_SCOPE").unwrap_or_else(|_| "openapi".to_string()));

pub fn router(auth_store: AuthStore) -> Router {
    Router::new()
        .route("/login", get(login))
        .route("/callback", get(callback))
        .route("/status", get(status))
        .route("/logout", delete(logout))
        .with_state(auth_store)
}

/// Redirect the user to U-Tec's OAuth2 authorization page.
async fn login() -> Response {
    let state = generate_state();
    let redirect_uri = format!("{}/auth/callback", REDIRECT_HOST);

    let authorize_url = format!(
        "{}?response_type=code&client_id={}&client_secret={}&scope={}&redirect_uri={}&state={}",
        AUTHORIZE_URI,
        &*CLIENT_ID,
        &*CLIENT_SECRET,
        &*SCOPE,
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(&state),
    );

    info!("Redirecting to U-Tec OAuth2 authorization");
    Redirect::temporary(&authorize_url).into_response()
}

#[derive(Deserialize)]
struct CallbackParams {
    authorization_code: Option<String>,
    code: Option<String>,
    state: Option<String>,
}

/// Handle the OAuth2 callback from U-Tec.
///
/// Exchanges the authorization code for an access token, verifies by fetching
/// user info, then persists the token to disk.
async fn callback(
    State(auth_store): State<AuthStore>,
    Query(params): Query<CallbackParams>,
) -> Response {
    // U-Tec uses `authorization_code` as the parameter name per their docs,
    // but fall back to standard `code` just in case
    let code = match params.authorization_code.or(params.code) {
        Some(c) => c,
        None => {
            error!("OAuth callback missing authorization code");
            return (
                axum::http::StatusCode::BAD_REQUEST,
                "Missing authorization code",
            )
                .into_response();
        }
    };

    if let Some(state) = &params.state {
        info!("OAuth callback received with state: {}", state);
        // TODO: Validate state matches what we sent (CSRF protection)
    }

    // Exchange authorization code for access token
    let token_response = match exchange_code(&code).await {
        Ok(t) => t,
        Err(e) => {
            error!("Failed to exchange authorization code: {e}");
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Token exchange failed: {e}"),
            )
                .into_response();
        }
    };

    info!("Successfully obtained access token");

    // Calculate expiry time
    let expires_at = token_response
        .expires_in
        .map(|secs| Utc::now() + chrono::Duration::seconds(secs as i64));

    // Verify the token works by fetching user info
    let client = UTec::new(token_response.access_token.clone());
    let (user_id, user_name) = match client.get_user().await {
        Ok(user) => {
            let name = format!("{} {}", user.first_name, user.last_name);
            info!(user_id = %user.id, name = %name, "Authenticated U-Tec user");
            (Some(user.id), Some(name))
        }
        Err(e) => {
            error!("Token valid but failed to fetch user info: {e}");
            (None, None)
        }
    };

    // Persist to disk
    let auth_data = AuthData {
        access_token: token_response.access_token,
        refresh_token: token_response.refresh_token,
        expires_at,
        user_id,
        user_name: user_name.clone(),
    };

    if let Err(e) = auth_store.save(auth_data).await {
        error!("Failed to save auth token: {e}");
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Authentication succeeded but failed to save token: {e}"),
        )
            .into_response();
    }

    // Redirect back to the frontend
    Redirect::temporary("/").into_response()
}

/// Auth status response for the frontend.
#[derive(Serialize)]
struct AuthStatus {
    authenticated: bool,
    user_name: Option<String>,
    expires_at: Option<String>,
}

/// Check whether we have a valid cached token.
async fn status(State(auth_store): State<AuthStore>) -> Json<AuthStatus> {
    match auth_store.get().await {
        Some(data) => {
            let expired = data
                .expires_at
                .map(|exp| Utc::now() >= exp)
                .unwrap_or(false);
            Json(AuthStatus {
                authenticated: !expired,
                user_name: data.user_name,
                expires_at: data.expires_at.map(|t| t.to_rfc3339()),
            })
        }
        None => Json(AuthStatus {
            authenticated: false,
            user_name: None,
            expires_at: None,
        }),
    }
}

/// Clear cached credentials.
async fn logout(State(auth_store): State<AuthStore>) -> Response {
    match auth_store.clear().await {
        Ok(_) => axum::http::StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            error!("Failed to clear auth: {e}");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to logout: {e}"),
            )
                .into_response()
        }
    }
}

#[derive(Deserialize, Debug)]
struct TokenResponse {
    access_token: String,
    #[allow(dead_code)]
    token_type: String,
    expires_in: Option<u64>,
    refresh_token: Option<String>,
}

/// Exchange an authorization code for an access token.
async fn exchange_code(code: &str) -> anyhow::Result<TokenResponse> {
    let url = format!(
        "{}?grant_type=authorization_code&client_id={}&code={}",
        TOKEN_URI, &*CLIENT_ID, code,
    );

    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Token endpoint returned {status}: {body}");
    }

    let token: TokenResponse = response.json().await?;
    Ok(token)
}

/// Generate a random state parameter for CSRF protection.
fn generate_state() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{:x}", nonce)
}
