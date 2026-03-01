use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpListener;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::sentinel::{is_valid_tag_id, process_scan};
use crate::ws::WsEvent;
use crate::AppState;

/// Bind to port 8008 and accept sentinel TCP connections.
pub async fn spawn_tcp_listener(state: AppState) {
    let listener = match TcpListener::bind("0.0.0.0:8008").await {
        Ok(l) => {
            info!("Sentinel TCP listener on 0.0.0.0:8008");
            l
        }
        Err(e) => {
            error!("Failed to bind sentinel TCP listener: {e}");
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

    // 1. Expect AUTHZ as the first message
    line.clear();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        anyhow::bail!("Connection closed before AUTHZ");
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
            let created: (Uuid, String) = sqlx::query_as(
                "INSERT INTO sentinels (secret) VALUES ($1) RETURNING id, name",
            )
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

    // 3. Read messages in a loop
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break; // Connection closed
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(payload) = trimmed.strip_prefix("LOG: ") {
            // Insert log into DB
            let log_row: Option<(Uuid, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
                "INSERT INTO sentinel_logs (sentinel_id, message) VALUES ($1, $2) RETURNING id, created_at",
            )
            .bind(sentinel_id)
            .bind(payload)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();

            if let Some((_log_id, created_at)) = log_row {
                let _ = state.events.send(WsEvent::SentinelLog {
                    sentinel_id,
                    message: payload.to_string(),
                    created_at: created_at.to_rfc3339(),
                });
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

    // 4. Disconnect cleanup
    info!(%addr, sentinel_id = %sentinel_id, "Sentinel disconnected");

    sqlx::query("UPDATE sentinels SET connected = false WHERE id = $1")
        .bind(sentinel_id)
        .execute(&state.db)
        .await?;

    let _ = state.events.send(WsEvent::SentinelDisconnected { id: sentinel_id });

    Ok(())
}
