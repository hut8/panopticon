# Panopticon

Web server for RFID door access control. Manages U-Tec smart locks via their OAuth2 API, served through an axum backend with an embedded Svelte 5 SPA.

## Stack

- **Backend:** Rust / axum, port 1337
- **Frontend:** Svelte 5, SvelteKit, Tailwind CSS 4, Skeleton UI v4.12
- **Deployment:** Single binary with embedded frontend (via `include_dir`)
- **Reverse proxy:** Caddy (`hut8.tools` → `localhost:1337`)

## U-Tec API

All U-Tec API calls go to a single endpoint with a uniform request/response envelope. The action is specified by `namespace` and `name` in the header.

### Endpoint

```
POST https://api.u-tec.com/action
Authorization: Bearer <ACCESS_TOKEN>
Content-Type: application/json
```

### Envelope format

Every request and response uses the same envelope. `messageId` is a UUID v4 echoed back in the response.

```json
{
    "header": {
        "namespace": "Uhome.Device",
        "name": "Discovery",
        "messageId": "d290f1ee-6c54-4b01-90e6-d701748f0851",
        "payloadVersion": "1"
    },
    "payload": { ... }
}
```

Errors are returned inside a 200 OK response in `payload.error`:

```json
{
    "payload": {
        "error": {
            "code": "INVALID_TOKEN",
            "message": "Token is invalid, expired, or malformed."
        }
    }
}
```

### OAuth2 flow

| Step | URL |
|------|-----|
| **Authorize** | `https://oauth.u-tec.com/authorize?response_type=code&client_id={ID}&client_secret={SECRET}&scope={SCOPE}&redirect_uri={URI}&state={STATE}` |
| **Callback** | `https://hut8.tools/auth/callback?authorization_code={CODE}&state={STATE}` |
| **Token** | `POST https://oauth.u-tec.com/token?grant_type=authorization_code&client_id={ID}&code={CODE}` |

### Actions

| Namespace | Name | Description | Rust method |
|-----------|------|-------------|-------------|
| `Uhome.Configure` | `Set` | Register notification webhook URL | `utec.set_notification_url(url, token)` |
| `Uhome.User` | `Get` | Get current user info | `utec.get_user()` |
| `Uhome.User` | `Logout` | Invalidate access token | `utec.logout()` |
| `Uhome.Device` | `Discovery` | List all devices with capabilities | `utec.discover_devices()` |
| `Uhome.Device` | `Query` | Query real-time device states | `utec.query_devices(&[device])` |
| `Uhome.Device` | `Command` | Send command to a device | `utec.send_command(device, cmd)` |

### Device state model

Devices report state via capability-based key-value entries:

| Capability | Name | Value | Description |
|------------|------|-------|-------------|
| `st.healthCheck` | `status` | `"online"` / `"offline"` | Device connectivity |
| `st.Lock` | `lockState` | `"locked"` / `"unlocked"` | Lock state |
| `st.BatteryLevel` | `level` | `0`-`100` | Battery percentage |
| `st.deferredResponse` | `seconds` | `10` | Command is async, wait N seconds |

### Device discovery

Discovery returns devices with `category` (e.g., `"LOCK"`, `"LIGHT"`), `handleType` (e.g., `"utec-lock"`), `deviceInfo` (manufacturer, model, firmware), and `customData` that must be echoed back in Query/Command requests.

### Usage (Rust)

```rust
use panopticon::utec::{UTec, CommandSpec};

let client = UTec::new(access_token);

// Get user info
let user = client.get_user().await?;

// Discover all locks
let locks = client.discover_locks().await?;
let lock = &locks[0];

// Query lock state
let state = client.query_device(lock).await?;
println!("Lock state: {:?}", state.lock_state());     // "locked" / "unlocked"
println!("Battery: {:?}", state.battery_level());       // Some(85)
println!("Online: {}", state.is_online());              // true

// Unlock
client.unlock(lock).await?;

// Send arbitrary command
client.send_command(lock, CommandSpec {
    capability: "st.Lock".to_string(),
    name: "lock".to_string(),
    arguments: None,
}).await?;
```

## Development

```bash
# Terminal 1: Rust backend
cargo run

# Terminal 2: Vite dev server (hot-reloading frontend)
cd web && npm install && npm run dev
```

