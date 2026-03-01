# Sentinel Protocol

The sentinel communicates with panopticon over a persistent TCP connection.
The protocol is plain-text, line-based (newline-delimited). Each message is
a single line of the form:

    TYPE: payload\n

The sentinel connects to panopticon on port **8008** at boot and holds the
connection open. If the connection drops, the sentinel reconnects and
re-authenticates automatically.

## Message types

All messages flow from sentinel to panopticon. There are no
panopticon-to-sentinel messages at this time.

### `AUTHZ`

Authenticate with a shared secret. Must be the first message sent after
connecting.

    AUTHZ: <secret>\n

If the secret is invalid, panopticon drops the connection.

### `LOG`

Forward a log line from the sentinel. The payload mirrors the ESP-IDF log
format with level and target.

    LOG: [<LEVEL> <target>] <message>\n

Example:

    LOG: [INFO esp_idf_svc::wifi] WiFi connected

### `SCAN`

Report a scanned RFID tag ID. The tag ID is 5 colon-separated uppercase
hex bytes.

    SCAN: <tag_id>\n

Example:

    SCAN: 80:00:48:23:4C
