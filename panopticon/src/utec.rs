//! U-Tec API client.
//!
//! All U-Tec API requests go to a single endpoint (`POST https://api.u-tec.com/action`)
//! with a JSON body that specifies the action via `header.namespace` and `header.name`.
//! Authentication is via Bearer token in the Authorization header.
//!
//! The request/response envelope is always the same shape — only the `payload`
//! contents change per action.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tracing::{debug, error};
use uuid::Uuid;

const API_URL: &str = "https://api.u-tec.com/action";

/// Top-level request envelope sent to the U-Tec API.
#[derive(Serialize, Debug)]
struct ApiRequest<P: Serialize> {
    header: RequestHeader,
    payload: P,
}

/// Header included in every U-Tec API request.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct RequestHeader {
    namespace: String,
    name: String,
    message_id: String,
    payload_version: String,
}

/// Top-level response envelope from the U-Tec API.
#[derive(Deserialize, Debug)]
struct ApiResponse<P> {
    header: ResponseHeader,
    payload: P,
}

/// Header included in every U-Tec API response.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct ResponseHeader {
    namespace: String,
    name: String,
    message_id: String,
    payload_version: String,
}

/// Error payload returned by the U-Tec API.
#[derive(Deserialize, Debug)]
struct ErrorPayload {
    error: Option<ApiError>,
}

/// A U-Tec API error with code and message.
#[derive(Deserialize, Debug, Clone)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

// ── Public types for API responses ─────────────────────────────────────────

/// User info returned by `Uhome.User/Get`.
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: String,
    pub last_name: String,
    #[serde(alias = "FirstName")]
    pub first_name: String,
}

#[derive(Deserialize, Debug)]
struct UserPayload {
    user: User,
}

/// A U-Tec lock device.
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Lock {
    pub id: String,
    pub name: String,
    pub model: Option<String>,
}

#[derive(Deserialize, Debug)]
struct DeviceListPayload {
    devices: Vec<Lock>,
}

/// Lock status (locked/unlocked).
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LockStatus {
    pub lock_state: String,
}

#[derive(Deserialize, Debug)]
struct LockStatusPayload {
    status: LockStatus,
}

// ── Client ─────────────────────────────────────────────────────────────────

/// Client for the U-Tec smart lock API.
///
/// Holds the OAuth2 access token and provides typed methods for each API action.
#[derive(Clone)]
pub struct UTec {
    access_token: String,
    http: reqwest::Client,
}

impl UTec {
    /// Create a new client with the given access token.
    pub fn new(access_token: String) -> Self {
        Self {
            access_token,
            http: reqwest::Client::new(),
        }
    }

    /// Send a request to the U-Tec API and deserialize the response payload.
    ///
    /// All U-Tec API calls share the same URL and envelope structure — only
    /// `namespace`, `name`, and `payload` differ.
    async fn request<Req, Resp>(
        &self,
        namespace: &str,
        name: &str,
        payload: Req,
    ) -> Result<Resp>
    where
        Req: Serialize,
        Resp: DeserializeOwned,
    {
        let message_id = Uuid::new_v4().to_string();

        let body = ApiRequest {
            header: RequestHeader {
                namespace: namespace.to_string(),
                name: name.to_string(),
                message_id: message_id.clone(),
                payload_version: "1".to_string(),
            },
            payload,
        };

        debug!(
            namespace,
            name,
            message_id,
            "U-Tec API request"
        );

        let response = self
            .http
            .post(API_URL)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .json(&body)
            .send()
            .await
            .context("Failed to send request to U-Tec API")?;

        let status = response.status();
        let response_text = response
            .text()
            .await
            .context("Failed to read U-Tec API response")?;

        if !status.is_success() {
            error!(
                %status,
                body = %response_text,
                "U-Tec API HTTP error"
            );
            bail!("U-Tec API returned HTTP {status}: {response_text}");
        }

        // Try to parse as an error response first
        if let Ok(err_resp) = serde_json::from_str::<ApiResponse<ErrorPayload>>(&response_text) {
            if let Some(api_err) = err_resp.payload.error {
                error!(
                    code = %api_err.code,
                    message = %api_err.message,
                    "U-Tec API error"
                );
                return Err(api_err.into());
            }
        }

        // Parse the success response
        let api_resp: ApiResponse<Resp> = serde_json::from_str(&response_text)
            .with_context(|| format!("Failed to parse U-Tec API response: {response_text}"))?;

        debug!(
            message_id = %api_resp.header.message_id,
            "U-Tec API response OK"
        );

        Ok(api_resp.payload)
    }

    /// Get the authenticated user's info.
    pub async fn get_user(&self) -> Result<User> {
        #[derive(Serialize)]
        struct Empty {}

        let payload: UserPayload = self
            .request("Uhome.User", "Get", Empty {})
            .await?;

        Ok(payload.user)
    }

    /// List all locks associated with the user's account.
    pub async fn list_locks(&self) -> Result<Vec<Lock>> {
        #[derive(Serialize)]
        struct Empty {}

        let payload: DeviceListPayload = self
            .request("Uhome.Device", "List", Empty {})
            .await?;

        Ok(payload.devices)
    }

    /// Get the current status of a lock.
    pub async fn get_lock_status(&self, device_id: &str) -> Result<LockStatus> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Req {
            device_id: String,
        }

        let payload: LockStatusPayload = self
            .request(
                "Uhome.Device",
                "GetLockStatus",
                Req {
                    device_id: device_id.to_string(),
                },
            )
            .await?;

        Ok(payload.status)
    }

    /// Lock a device.
    pub async fn lock(&self, device_id: &str) -> Result<()> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Req {
            device_id: String,
        }

        #[derive(Deserialize)]
        struct Resp {}

        let _: Resp = self
            .request(
                "Uhome.Device",
                "Lock",
                Req {
                    device_id: device_id.to_string(),
                },
            )
            .await?;

        Ok(())
    }

    /// Unlock a device.
    pub async fn unlock(&self, device_id: &str) -> Result<()> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Req {
            device_id: String,
        }

        #[derive(Deserialize)]
        struct Resp {}

        let _: Resp = self
            .request(
                "Uhome.Device",
                "Unlock",
                Req {
                    device_id: device_id.to_string(),
                },
            )
            .await?;

        Ok(())
    }
}
