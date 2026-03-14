//! Persistent auth token storage.
//!
//! Stores the single U-Tec OAuth2 token to disk as JSON so it survives
//! restarts. Only one user is ever logged in.
//!
//! # Storage location
//!
//! Tries, in order:
//! 1. `/var/lib/panopticon/auth.json` — production (systemd creates this dir)
//! 2. `$XDG_DATA_HOME/panopticon/auth.json` — typically `~/.local/share/panopticon/`

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::oauth;
use crate::utec::UTec;

/// Persisted auth state.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AuthData {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// When the access token expires (if known).
    pub expires_at: Option<DateTime<Utc>>,
    /// U-Tec user ID for logging/display purposes.
    pub user_id: Option<String>,
    pub user_name: Option<String>,
    /// Token sent to U-Tec during webhook registration and echoed back
    /// on each notification for authentication.
    #[serde(default)]
    pub notification_token: Option<String>,
}

/// Thread-safe auth store backed by a JSON file.
#[derive(Clone)]
pub struct AuthStore {
    inner: Arc<RwLock<Option<AuthData>>>,
    path: PathBuf,
}

#[allow(dead_code)]
impl AuthStore {
    /// Create a new AuthStore, loading any existing auth data from disk.
    pub fn new() -> Result<Self> {
        let path = resolve_auth_path();
        info!(path = %path.display(), "Auth store location");

        let data = match std::fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str::<AuthData>(&contents) {
                Ok(data) => {
                    info!(
                        user = data.user_name.as_deref().unwrap_or("unknown"),
                        "Loaded existing auth token"
                    );
                    Some(data)
                }
                Err(e) => {
                    warn!(
                        "Failed to parse {}: {e} — starting without auth",
                        path.display()
                    );
                    None
                }
            },
            Err(_) => None,
        };

        Ok(Self {
            inner: Arc::new(RwLock::new(data)),
            path,
        })
    }

    /// Store new auth data and persist to disk.
    pub async fn save(&self, data: AuthData) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }

        let json = serde_json::to_string_pretty(&data)?;
        std::fs::write(&self.path, &json)
            .with_context(|| format!("Failed to write {}", self.path.display()))?;

        info!(
            path = %self.path.display(),
            user = data.user_name.as_deref().unwrap_or("unknown"),
            "Auth token saved"
        );

        *self.inner.write().await = Some(data);
        Ok(())
    }

    /// Get a UTec client if we have a valid token, or None.
    ///
    /// If the access token is expired but a refresh token is available,
    /// automatically attempts to refresh before returning None.
    pub async fn client(&self) -> Option<UTec> {
        // Fast path: check with read lock
        {
            let guard = self.inner.read().await;
            let data = guard.as_ref()?;
            if let Some(expires_at) = data.expires_at {
                if Utc::now() < expires_at {
                    return Some(UTec::new(data.access_token.clone()));
                }
                // Expired — fall through to refresh
            } else {
                // No expiry info — assume valid
                return Some(UTec::new(data.access_token.clone()));
            }
        }

        // Token expired — try to refresh
        self.try_refresh().await
    }

    /// Attempt to refresh an expired access token.
    ///
    /// Acquires the write lock first and double-checks expiry to ensure
    /// only one task performs the refresh (if U-Tec rotates refresh tokens,
    /// concurrent refreshes with the same old token would race).
    async fn try_refresh(&self) -> Option<UTec> {
        // Acquire write lock to serialize refresh attempts
        let mut guard = self.inner.write().await;
        let data = guard.as_mut()?;

        // Double-check: another task may have refreshed while we waited
        if let Some(expires_at) = data.expires_at {
            if Utc::now() < expires_at {
                return Some(UTec::new(data.access_token.clone()));
            }
        }

        let refresh_token = data.refresh_token.clone()?;
        warn!("Access token expired, attempting refresh");

        // Drop the lock during the network call to avoid blocking readers
        // for the duration of the HTTP request. We'll re-acquire after.
        drop(guard);

        match oauth::refresh_access_token(&refresh_token).await {
            Ok(token_response) => {
                // 30-second grace period (matches Python reference implementation).
                // Only apply when expires_in is large enough to avoid a refresh loop.
                let expires_at = token_response.expires_in.map(|secs| {
                    let grace = if secs > 120 { 30 } else { 0 };
                    Utc::now() + chrono::Duration::seconds((secs - grace) as i64)
                });

                let mut guard = self.inner.write().await;
                if let Some(data) = guard.as_mut() {
                    data.access_token = token_response.access_token.clone();
                    // Use new refresh_token if provided, otherwise keep the old one
                    if token_response.refresh_token.is_some() {
                        data.refresh_token = token_response.refresh_token;
                    }
                    data.expires_at = expires_at;

                    // Persist to disk
                    let data_clone = data.clone();
                    drop(guard);
                    if let Err(e) = self.save(data_clone).await {
                        error!("Failed to persist refreshed token: {e}");
                    }

                    // Re-read to get the saved data
                    let guard = self.inner.read().await;
                    let data = guard.as_ref()?;
                    Some(UTec::new(data.access_token.clone()))
                } else {
                    None
                }
            }
            Err(e) => {
                error!("Token refresh failed: {e:#}");
                None
            }
        }
    }

    /// Get the current auth data (if any).
    pub async fn get(&self) -> Option<AuthData> {
        self.inner.read().await.clone()
    }

    /// Get the notification token used to authenticate incoming webhooks.
    pub async fn notification_token(&self) -> Option<String> {
        self.inner
            .read()
            .await
            .as_ref()
            .and_then(|d| d.notification_token.clone())
    }

    /// Clear auth data (logout).
    pub async fn clear(&self) -> Result<()> {
        *self.inner.write().await = None;
        if self.path.exists() {
            std::fs::remove_file(&self.path)
                .with_context(|| format!("Failed to remove {}", self.path.display()))?;
            info!(path = %self.path.display(), "Auth token removed");
        }
        Ok(())
    }
}

/// Determine where to store auth.json.
///
/// 1. `/var/lib/panopticon/` if it exists and is writable
/// 2. `$XDG_DATA_HOME/panopticon/` (typically `~/.local/share/panopticon/`)
pub fn resolve_auth_path() -> PathBuf {
    let system_dir = Path::new("/var/lib/panopticon");
    if is_writable_dir(system_dir) {
        return system_dir.join("auth.json");
    }

    // Fall back to XDG data directory
    let xdg_dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("panopticon");

    xdg_dir.join("auth.json")
}

/// Check if a directory exists and is writable.
fn is_writable_dir(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    // Try creating a temp file to test writability
    let test = path.join(".write_test");
    match std::fs::write(&test, b"") {
        Ok(_) => {
            let _ = std::fs::remove_file(&test);
            true
        }
        Err(_) => false,
    }
}
