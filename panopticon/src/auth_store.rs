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
use tracing::{info, warn};

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
    pub async fn client(&self) -> Option<UTec> {
        let guard = self.inner.read().await;
        let data = guard.as_ref()?;

        // Check expiry if we know it
        if let Some(expires_at) = data.expires_at {
            if Utc::now() >= expires_at {
                warn!("Access token expired");
                return None;
            }
        }

        Some(UTec::new(data.access_token.clone()))
    }

    /// Get the current auth data (if any).
    pub async fn get(&self) -> Option<AuthData> {
        self.inner.read().await.clone()
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
fn resolve_auth_path() -> PathBuf {
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
