//! OAuth2 flow for U-Tec smart lock API.
//!
//! Flow:
//! 1. User visits /auth/login → redirected to U-Tec authorization endpoint
//! 2. U-Tec redirects back to /auth/callback with authorization_code + state
//! 3. We exchange the code for an access token via U-Tec's token endpoint
//! 4. Token is stored for subsequent API calls

use axum::{
    Router,
    extract::Query,
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use serde::Deserialize;
use tracing::{error, info};

/// U-Tec OAuth2 endpoints
const AUTHORIZE_URI: &str = "https://oauth.u-tec.com/authorize";
const TOKEN_URI: &str = "https://oauth.u-tec.com/token";

/// Callback host
const REDIRECT_HOST: &str = "https://hut8.tools";

// TODO: Move these to environment variables before deployment
const CLIENT_ID: &str = "YOUR_CLIENT_ID";
const CLIENT_SECRET: &str = "YOUR_CLIENT_SECRET";
const SCOPE: &str = "openapi";

pub fn router() -> Router {
    Router::new()
        .route("/login", get(login))
        .route("/callback", get(callback))
}

/// Redirect the user to U-Tec's OAuth2 authorization page.
async fn login() -> Response {
    // Generate a random state parameter for CSRF protection
    let state = generate_state();

    let redirect_uri = format!("{}/auth/callback", REDIRECT_HOST);

    let authorize_url = format!(
        "{}?response_type=code&client_id={}&client_secret={}&scope={}&redirect_uri={}&state={}",
        AUTHORIZE_URI,
        CLIENT_ID,
        CLIENT_SECRET,
        SCOPE,
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
/// U-Tec redirects here with `authorization_code` and `state` parameters.
/// We exchange the code for an access token.
async fn callback(Query(params): Query<CallbackParams>) -> Response {
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
    match exchange_code(&code).await {
        Ok(token_response) => {
            info!("Successfully obtained access token");
            // TODO: Store the token (in-memory, database, or cookie session)
            // For now, just confirm success
            (
                axum::http::StatusCode::OK,
                format!("Authentication successful. Token type: {}", token_response.token_type),
            )
                .into_response()
        }
        Err(e) => {
            error!("Failed to exchange authorization code: {e}");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Token exchange failed: {e}"),
            )
                .into_response()
        }
    }
}

#[derive(Deserialize, Debug)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    expires_in: Option<u64>,
    refresh_token: Option<String>,
}

/// Exchange an authorization code for an access token.
async fn exchange_code(code: &str) -> anyhow::Result<TokenResponse> {
    let url = format!(
        "{}?grant_type=authorization_code&client_id={}&code={}",
        TOKEN_URI, CLIENT_ID, code,
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
