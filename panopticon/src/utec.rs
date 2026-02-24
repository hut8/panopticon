//! U-Tec API client.
//!
//! All U-Tec API requests go to a single endpoint (`POST https://api.u-tec.com/action`)
//! with a JSON body that specifies the action via `header.namespace` and `header.name`.
//! Authentication is via Bearer token in the Authorization header.
//!
//! The request/response envelope is always the same shape — only the `payload`
//! contents change per action.
//!
//! # Actions
//!
//! | Namespace | Name | Description |
//! |-----------|------|-------------|
//! | `Uhome.Configure` | `Set` | Register notification webhook URL |
//! | `Uhome.User` | `Get` | Get current user info |
//! | `Uhome.User` | `Logout` | Log out current user |
//! | `Uhome.Device` | `Discovery` | List all devices and their capabilities |
//! | `Uhome.Device` | `Query` | Query real-time device states |
//! | `Uhome.Device` | `Command` | Send a command to devices |

use anyhow::{bail, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tracing::{debug, error};
use uuid::Uuid;

const API_URL: &str = "https://api.u-tec.com/action";

// ── Envelope types ─────────────────────────────────────────────────────────

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
    #[allow(dead_code)]
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
/// Errors come back inside a 200 OK response, in `payload.error`.
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

// ── Uhome.User types ───────────────────────────────────────────────────────

/// User info returned by `Uhome.User/Get`.
#[derive(Deserialize, Debug, Clone)]
pub struct User {
    pub id: String,
    pub last_name: String,
    pub first_name: String,
}

#[derive(Deserialize, Debug)]
struct UserPayload {
    user: User,
}

// ── Uhome.Device types ─────────────────────────────────────────────────────

/// A device returned by `Uhome.Device/Discovery`.
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    /// Device type: "LOCK", "LIGHT", etc.
    pub category: Option<String>,
    /// Predefined function set: "utec-lock", "utec-bulb-color-rgbw", etc.
    pub handle_type: Option<String>,
    pub device_info: Option<DeviceInfo>,
    /// Opaque data that must be echoed back in Query/Command requests.
    pub custom_data: Option<serde_json::Value>,
    /// Device-specific configuration (e.g., color model for lights).
    pub attributes: Option<serde_json::Value>,
}

impl Device {
    pub fn is_lock(&self) -> bool {
        matches!(self.category.as_deref(), Some("LOCK" | "SmartLock"))
    }
}

/// Basic device info from discovery.
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub hw_version: Option<String>,
}

/// A capability state returned by `Uhome.Device/Query` and `Uhome.Device/Command`.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct DeviceState {
    pub capability: String,
    pub name: String,
    pub value: serde_json::Value,
}

/// A device with its current states, as returned by Query/Command responses.
#[derive(Deserialize, Debug, Clone)]
pub struct DeviceWithStates {
    pub id: String,
    pub states: Vec<DeviceState>,
}

impl DeviceWithStates {
    /// Find a state by capability and name.
    pub fn get_state(&self, capability: &str, name: &str) -> Option<&DeviceState> {
        self.states
            .iter()
            .find(|s| s.capability == capability && s.name == name)
    }

    /// Check if the device is online (has `st.healthCheck/status == "Online"`).
    pub fn is_online(&self) -> bool {
        self.get_state("st.healthCheck", "status")
            .and_then(|s| s.value.as_str())
            .map(|s| s.eq_ignore_ascii_case("online"))
            .unwrap_or(false)
    }

    /// Get the lock state if this is a lock device.
    /// Returns "locked" or "unlocked" (normalized to lowercase).
    pub fn lock_state(&self) -> Option<String> {
        self.get_state("st.lock", "lockState")
            .and_then(|s| s.value.as_str())
            .map(|s| s.to_lowercase())
    }

    /// Get the battery level if available.
    pub fn battery_level(&self) -> Option<u64> {
        self.get_state("st.batteryLevel", "level")
            .and_then(|s| s.value.as_u64())
    }
}

// ── Payload types for requests/responses ───────────────────────────────────

#[derive(Serialize, Debug)]
struct EmptyPayload {}

#[derive(Deserialize, Debug)]
struct DiscoveryPayload {
    devices: Vec<Device>,
}

/// A device reference for Query/Command requests.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct DeviceRef {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    custom_data: Option<serde_json::Value>,
}

#[derive(Serialize, Debug)]
struct QueryPayload {
    devices: Vec<DeviceRef>,
}

/// A device command for Command requests.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct DeviceCommand {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    custom_data: Option<serde_json::Value>,
    command: CommandSpec,
}

/// The command specification within a Command request.
#[derive(Serialize, Debug)]
pub struct CommandSpec {
    pub capability: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<serde_json::Value>,
}

#[derive(Serialize, Debug)]
struct CommandPayload {
    devices: Vec<DeviceCommand>,
}

#[derive(Deserialize, Debug)]
struct DevicesResponsePayload {
    devices: Vec<DeviceWithStates>,
}

/// Notification webhook configuration for `Uhome.Configure/Set`.
#[derive(Serialize, Debug)]
struct ConfigurePayload {
    configure: ConfigureNotification,
}

#[derive(Serialize, Debug)]
struct ConfigureNotification {
    notification: NotificationConfig,
}

#[derive(Serialize, Debug)]
struct NotificationConfig {
    access_token: String,
    url: String,
}

// ── Client ─────────────────────────────────────────────────────────────────

