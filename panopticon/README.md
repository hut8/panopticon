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

### Request format

```json
{
    "header": {
        "namespace": "Uhome.Device",
        "name": "Lock",
        "messageId": "d290f1ee-6c54-4b01-90e6-d701748f0851",
        "payloadVersion": "1"
    },
    "payload": {
        "deviceId": "..."
    }
}
```

`messageId` is a UUID v4. The response echoes it back.

### Response format

```json
{
    "header": {
        "namespace": "Uhome.Device",
        "name": "Lock",
        "messageId": "d290f1ee-6c54-4b01-90e6-d701748f0851",
        "payloadVersion": "1"
    },
    "payload": { ... }
}
```

### Error format

Errors are returned inside `payload.error` (not as HTTP error codes):

```json
{
    "header": { ... },
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

### Implemented actions

| Namespace | Name | Description | Rust method |
|-----------|------|-------------|-------------|
| `Uhome.User` | `Get` | Get authenticated user info | `utec.get_user()` |
| `Uhome.Device` | `List` | List all locks | `utec.list_locks()` |
| `Uhome.Device` | `GetLockStatus` | Get lock/unlock state | `utec.get_lock_status(id)` |
| `Uhome.Device` | `Lock` | Lock a device | `utec.lock(id)` |
| `Uhome.Device` | `Unlock` | Unlock a device | `utec.unlock(id)` |

### Usage (Rust)

```rust
use panopticon::utec::UTec;

let client = UTec::new(access_token);
let user = client.get_user().await?;
let locks = client.list_locks().await?;
client.unlock(&locks[0].id).await?;
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
