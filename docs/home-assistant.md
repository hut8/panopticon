# Home Assistant Integration (MQTT)

Panopticon can integrate with [Home Assistant](https://www.home-assistant.io/) via MQTT. When enabled, it publishes device state using HA's [MQTT Discovery](https://www.home-assistant.io/integrations/mqtt/#mqtt-discovery) protocol, so entities auto-appear in HA with zero HA-side configuration beyond having an MQTT broker.

The bridge is optional — it only activates when `MQTT_HOST` is set.

## Prerequisites

1. An MQTT broker accessible to both Panopticon and Home Assistant (e.g., [Mosquitto](https://mosquitto.org/))
2. The [MQTT integration](https://www.home-assistant.io/integrations/mqtt/) configured in Home Assistant, pointed at the same broker
3. MQTT discovery enabled in HA (this is the default)

## Configuration

Set these environment variables for Panopticon (in your `.env` file or systemd `EnvironmentFile`):

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `MQTT_HOST` | Yes | — | Broker hostname (e.g., `localhost` or `192.168.1.50`) |
| `MQTT_PORT` | No | `1883` | Broker port |
| `MQTT_USERNAME` | No | — | Broker auth username |
| `MQTT_PASSWORD` | No | — | Broker auth password |
| `MQTT_CLIENT_ID` | No | `panopticon` | MQTT client ID |
| `MQTT_DISCOVERY_PREFIX` | No | `homeassistant` | HA discovery topic prefix |

Example `.env` addition:

```
MQTT_HOST=localhost
MQTT_USERNAME=panopticon
MQTT_PASSWORD=secretpassword
```

## Entities

Once connected, the following entities appear automatically in Home Assistant:

### Per lock device

| Entity | HA type | Description |
|--------|---------|-------------|
| Lock | `lock` | Lock/unlock control — commands are forwarded to the U-Tec API |
| Battery | `sensor` | Battery level (0–100%) |
| Online | `binary_sensor` | Whether the lock is reachable |

### Global

| Entity | HA type | Description |
|--------|---------|-------------|
| Last RFID Scan | `sensor` | Tag ID of the last scan, with `action` and `created_at` as attributes |
| Sentinel Mode | `select` | Switch between `guard` and `enroll` mode |

### Per sentinel

| Entity | HA type | Description |
|--------|---------|-------------|
| Connected | `binary_sensor` | Whether the sentinel is currently connected |

## MQTT topics

All state topics are under the `panopticon/` prefix:

```
panopticon/bridge/state              → "online" / "offline" (LWT)
panopticon/lock/{id}/state           → "LOCKED" / "UNLOCKED"
panopticon/lock/{id}/battery         → "85" (percentage)
panopticon/lock/{id}/availability    → "ON" / "OFF"
panopticon/lock/{id}/set             → command topic (send "LOCK" or "UNLOCK")
panopticon/scan/last                 → {"tag_id":"...","action":"granted","created_at":"..."}
panopticon/sentinel/{id}/connected   → "ON" / "OFF"
panopticon/sentinel/mode/state       → "guard" / "enroll"
panopticon/sentinel/mode/set         → command topic (send "guard" or "enroll")
```

Discovery configs are published (retained) under `{discovery_prefix}/{component}/panopticon/{object_id}/config`.

## How it works

The MQTT bridge runs as a background task that:

1. **Listens to internal events** — subscribes to the same broadcast channel as the WebSocket and push notification systems, translating `WsEvent`s into MQTT publishes
2. **Handles incoming commands** — subscribes to `panopticon/lock/+/set` and `panopticon/sentinel/mode/set`, forwarding lock commands to the U-Tec API and mode changes to the database
3. **Periodic refresh** — every 5 minutes, queries all device states and republishes (catches battery/online changes that don't generate events)
4. **Auto-reconnect** — on broker disconnect, `rumqttc` reconnects automatically; on reconnect, all discovery configs and state are republished
5. **Last Will and Testament** — the broker publishes `offline` to `panopticon/bridge/state` if Panopticon disconnects unexpectedly, causing HA to mark all entities as unavailable

## Verification

After setting `MQTT_HOST` and restarting Panopticon:

```bash
# Watch discovery configs appear (retained)
mosquitto_sub -h localhost -t 'homeassistant/#' -v

# Watch state updates
mosquitto_sub -h localhost -t 'panopticon/#' -v
```

In Home Assistant, the entities should appear under **Settings → Devices & services → MQTT**. You can:

- Lock/unlock from the HA dashboard or automations
- Monitor battery levels and online status
- See RFID scan events in real time
- Switch sentinel mode between guard and enroll
- Verify that killing Panopticon marks all entities as unavailable

## Example automation

Notify when access is denied:

```yaml
automation:
  - alias: "Notify on denied RFID scan"
    trigger:
      - platform: state
        entity_id: sensor.last_rfid_scan
    condition:
      - condition: template
        value_template: "{{ trigger.to_state.attributes.action == 'denied' }}"
    action:
      - service: notify.mobile_app
        data:
          title: "Access Denied"
          message: "Card {{ trigger.to_state.state }} was denied"
```