/// Client for the U-Tec smart lock API.
///
/// Holds the OAuth2 access token and provides typed methods for each API action.
/// All methods go through a single generic `request()` that handles the envelope
/// format, UUID message IDs, and error detection.
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
    async fn request<Req, Resp>(&self, namespace: &str, name: &str, payload: Req) -> Result<Resp>
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

        let request_json = serde_json::to_string(&body).context("Failed to serialize request")?;
        debug!(namespace, name, message_id, body = %request_json, "U-Tec API request");

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

        debug!(%status, body = %response_text, "U-Tec API response");

        if !status.is_success() {
            error!(%status, body = %response_text, "U-Tec API HTTP error");
            bail!("U-Tec API returned HTTP {status}: {response_text}");
        }

        // Try to parse as an error response first — U-Tec returns errors
        // inside 200 OK responses in payload.error
        if let Ok(err_resp) = serde_json::from_str::<ApiResponse<ErrorPayload>>(&response_text) {
            if let Some(api_err) = err_resp.payload.error {
                error!(code = %api_err.code, message = %api_err.message, body = %response_text, "U-Tec API error");
                return Err(api_err.into());
            }
        }

        // Parse the success response
        let api_resp: ApiResponse<Resp> = serde_json::from_str(&response_text)
            .with_context(|| format!("Failed to parse U-Tec API response: {response_text}"))?;

        debug!(message_id = %api_resp.header.message_id, "U-Tec API response OK");
        Ok(api_resp.payload)
    }

    // ── Uhome.Configure ────────────────────────────────────────────────────

    /// Register a webhook URL for device event notifications.
    ///
    /// The `notification_token` is sent back by U-Tec with each notification
    /// for authentication. Update it periodically for security.
    pub async fn set_notification_url(&self, url: &str, notification_token: &str) -> Result<()> {
        let payload = ConfigurePayload {
            configure: ConfigureNotification {
                notification: NotificationConfig {
                    access_token: notification_token.to_string(),
                    url: url.to_string(),
                },
            },
        };

        let _: serde_json::Value = self.request("Uhome.Configure", "Set", payload).await?;
        Ok(())
    }

    // ── Uhome.User ─────────────────────────────────────────────────────────

    /// Get the authenticated user's info.
    pub async fn get_user(&self) -> Result<User> {
        let payload: UserPayload = self.request("Uhome.User", "Get", EmptyPayload {}).await?;
        Ok(payload.user)
    }

    /// Log out the current user (invalidates the access token).
    pub async fn logout(&self) -> Result<()> {
        let _: serde_json::Value = self
            .request("Uhome.User", "Logout", EmptyPayload {})
            .await?;
        Ok(())
    }

    // ── Uhome.Device ───────────────────────────────────────────────────────

    /// Discover all devices associated with the user's account.
    ///
    /// Returns the full device list with capabilities, device info, and
    /// custom data that must be echoed back in Query/Command requests.
    pub async fn discover_devices(&self) -> Result<Vec<Device>> {
        let payload: DiscoveryPayload = self
            .request("Uhome.Device", "Discovery", EmptyPayload {})
            .await?;
        Ok(payload.devices)
    }

    /// Query the real-time state of one or more devices.
    ///
    /// Returns states like lock status, battery level, and online/offline.
    /// Include `custom_data` from the Discovery response if the device had any.
    pub async fn query_devices(&self, devices: &[&Device]) -> Result<Vec<DeviceWithStates>> {
        let refs: Vec<DeviceRef> = devices
            .iter()
            .map(|d| DeviceRef {
                id: d.id.clone(),
                custom_data: d.custom_data.clone(),
            })
            .collect();

        let payload: DevicesResponsePayload = self
            .request("Uhome.Device", "Query", QueryPayload { devices: refs })
            .await?;
        Ok(payload.devices)
    }

    /// Send a command to a device.
    ///
    /// The response may include a `st.deferredResponse` state indicating the
    /// command is being executed asynchronously (e.g., "seconds": 10 means
    /// the lock will respond within 10 seconds).
    pub async fn send_command(
        &self,
        device: &Device,
        command: CommandSpec,
    ) -> Result<Vec<DeviceWithStates>> {
        let payload: DevicesResponsePayload = self
            .request(
                "Uhome.Device",
                "Command",
                CommandPayload {
                    devices: vec![DeviceCommand {
                        id: device.id.clone(),
                        custom_data: device.custom_data.clone(),
                        command,
                    }],
                },
            )
            .await?;
        Ok(payload.devices)
    }

    // ── Convenience methods for locks ──────────────────────────────────────

    /// Discover only lock devices.
    pub async fn discover_locks(&self) -> Result<Vec<Device>> {
        let all = self.discover_devices().await?;
        Ok(all.into_iter().filter(|d| d.is_lock()).collect())
    }

    /// Query the state of a single device. Returns its states.
    pub async fn query_device(&self, device: &Device) -> Result<DeviceWithStates> {
        let mut results = self.query_devices(&[device]).await?;
        results.pop().context("No device state returned")
    }

    /// Lock a device by sending the `st.lock/lock` command.
    pub async fn lock(&self, device: &Device) -> Result<Vec<DeviceWithStates>> {
        self.send_command(
            device,
            CommandSpec {
                capability: "st.lock".to_string(),
                name: "lock".to_string(),
                arguments: None,
            },
        )
        .await
    }

    /// Unlock a device by sending the `st.lock/unlock` command.
    pub async fn unlock(&self, device: &Device) -> Result<Vec<DeviceWithStates>> {
        self.send_command(
            device,
            CommandSpec {
                capability: "st.lock".to_string(),
                name: "unlock".to_string(),
                arguments: None,
            },
        )
        .await
    }
}
