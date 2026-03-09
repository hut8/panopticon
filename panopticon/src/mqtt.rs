use std::time::Duration;

use rumqttc::{AsyncClient, Event, Incoming, LastWill, MqttOptions, Publish, QoS};
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

use crate::ws::WsEvent;
use crate::AppState;

// ── Config ──────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct MqttConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub client_id: String,
    pub discovery_prefix: String,
}

impl MqttConfig {
    pub fn from_env() -> Option<Self> {
        let host = match std::env::var("MQTT_HOST") {
            Ok(h) => h,
            Err(_) => {
                info!("MQTT_HOST not set, MQTT bridge disabled");
                return None;
            }
        };

        let port = std::env::var("MQTT_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(1883);

        let username = std::env::var("MQTT_USERNAME").ok();
        let password = std::env::var("MQTT_PASSWORD").ok();
        let client_id = std::env::var("MQTT_CLIENT_ID").unwrap_or_else(|_| "panopticon".into());
        let discovery_prefix =
            std::env::var("MQTT_DISCOVERY_PREFIX").unwrap_or_else(|_| "homeassistant".into());

        info!(host, port, client_id, "MQTT bridge enabled");

        Some(Self {
            host,
            port,
            username,
            password,
            client_id,
            discovery_prefix,
        })
    }
}

// ── Topic helpers ───────────────────────────────────────────────────────────

const BASE: &str = "panopticon";

fn lock_state_topic(device_id: &str) -> String {
    format!("{BASE}/lock/{device_id}/state")
}

fn battery_topic(device_id: &str) -> String {
    format!("{BASE}/lock/{device_id}/battery")
}

fn lock_availability_topic(device_id: &str) -> String {
    format!("{BASE}/lock/{device_id}/availability")
}

fn lock_command_topic(device_id: &str) -> String {
    format!("{BASE}/lock/{device_id}/set")
}

fn scan_topic() -> String {
    format!("{BASE}/scan/last")
}

fn sentinel_connected_topic(id: &uuid::Uuid) -> String {
    format!("{BASE}/sentinel/{id}/connected")
}

fn mode_state_topic() -> String {
    format!("{BASE}/sentinel/mode/state")
}

fn mode_command_topic() -> String {
    format!("{BASE}/sentinel/mode/set")
}

fn bridge_state_topic() -> String {
    format!("{BASE}/bridge/state")
}

fn discovery_topic(config: &MqttConfig, component: &str, object_id: &str) -> String {
    let sanitized = object_id.replace(':', "_");
    format!(
        "{}/{component}/panopticon/{sanitized}/config",
        config.discovery_prefix
    )
}

// ── Discovery payload builders ──────────────────────────────────────────────

fn lock_discovery(config: &MqttConfig, device_id: &str, device_name: &str) -> serde_json::Value {
    json!({
        "name": device_name,
        "unique_id": format!("panopticon_lock_{device_id}"),
        "command_topic": lock_command_topic(device_id),
        "state_topic": lock_state_topic(device_id),
        "availability": [
            { "topic": bridge_state_topic() }
        ],
        "payload_lock": "LOCK",
        "payload_unlock": "UNLOCK",
        "state_locked": "LOCKED",
        "state_unlocked": "UNLOCKED",
        "device": device_obj(config, device_id, device_name),
    })
}

fn battery_discovery(config: &MqttConfig, device_id: &str, device_name: &str) -> serde_json::Value {
    json!({
        "name": format!("{device_name} Battery"),
        "unique_id": format!("panopticon_battery_{device_id}"),
        "state_topic": battery_topic(device_id),
        "device_class": "battery",
        "unit_of_measurement": "%",
        "availability": [
            { "topic": bridge_state_topic() }
        ],
        "device": device_obj(config, device_id, device_name),
    })
}

fn online_discovery(config: &MqttConfig, device_id: &str, device_name: &str) -> serde_json::Value {
    json!({
        "name": format!("{device_name} Online"),
        "unique_id": format!("panopticon_online_{device_id}"),
        "state_topic": lock_availability_topic(device_id),
        "device_class": "connectivity",
        "payload_on": "ON",
        "payload_off": "OFF",
        "availability": [
            { "topic": bridge_state_topic() }
        ],
        "device": device_obj(config, device_id, device_name),
    })
}

fn scan_discovery(config: &MqttConfig) -> serde_json::Value {
    json!({
        "name": "Last RFID Scan",
        "unique_id": "panopticon_scan_last",
        "state_topic": scan_topic(),
        "value_template": "{{ value_json.tag_id }}",
        "json_attributes_topic": scan_topic(),
        "icon": "mdi:card-account-details",
        "availability": [
            { "topic": bridge_state_topic() }
        ],
        "device": {
            "identifiers": [format!("panopticon_{}", config.client_id)],
            "name": "Panopticon",
            "manufacturer": "Panopticon",
        },
    })
}

fn sentinel_discovery(
    config: &MqttConfig,
    sentinel_id: &uuid::Uuid,
    sentinel_name: &str,
) -> serde_json::Value {
    json!({
        "name": format!("{sentinel_name} Connected"),
        "unique_id": format!("panopticon_sentinel_{sentinel_id}"),
        "state_topic": sentinel_connected_topic(sentinel_id),
        "device_class": "connectivity",
        "payload_on": "ON",
        "payload_off": "OFF",
        "availability": [
            { "topic": bridge_state_topic() }
        ],
        "device": {
            "identifiers": [format!("panopticon_sentinel_{sentinel_id}")],
            "name": format!("Sentinel: {sentinel_name}"),
            "manufacturer": "Panopticon",
            "via_device": format!("panopticon_{}", config.client_id),
        },
    })
}

fn mode_discovery(config: &MqttConfig) -> serde_json::Value {
    json!({
        "name": "Sentinel Mode",
        "unique_id": "panopticon_sentinel_mode",
        "state_topic": mode_state_topic(),
        "command_topic": mode_command_topic(),
        "options": ["guard", "enroll"],
        "icon": "mdi:shield-lock",
        "availability": [
            { "topic": bridge_state_topic() }
        ],
        "device": {
            "identifiers": [format!("panopticon_{}", config.client_id)],
            "name": "Panopticon",
            "manufacturer": "Panopticon",
        },
    })
}

fn device_obj(config: &MqttConfig, device_id: &str, device_name: &str) -> serde_json::Value {
    json!({
        "identifiers": [format!("panopticon_lock_{device_id}")],
        "name": device_name,
        "manufacturer": "U-Tec",
        "via_device": format!("panopticon_{}", config.client_id),
    })
}

// ── Bulk publish ────────────────────────────────────────────────────────────

async fn publish_all_discovery(client: &AsyncClient, config: &MqttConfig, state: &AppState) {
    // Lock devices
    if let Some(utec) = state.auth_store.client().await {
        match utec.discover_locks().await {
            Ok(locks) => {
                for lock in &locks {
                    let id = &lock.id;
                    let name = &lock.name;
                    publish_retained(
                        client,
                        &discovery_topic(config, "lock", &format!("lock_{id}")),
                        &lock_discovery(config, id, name),
                    )
                    .await;
                    publish_retained(
                        client,
                        &discovery_topic(config, "sensor", &format!("battery_{id}")),
                        &battery_discovery(config, id, name),
                    )
                    .await;
                    publish_retained(
                        client,
                        &discovery_topic(config, "binary_sensor", &format!("online_{id}")),
                        &online_discovery(config, id, name),
                    )
                    .await;
                }
            }
            Err(e) => error!("MQTT: failed to discover locks for discovery: {e:#}"),
        }
    }

    // Scan sensor
    publish_retained(
        client,
        &discovery_topic(config, "sensor", "scan_last"),
        &scan_discovery(config),
    )
    .await;

    // Mode select
    publish_retained(
        client,
        &discovery_topic(config, "select", "sentinel_mode"),
        &mode_discovery(config),
    )
    .await;

    // Sentinels
    publish_sentinel_discovery(client, config, &state.db).await;
}

async fn publish_sentinel_discovery(client: &AsyncClient, config: &MqttConfig, db: &PgPool) {
    let rows: Result<Vec<(uuid::Uuid, String)>, _> =
        sqlx::query_as("SELECT id, name FROM sentinels")
            .fetch_all(db)
            .await;

    match rows {
        Ok(sentinels) => {
            for (id, name) in &sentinels {
                publish_retained(
                    client,
                    &discovery_topic(config, "binary_sensor", &format!("sentinel_{id}")),
                    &sentinel_discovery(config, id, name),
                )
                .await;
            }
        }
        Err(e) => error!("MQTT: failed to query sentinels for discovery: {e:#}"),
    }
}

async fn publish_all_states(client: &AsyncClient, state: &AppState) {
    // Lock states
    if let Some(utec) = state.auth_store.client().await {
        match utec.discover_locks().await {
            Ok(locks) => {
                let lock_refs: Vec<&_> = locks.iter().collect();
                match utec.query_devices(&lock_refs).await {
                    Ok(states) => {
                        for lock in &locks {
                            let device_states = states.iter().find(|s| s.id == lock.id);
                            if let Some(ds) = device_states {
                                if let Some(ls) = ds.lock_state() {
                                    let mqtt_state =
                                        if ls == "locked" { "LOCKED" } else { "UNLOCKED" };
                                    publish(client, &lock_state_topic(&lock.id), mqtt_state).await;
                                }
                                if let Some(battery) = ds.battery_level() {
                                    let (min, max) = lock
                                        .attributes
                                        .as_ref()
                                        .and_then(|a| a.get("batteryLevelRange"))
                                        .map(|r| {
                                            let min =
                                                r.get("min").and_then(|v| v.as_u64()).unwrap_or(0);
                                            let max = r
                                                .get("max")
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(100);
                                            (min, max)
                                        })
                                        .unwrap_or((0, 100));
                                    let pct = if max > min {
                                        ((battery.saturating_sub(min)) * 100) / (max - min)
                                    } else {
                                        battery
                                    };
                                    publish(client, &battery_topic(&lock.id), &pct.to_string())
                                        .await;
                                }
                                let online = if ds.is_online() { "ON" } else { "OFF" };
                                publish(client, &lock_availability_topic(&lock.id), online).await;
                            }
                        }
                    }
                    Err(e) => error!("MQTT: failed to query device states: {e:#}"),
                }
            }
            Err(e) => error!("MQTT: failed to discover locks for state publish: {e:#}"),
        }
    }

    // Sentinel states
    let rows: Result<Vec<(uuid::Uuid, bool)>, _> =
        sqlx::query_as("SELECT id, connected FROM sentinels")
            .fetch_all(&state.db)
            .await;
    if let Ok(sentinels) = rows {
        for (id, connected) in sentinels {
            let payload = if connected { "ON" } else { "OFF" };
            publish(client, &sentinel_connected_topic(&id), payload).await;
        }
    }

    // Mode
    let mode: Result<String, _> =
        sqlx::query_scalar("SELECT value FROM system_config WHERE key = 'sentinel_mode'")
            .fetch_one(&state.db)
            .await;
    if let Ok(mode) = mode {
        publish(client, &mode_state_topic(), &mode).await;
    }
}

// ── Command handlers ────────────────────────────────────────────────────────

async fn handle_lock_command(state: &AppState, device_id: &str, payload: &str) {
    let command = payload.trim().to_uppercase();
    if command != "LOCK" && command != "UNLOCK" {
        warn!("MQTT: ignoring unknown lock command: {payload}");
        return;
    }

    let Some(utec) = state.auth_store.client().await else {
        error!("MQTT: no U-Tec client available for lock command");
        return;
    };

    let locks = match utec.discover_locks().await {
        Ok(l) => l,
        Err(e) => {
            error!("MQTT: failed to discover locks for command: {e:#}");
            return;
        }
    };

    let Some(device) = locks.iter().find(|d| d.id == device_id) else {
        error!("MQTT: device {device_id} not found");
        return;
    };

    let result = if command == "LOCK" {
        utec.lock(device).await
    } else {
        utec.unlock(device).await
    };

    match result {
        Ok(results) => {
            crate::api::handle_lock_response(state, device_id, device, &results, "mqtt", None)
                .await;
            info!(device_id, command, "MQTT: lock command executed");
        }
        Err(e) => error!(device_id, command, "MQTT: lock command failed: {e:#}"),
    }
}

async fn handle_mode_command(state: &AppState, payload: &str) {
    let mode = payload.trim().to_lowercase();
    if mode != "guard" && mode != "enroll" {
        warn!("MQTT: ignoring unknown mode command: {payload}");
        return;
    }

    match sqlx::query("UPDATE system_config SET value = $1 WHERE key = 'sentinel_mode'")
        .bind(&mode)
        .execute(&state.db)
        .await
    {
        Ok(_) => {
            info!(mode, "MQTT: sentinel mode changed");
            let _ = state.events.send(WsEvent::ModeChanged { mode });
        }
        Err(e) => error!("MQTT: failed to set mode: {e:#}"),
    }
}

// ── Publish helpers ─────────────────────────────────────────────────────────

async fn publish(client: &AsyncClient, topic: &str, payload: &str) {
    if let Err(e) = client
        .publish(topic, QoS::AtLeastOnce, false, payload.as_bytes())
        .await
    {
        error!(topic, "MQTT publish failed: {e}");
    }
}

async fn publish_retained(client: &AsyncClient, topic: &str, payload: &serde_json::Value) {
    let bytes = serde_json::to_string(payload).unwrap();
    if let Err(e) = client
        .publish(topic, QoS::AtLeastOnce, true, bytes.as_bytes())
        .await
    {
        error!(topic, "MQTT publish (retained) failed: {e}");
    }
}

// ── Entry point ─────────────────────────────────────────────────────────────

pub async fn spawn_mqtt_bridge(
    mut rx: broadcast::Receiver<WsEvent>,
    state: AppState,
    config: MqttConfig,
) {
    let mut opts = MqttOptions::new(&config.client_id, &config.host, config.port);
    opts.set_keep_alive(Duration::from_secs(30));
    opts.set_last_will(LastWill::new(
        bridge_state_topic(),
        "offline",
        QoS::AtLeastOnce,
        true,
    ));

    if let (Some(ref user), Some(ref pass)) = (&config.username, &config.password) {
        opts.set_credentials(user, pass);
    }

    let (client, mut eventloop) = AsyncClient::new(opts, 64);
    let mut refresh_interval = tokio::time::interval(Duration::from_secs(300));

    info!("MQTT bridge started");

    loop {
        tokio::select! {
            event = eventloop.poll() => {
                match event {
                    Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                        info!("MQTT: connected to broker");
                        // Publish online (retained, to match LWT)
                        let _ = client
                            .publish(bridge_state_topic(), QoS::AtLeastOnce, true, "online")
                            .await;

                        // Subscribe to command topics
                        if let Err(e) = client
                            .subscribe(
                                format!("{BASE}/lock/+/set"),
                                QoS::AtLeastOnce,
                            )
                            .await
                        {
                            error!("MQTT: failed to subscribe to lock commands: {e}");
                        }
                        if let Err(e) = client
                            .subscribe(&mode_command_topic(), QoS::AtLeastOnce)
                            .await
                        {
                            error!("MQTT: failed to subscribe to mode commands: {e}");
                        }

                        // Publish discovery configs and current state
                        publish_all_discovery(&client, &config, &state).await;
                        publish_all_states(&client, &state).await;
                    }
                    Ok(Event::Incoming(Incoming::Publish(msg))) => {
                        handle_incoming_publish(&state, &msg).await;
                    }
                    Ok(_) => {} // PingResp, SubAck, etc.
                    Err(e) => {
                        error!("MQTT: eventloop error: {e}");
                        // rumqttc will auto-reconnect; sleep briefly to avoid tight loop
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }

            result = rx.recv() => {
                match result {
                    Ok(event) => {
                        handle_ws_event(&client, &config, &event).await;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("MQTT bridge lagged, skipped {n} events");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        info!("MQTT bridge shutting down (channel closed)");
                        return;
                    }
                }
            }

            _ = refresh_interval.tick() => {
                publish_all_states(&client, &state).await;
            }
        }
    }
}

async fn handle_incoming_publish(state: &AppState, msg: &Publish) {
    let topic = &msg.topic;
    let payload = match std::str::from_utf8(&msg.payload) {
        Ok(s) => s,
        Err(_) => {
            warn!("MQTT: received non-UTF8 payload on {topic}");
            return;
        }
    };

    // panopticon/lock/{device_id}/set
    if let Some(rest) = topic.strip_prefix(&format!("{BASE}/lock/")) {
        if let Some(device_id) = rest.strip_suffix("/set") {
            handle_lock_command(state, device_id, payload).await;
            return;
        }
    }

    // panopticon/sentinel/mode/set
    if topic == &mode_command_topic() {
        handle_mode_command(state, payload).await;
    }
}

async fn handle_ws_event(client: &AsyncClient, config: &MqttConfig, event: &WsEvent) {
    match event {
        WsEvent::LockState {
            device_id,
            lock_state,
        } => {
            let mqtt_state = if lock_state == "locked" {
                "LOCKED"
            } else {
                "UNLOCKED"
            };
            publish(client, &lock_state_topic(device_id), mqtt_state).await;
        }
        WsEvent::Scan {
            tag_id,
            action,
            created_at,
        } => {
            let payload = json!({
                "tag_id": tag_id,
                "action": action,
                "created_at": created_at,
            })
            .to_string();
            if let Err(e) = client
                .publish(&scan_topic(), QoS::AtLeastOnce, false, payload.as_bytes())
                .await
            {
                error!("MQTT: failed to publish scan: {e}");
            }
        }
        WsEvent::ModeChanged { mode } => {
            publish(client, &mode_state_topic(), mode).await;
        }
        WsEvent::SentinelConnected { id, name } => {
            publish(client, &sentinel_connected_topic(id), "ON").await;
            publish_retained(
                client,
                &discovery_topic(config, "binary_sensor", &format!("sentinel_{id}")),
                &sentinel_discovery(config, id, name),
            )
            .await;
        }
        WsEvent::SentinelDisconnected { id } => {
            publish(client, &sentinel_connected_topic(id), "OFF").await;
        }
        _ => {} // CardAdded, CardRemoved, SentinelLog — no MQTT mapping
    }
}
