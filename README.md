# rfid-door

RFID-based door access control system with two components:

- **sentinel/** — ESP32 firmware (Rust). Reads 125kHz RFID tags via a RFIDuino Shield v1.2 and reports scans to the panopticon server over WiFi.
- **panopticon/** — Web server (Rust/axum). Manages access control via a Svelte 5 frontend with U-Tec smart lock OAuth2 integration.

---

## Sentinel (ESP32 firmware)

### How it works

1. The RFIDuino Shield's EM4095 chip continuously reads 125kHz RFID tags and outputs a demodulated digital signal
2. The ESP32 Manchester-decodes this signal into a 5-byte tag ID (a direct Rust port of the RFIDuino Arduino library)
3. The tag ID is POSTed to the panopticon server's `/api/sentinel/scan` endpoint with a shared secret for authentication
4. Panopticon decides whether to grant or deny access

---

## Hardware

### Why ESP32 instead of Arduino?

The RFIDuino Shield v1.2 is designed for Arduino, so the choice to move to an ESP32 was deliberate.

**Why not classic Arduino (ATmega328P)?**
- AVR is a **Tier 3** Rust target — requires nightly compiler, known LLVM backend bugs
- Only **2KB SRAM** — TLS needs ~20-40KB minimum for handshake buffers
- No WiFi — would need an external module (ESP8266 AT coprocessor), meaning WiFi/HTTPS runs in proprietary firmware you don't control
- HTTPS is physically impossible in 2KB RAM regardless of language

**Why not Cortex-M Arduino boards (Uno R4 WiFi, Nano 33 IoT)?**
- These all use a **WiFi coprocessor architecture**: the main MCU (where your Rust runs) talks to a separate ESP32-S3 or u-blox NINA module over UART
- WiFi and TLS run on black-box proprietary firmware inside the coprocessor
- So the "write everything in Rust" goal breaks — the most complex part (WiFi + HTTPS) is happening in C firmware you don't control
- The Uno R4 WiFi is $25 and literally has an ESP32-S3 inside it — you're paying for a Cortex-M4 middleman
- Cortex-M Rust is more mature *in general*, but that advantage evaporates when WiFi is stuck behind a serial protocol to a coprocessor

**Why ESP32 wins:**
- **Xtensa** dual-core @ 240MHz — well-supported via the `esp` Rust toolchain fork
- **WiFi integrated into the MCU** — the entire stack (GPIO, WiFi, TLS, HTTP) runs in one binary, all callable from Rust
- **520KB SRAM, 4MB flash** — plenty for TLS, HTTP buffers, and application logic
- **$3-6** for an ESP32 DevKit v1 board
- `esp-idf-svc` provides WiFi and HTTPS (via bundled mbedTLS) with ergonomic Rust APIs

### Recommended dev boards

| Board | Price | Notes |
|-------|-------|-------|
| **ESP32 DevKit v1** | ~$3-6 | 30-pin, USB-micro, widely available, all GPIOs broken out |
| **ESP32-DevKitC-32E** | ~$8-10 | Espressif official, USB-micro, 38-pin, reliable |
| **ESP32-WROOM-32E** | ~$3-5 | Module only — solder to a breakout if you want compact |

All have USB with built-in USB-serial (no external programmer needed).

### ESP32 GPIO constraints

On the original ESP32, **GPIO6–11 are connected to the internal SPI flash** and must not be used for external peripherals. GPIO16–17 may be connected to PSRAM on WROVER modules. The pin assignments below avoid all of these.

### Why the RFIDuino library port is feasible

The RFIDuino library looks complex but is actually ~170 lines of bit-banging:
- The **EM4095** chip does all analog RF (antenna driving, envelope detection, demodulation)
- The MCU just reads a clean digital signal and Manchester-decodes it
- Communication is pure GPIO reads with 320µs timing delays — no SPI, no UART, no complex peripherals
- The 320µs bit period is very relaxed for a 160MHz core (massive timing margin even with WiFi interrupts)
- The EM4100 protocol is elegant: 9 header bits, then a 10×5 data matrix with row and column parity checks

### `std` vs `no_std` — why we chose `std`

| Approach | Pros | Cons |
|----------|------|------|
| **`esp-idf-svc` (std)** | Easy WiFi/TLS, familiar Rust, ergonomic HTTP | Larger binary, slower compile, pulls in ESP-IDF C framework |
| **`esp-hal` + `embassy` (no_std)** | Minimal, pure Rust, async, smaller binary | Manual WiFi/TLS setup, less mature HTTP stack |

**We use `std` (`esp-idf-svc`)** because HTTPS requires TLS, and the std ecosystem makes TLS trivial via ESP-IDF's bundled mbedTLS with a root CA certificate bundle. In `no_std`, you'd need to manually integrate an `embedded-tls` stack — doable but painful. Binary size is irrelevant with 4MB flash.

---

## Electrical considerations

### Voltage compatibility

This is the most important hardware detail. The RFIDuino Shield is designed for 5V Arduino boards:

| Component | Voltage |
|-----------|---------|
| EM4095 operating range | **2.7V – 5.5V** |
| Arduino Uno VCC | 5V |
| ESP32 VCC | **3.3V** |
| ESP32 GPIO absolute max | **3.6V** (NOT 5V tolerant) |

**If you power the EM4095 at 5V, its demod_out signal swings to 5V — this will damage the ESP32.**

### Recommended solution: power the shield at 3.3V

The EM4095 is spec'd down to 2.7V. Supply 3.3V from the ESP32 to the shield's VCC:
- All signal levels stay within 3.3V — directly safe for ESP32 GPIOs
- Antenna range is slightly reduced vs 5V, but the v1.2 shield has a strong antenna and should still read tags at contact/near-contact distance
- LEDs will be dimmer but functional

### Alternative: 5V + level shifters

If you need maximum range:
- Power the shield at 5V (from USB or external supply)
- Use a bidirectional 3.3V ↔ 5V logic level shifter on all signal lines
- More wiring, but full RF performance

### Wiring

You are NOT stacking the shield — you wire point-to-point from the ESP32 to the shield's Arduino header pads or RobotGeek 3-pin connectors.

```
ESP32 DevKit               RFIDuino Shield v1.2        Function
─────────────              ────────────────────        ────────
GPIO13 ──────────────────── D3 pad (demod_out)          RFID data from EM4095
GPIO14 ──────────────────── D2 pad (rdy_clk)            EM4095 clock
GPIO15 ──────────────────── D7 pad (shd)                EM4095 shutdown
GPIO18 ──────────────────── D6 pad (mod)                EM4095 modulation
GPIO19 ──────────────────── D5 pad (buzzer)             Piezo buzzer (PWM)
GPIO21 ──────────────────── D8 pad (led1)               Red LED
GPIO22 ──────────────────── D4 pad (led2)               Green LED
3.3V   ──────────────────── VCC
GND    ──────────────────── GND
```

The GPIO numbers are configurable — edit the pin assignments in `src/main.rs`.

The buzzer is a passive piezo driven with a PWM square wave. The Arduino library uses `tone()` at 1300–4500Hz; on the ESP32 you use the LEDC peripheral to generate the same kind of signal.

### Which connectors on the shield?

The shield has several connection points:
- **Arduino header pads** — the through-hole pads where it would normally stack onto an Arduino. The RFID signals (D2, D3, D6, D7), buzzer (D5), and LEDs (D4, D8) are all here.
- **3-pin RobotGeek connectors** — rows of 3-pin headers labelled **S-V-G** on the board, where **S** = Signal, **V** = Voltage (VCC), **G** = Ground. These break out individual Arduino pins with power and ground alongside, so you can plug in a servo or sensor with a single 3-pin cable. The shield has two groups:
  - **DIO-9 through DIO-12** — Digital I/O pins 9–12 (unused by the shield itself)
  - **AIO-0 through AIO-3** — Analog I/O pins A0–A3 (unused by the shield itself)
- **4-pin I2C connector** — not needed for this project
- **XBee socket** — not needed

The DIO/AIO connectors are free pins for your own peripherals. The 7 pins the shield actually uses (D2, D3, D4, D5, D6, D7, D8) are only accessible via the Arduino header pads — they don't have their own 3-pin breakout connectors.

---

## Setup

### Prerequisites

```bash
# Install Rust (if you don't have it)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install the Xtensa Rust toolchain for ESP32
cargo install espup
espup install
source ~/export-esp.sh   # sets up PATH and LIBCLANG_PATH

# Install ESP32 Rust tooling
cargo install espflash         # Flashing tool (includes monitor)
cargo install ldproxy          # Linker proxy for ESP-IDF
```

### Configuration

Copy the example env file and fill in your values:

```bash
cd sentinel
cp .env.example .env
```

Edit `sentinel/.env` with your WiFi credentials, panopticon server URL, and shared secret:

```
WIFI_SSID=your_wifi_ssid
WIFI_PASS=your_wifi_password
PANOPTICON_URL=https://your-panopticon-server.example.com
SENTINEL_SECRET=generate_a_random_32char_hex_string
```

These are embedded into the firmware binary at compile time via `build.rs` — the `.env` file is gitignored and never committed.

The `SENTINEL_SECRET` must match the value configured on the panopticon server.

### Build and flash

**Important:** Connect the USB cable directly to the ESP32 dev board's USB port, not the USB port on any breakout board or shield. The board's USB-serial chip is used for both flashing and serial monitoring.

```bash
cd sentinel

# Build (first build downloads ESP-IDF SDK — takes a while)
cargo build

# Build and flash to connected ESP32, then open serial monitor
cargo run
```

The serial monitor (`espflash monitor`) shows log output over USB at runtime.

---

## Project structure

```
rfid-door/
├── sentinel/                   # ESP32 firmware
│   ├── Cargo.toml
│   ├── build.rs
│   ├── sdkconfig.defaults
│   ├── .cargo/config.toml
│   ├── src/
│   │   ├── main.rs             # WiFi, IFTTT webhook, main scan loop
│   │   └── rfiduino.rs         # Ported RFIDuino library (Manchester decode)
│   └── RFIDuino/               # Original Arduino library (reference only)
│
├── panopticon/                 # Web server
│   ├── Cargo.toml
│   ├── build.rs                # Builds Svelte app and embeds into binary
│   ├── deploy                  # Build + install + restart script
│   ├── panopticon.service      # systemd unit file
│   ├── Caddyfile               # Reverse proxy config for hut8.tools
│   ├── src/
│   │   ├── main.rs             # Axum server on :1337, static file serving
│   │   └── oauth.rs            # U-Tec OAuth2 flow
│   └── web/                    # Svelte 5 SPA (SvelteKit + Tailwind + Skeleton)
│       ├── package.json
│       ├── svelte.config.js
│       ├── vite.config.ts
│       └── src/
│           ├── app.css
│           ├── app.html
│           └── routes/
│               ├── +layout.svelte
│               ├── +layout.ts
│               └── +page.svelte
└── README.md
```

## About the EM4100 protocol

The EM4100 is a 125kHz read-only RFID protocol from the 1990s. Each tag transmits a 64-bit Manchester-encoded frame:

```
┌─────────────────────────────────────────────────────────────────┐
│  9 header bits (all 1s)                                        │
├──────┬──────┬──────┬──────┬────────┐                           │
│ D0   │ D1   │ D2   │ D3   │ Parity │  ← Row 0 (4 data + 1P) │
│ D4   │ D5   │ D6   │ D7   │ Parity │  ← Row 1               │
│  ⋮   │  ⋮   │  ⋮   │  ⋮   │   ⋮    │     ⋮                  │
│ D36  │ D37  │ D38  │ D39  │ Parity │  ← Row 9               │
├──────┼──────┼──────┼──────┼────────┤                           │
│  CP  │  CP  │  CP  │  CP  │   0    │  ← Column parity row   │
└──────┴──────┴──────┴──────┴────────┘                           │
└─────────────────────────────────────────────────────────────────┘
```

40 data bits = 5 bytes: 1 manufacturer/version byte + 4 ID bytes. Row parity is even parity per row; column parity is even parity per column. This 2D parity grid provides basic error detection — elegant for a protocol designed before CRC was cheap to compute in silicon.

---

## Panopticon (web server)

Axum web server that manages door access via a Svelte 5 SPA frontend.

### Stack

- **Backend:** Rust / axum on port 1337
- **Frontend:** Svelte 5, SvelteKit, Tailwind CSS 4, Skeleton UI v4.12
- **Deployment:** Single binary with embedded frontend assets (via `include_dir`)
- **Reverse proxy:** Caddy (hut8.tools → localhost:1337)
- **Process management:** systemd

### U-Tec OAuth2 integration

Panopticon implements OAuth2 for the U-Tec smart lock API:

1. `/auth/login` → Redirects to `https://oauth.u-tec.com/authorize`
2. U-Tec redirects back to `https://hut8.tools/auth/callback` with an authorization code
3. `/auth/callback` → Exchanges the code for an access token via `https://oauth.u-tec.com/token`

Set `CLIENT_ID` and `CLIENT_SECRET` in `panopticon/src/oauth.rs` before deploying. (These should be moved to environment variables for production.)

### Development

```bash
# Terminal 1: Run the Rust backend
cd panopticon
cargo run

# Terminal 2: Run the Vite dev server (hot-reloading frontend)
cd panopticon/web
npm install
npm run dev
```

The Vite dev server (port 5173) proxies `/auth/*` requests to the Rust backend (port 1337).

### Deployment

```bash
cd panopticon
./deploy
```

This script:
1. Builds the release binary (which also builds the Svelte app via `build.rs`)
2. Stops the systemd service
3. Installs the binary to `/usr/local/bin/panopticon`
4. Installs the systemd unit file
5. Starts the service

### Caddy setup

Copy the Caddyfile or add its contents to your existing Caddy config:

```bash
sudo cp panopticon/Caddyfile /etc/caddy/Caddyfile
sudo systemctl reload caddy
```
