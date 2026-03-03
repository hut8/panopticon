# Sentinel Protocol

The sentinel communicates with panopticon over a persistent TCP connection.
The protocol is plain-text, line-based (newline-delimited). Each message is
a single line of the form:

    TYPE: payload\n

The sentinel connects to panopticon on port **8008** at boot and holds the
connection open. If the connection drops, the sentinel reconnects and
re-authenticates automatically.

## Message types

Messages flow in both directions. The sentinel sends `AUTHZ`, `LOG`, and
`SCAN` messages. Panopticon responds to `SCAN` messages with a `RESULT`.

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

### `RESULT` (panopticon → sentinel)

Sent by panopticon in response to a `SCAN` message. Contains the access
decision so the sentinel can provide LED feedback to the user.

    RESULT: <action>\n

Where `<action>` is one of:

- `granted` — the card is recognized and access is granted
- `denied` — the card is not recognized
- `enrolled` — the card was added in enrollment mode

Only sent in response to `SCAN` messages. `LOG` and `AUTHZ` messages
receive no response. If the sentinel does not read the response (e.g. an
older firmware version), the unread bytes accumulate harmlessly in the
TCP receive buffer.