Vite (port 5173) proxies `/auth/*` to the Rust backend (port 1337).

## Deployment

```bash
./deploy
```

Builds release binary (including Svelte app), installs to `/usr/local/bin/panopticon`, and restarts the systemd service.

### Caddy

```bash
sudo cp Caddyfile /etc/caddy/Caddyfile
sudo systemctl reload caddy
```

## Known bugs

### 1. U-Tec webhook notifications never arrive

Registering a notification webhook via `Uhome.Configure/Set` returns a successful response, but U-Tec never calls the registered URL when device state changes (e.g., lock/unlock from the Google Home app or physically). No incoming webhook requests appear in server logs at all.

### 2. Missing `DoorSensor` capability for `utec-lock-sensor` devices

The [U-Tec documentation](https://doc.api.u-tec.com/#774fb309-95c0-4a3a-a03b-5448f0172bc4) indicates that devices with `handleType: "utec-lock-sensor"` should report a `DoorSensor` capability (door open/closed state). However, the API does not return this capability even though the device identifies as `utec-lock-sensor`.

Discovery response shows `handleType: "utec-lock-sensor"`:
```json
{
  "devices": [{
    "id": "AA:BB:CC:DD:EE:FF",
    "name": "Front Door",
    "category": "SmartLock",
    "handleType": "utec-lock-sensor",
    "deviceInfo": { "manufacturer": "U-tec", "model": "Bolt-NFC-W", "hwVersion": "01.50.0036" },
    "attributes": { "batteryLevelRange": { "min": 1, "max": 5, "step": 1 } }
  }]
}
```

But the Query response only returns `st.healthCheck`, `st.lock`, and `st.batteryLevel` — no `DoorSensor`:
```json
{
  "devices": [{
    "id": "AA:BB:CC:DD:EE:FF",
    "states": [
      { "capability": "st.healthCheck", "name": "status", "value": "Online" },
      { "capability": "st.lock", "name": "lockState", "value": "Unlocked" },
      { "capability": "st.lock", "name": "lockMode", "value": 0 },
      { "capability": "st.batteryLevel", "name": "level", "value": 5 }
    ]
  }]
}
```

### 3. `st.doorSensor` documentation describes the wrong capability

The U-Tec docs describe `st.doorSensor` as "An indication of the status of the battery", which is clearly a copy-paste error from `st.batteryLevel`. This is the capability we need for door open/closed state from `utec-lock-sensor` devices (see bug #2), but the incorrect documentation makes it unclear what the actual payload looks like.

### 4. `st.deferredResponse` — no async callback, wrong capability name

The [documentation](https://doc.api.u-tec.com/#9adec248-fae5-4265-a432-eafaf952b0b7) says: "Typically, if a device operates with an asynchronous response mechanism, we first send a synchronous message to notify that a response may require a certain amount of waiting time. Then, once the operation is successfully completed on the device, an asynchronous response is sent back."

In practice, no asynchronous response is ever sent back. You must make a second request (Query) after the indicated delay to get the actual device state. This is actually described in the attribute documentation itself: "The approximate time before you send your second response, in seconds" — contradicting the claim of an async callback.

Additionally, the capability is documented as `st.DeferredResponse` but the API actually returns `st.deferredResponse` (lowercase `d`).

### 5. `st.lockUser` type values are wrong in the documentation

The [documentation](https://doc.api.u-tec.com/#4f860cbb-5470-41f2-85e3-b868e168db83) defines user types as `0: Normal User, 2: Temporary User, 3: Admin`. The actual API returns different values:

| Documented | Actual |
|------------|--------|
| `0` = Normal User | Not observed |
| `2` = Temporary User | Not observed |
| `3` = Admin | `3` = Normal User |
| (not documented) | `1` = Admin |

Observed response:
```json
{
  "users": [
    { "id": 1000000001, "name": "Alice", "type": 1, "status": 1, "sync_status": 1 },
    { "id": 1000000002, "name": "Bob", "type": 3, "status": 1, "sync_status": 1 }
  ]
}
```

Here `type: 1` is the account owner (admin) and `type: 3` is a normal user — the opposite of what the docs say.

### 6. Developer support

The correct URL for xthings developer support requests is: https://developer.xthings.com/hc/en-us/requests/new
