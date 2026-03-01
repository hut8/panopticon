use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpListener;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::sentinel::{is_valid_tag_id, process_scan};
use crate::ws::WsEvent;
use crate::AppState;

/// Maximum allowed line length from a sentinel (8 KiB).
const MAX_LINE_LENGTH: usize = 8192;

/// Bind to a configurable address and accept sentinel TCP connections.
pub async fn spawn_tcp_listener(state: AppState) {
    let addr = std::env::var("SENTINEL_TCP_ADDR").unwrap_or_else(|_| "0.0.0.0:8008".to_string());
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => {
            info!("Sentinel TCP listener on {addr}");
            l
        }
        Err(e) => {
            error!("Failed to bind sentinel TCP listener on {addr}: {e}");
            return;
        }
    };

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                info!(%addr, "Sentinel TCP connection");
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(state, stream, addr).await {
                        warn!(%addr, "Sentinel connection error: {e}");
                    }
                });
            }
            Err(e) => {
                error!("Failed to accept sentinel connection: {e}");
            }
        }
    }
}

async fn handle_connection(
    state: AppState,
    stream: tokio::net::TcpStream,
    addr: std::net::SocketAddr,
) -> anyhow::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();

    // 1. Expect AUTHZ as the first message (with 10-second timeout)
    line.clear();
    let n = match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        reader.read_line(&mut line),
    )
    .await
    {
        Ok(Ok(n)) => n,
        Ok(Err(e)) => anyhow::bail!("Read error during AUTHZ: {e}"),
        Err(_) => anyhow::bail!("Timed out waiting for AUTHZ"),
    };
    if n == 0 {
        anyhow::bail!("Connection closed before AUTHZ");
    }
    if line.len() > MAX_LINE_LENGTH {
        anyhow::bail!("AUTHZ line exceeds maximum length");
    }

    let trimmed = line.trim();
    let secret = trimmed
        .strip_prefix("AUTHZ: ")
        .ok_or_else(|| anyhow::anyhow!("Expected AUTHZ message, got: {trimmed}"))?;

    if secret != state.sentinel_secret {
        warn!(%addr, "Invalid sentinel secret");
        anyhow::bail!("Invalid secret");
    }

    // 2. Look up or create sentinel in DB (keyed by secret)
    let row: Option<(Uuid, String)> =
        sqlx::query_as("SELECT id, name FROM sentinels WHERE secret = $1")
            .bind(secret)
            .fetch_optional(&state.db)
            .await?;

    let (sentinel_id, sentinel_name) = match row {
        Some(r) => r,
        None => {
            let created: (Uuid, String) =
                sqlx::query_as("INSERT INTO sentinels (secret) VALUES ($1) RETURNING id, name")
                    .bind(secret)
                    .fetch_one(&state.db)
                    .await?;
            created
        }
    };

    // Mark connected
    sqlx::query("UPDATE sentinels SET connected = true, last_connected_at = now() WHERE id = $1")
        .bind(sentinel_id)
        .execute(&state.db)
        .await?;

    info!(%addr, sentinel_id = %sentinel_id, name = %sentinel_name, "Sentinel authenticated");

    let _ = state.events.send(WsEvent::SentinelConnected {
        id: sentinel_id,
        name: sentinel_name.clone(),
    });

    // 3. Read messages in a loop — use a closure-like pattern to guarantee cleanup
    let loop_result: anyhow::Result<()> = async {
        loop {
            line.clear();
            let n = match reader.read_line(&mut line).await {
                Ok(n) => n,
                Err(e) => {
                    warn!(%addr, sentinel_id = %sentinel_id, "Read error: {e}");
                    break;
                }
            };
            if n == 0 {
                break; // Connection closed
            }
            if line.len() > MAX_LINE_LENGTH {
                warn!(%addr, sentinel_id = %sentinel_id, "Line exceeds max length ({} bytes), dropping", line.len());
                continue;
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            if let Some(payload) = trimmed.strip_prefix("LOG: ") {
                // Insert log into DB — log errors instead of swallowing them
                match sqlx::query_as::<_, (Uuid, chrono::DateTime<chrono::Utc>)>(
                    "INSERT INTO sentinel_logs (sentinel_id, message) VALUES ($1, $2) RETURNING id, created_at",
                )
                .bind(sentinel_id)
                .bind(payload)
                .fetch_one(&state.db)
                .await
                {
                    Ok((_log_id, created_at)) => {
                        let _ = state.events.send(WsEvent::SentinelLog {
                            sentinel_id,
                            message: payload.to_string(),
                            created_at: created_at.to_rfc3339(),
                        });
                    }
                    Err(e) => {
                        error!(%addr, sentinel_id = %sentinel_id, "Failed to insert sentinel log: {e}");
                    }
                }
            } else if let Some(tag_id) = trimmed.strip_prefix("SCAN: ") {
                if !is_valid_tag_id(tag_id) {
                    warn!(%addr, tag_id, "Invalid tag_id format from sentinel");
                    continue;
                }

                match process_scan(&state, tag_id).await {
                    Ok(action) => {
                        info!(%addr, tag_id, action, "Scan processed via TCP");
                    }
                    Err(e) => {
                        error!(%addr, tag_id, "Failed to process scan: {e}");
                    }
                }
            } else {
                warn!(%addr, "Unknown message from sentinel: {trimmed}");
            }
        }
        Ok(())
    }
    .await;

    if let Err(e) = &loop_result {
        warn!(%addr, sentinel_id = %sentinel_id, "Message loop ended with error: {e}");
    }

    // 4. Disconnect cleanup (always runs after auth, regardless of how the loop exited)
    info!(%addr, sentinel_id = %sentinel_id, "Sentinel disconnected");

    if let Err(e) = sqlx::query("UPDATE sentinels SET connected = false WHERE id = $1")
        .bind(sentinel_id)
        .execute(&state.db)
        .await
    {
        error!(%addr, sentinel_id = %sentinel_id, "Failed to mark sentinel disconnected: {e}");
    }

    let _ = state
        .events
        .send(WsEvent::SentinelDisconnected { id: sentinel_id });

    Ok(())
}
