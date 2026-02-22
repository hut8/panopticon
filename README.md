# rfid-door

RFID-based door access control running on an **ESP32-C3** (RISC-V) in **Rust**, using a **RFIDuino Shield v1.2** for 125kHz EM4100 tag reading. Authorised tags trigger an IFTTT webhook over HTTPS/WiFi.

## How it works

1. The RFIDuino Shield's EM4095 chip continuously reads 125kHz RFID tags and outputs a demodulated digital signal
2. The ESP32-C3 Manchester-decodes this signal into a 5-byte tag ID (a direct Rust port of the RFIDuino Arduino library)
3. The tag ID is compared against a hardcoded allowlist
4. If authorised, an HTTPS POST is sent to an IFTTT webhook with the tag ID as payload

---

## Hardware

### Why ESP32-C3 (RISC-V) instead of Arduino?

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

**Why ESP32-C3 wins:**
- **RISC-V** single-core @ 160MHz — Tier 2 in upstream Rust (stable compiler, no custom toolchain)
- **WiFi integrated into the MCU** — the entire stack (GPIO, WiFi, TLS, HTTP) runs in one binary, all callable from Rust
- **400KB SRAM, 4MB flash** — plenty for TLS, HTTP buffers, and application logic
- **$3-5** for an ESP32-C3 SuperMini dev board
- `esp-idf-svc` provides WiFi and HTTPS (via bundled mbedTLS) with ergonomic Rust APIs
- `esp-hal` 1.0.0-beta (Feb 2025) marks the ecosystem as stable and production-ready

### Recommended dev boards

| Board | Price | Notes |
|-------|-------|-------|
| **ESP32-C3 SuperMini** | ~$3-5 | Best bang for buck. USB-C, 13 GPIOs, ultra-compact (22.5×18mm) |
| **Seeed XIAO ESP32-C3** | ~$5-7 | More polished, castellated pads, excellent documentation |
| **ESP32-C6-DevKitC-1** | ~$8-12 | WiFi 6 + Zigbee/Thread/Matter. Overkill, but future-proof |

All have USB-C with built-in USB-serial (no external programmer needed).

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
| ESP32-C3 VCC | **3.3V** |
| ESP32-C3 GPIO absolute max | **3.6V** (NOT 5V tolerant) |

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
ESP32-C3 SuperMini          RFIDuino Shield v1.2
──────────────────          ────────────────────
GPIO2  ──────────────────── demod_out  (D3 pad)
GPIO3  ──────────────────── rdy_clk    (D2 pad)
GPIO4  ──────────────────── shd        (D7 pad)
GPIO5  ──────────────────── mod        (D6 pad)
3.3V   ──────────────────── VCC
GND    ──────────────────── GND
```

The GPIO numbers are configurable — edit the `PIN_*` constants in `src/main.rs`.

### Which connectors on the shield?

The shield has several connection points:
- **3-pin RobotGeek connectors** (Signal-VCC-GND) on the digital I/O side — easiest for jumper wires
- **Arduino header pads** — the through-hole pads where it would normally stack onto an Arduino
- **4-pin I2C connector** — not needed for this project
- **XBee socket** — not needed

The 4 signals you need (demod_out, rdy_clk, shd, mod) are on digital pins D2, D3, D6, D7 of the Arduino header footprint.

---

## Setup

### Prerequisites

```bash
# Install Rust (if you don't have it)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add the RISC-V target for ESP32-C3
rustup target add riscv32imc-unknown-none-elf

# Install ESP32 Rust tooling
cargo install cargo-generate  # Project scaffolding (already done)
cargo install espflash         # Flashing tool (includes monitor)
cargo install ldproxy          # Linker proxy for ESP-IDF
```

### Configuration

Edit the constants at the top of `src/main.rs`:

```rust
const WIFI_SSID: &str = "YOUR_WIFI_SSID";
const WIFI_PASS: &str = "YOUR_WIFI_PASSWORD";
const IFTTT_EVENT: &str = "rfid_door";
const IFTTT_KEY: &str = "YOUR_IFTTT_KEY";
```

Add your RFID tag IDs to the allowlist:

```rust
const ALLOWED_TAGS: &[TagId] = &[
    [128, 0, 72, 35, 76],  // Replace with your actual tag IDs
];
```

To discover your tag IDs, flash the firmware with an empty allowlist and scan your tags — the IDs will appear in the serial monitor log.

### Build and flash

```bash
# Build (first build downloads ESP-IDF SDK — takes a while)
cargo build

# Build and flash to connected ESP32-C3, then open serial monitor
cargo run
```

The serial monitor (`espflash monitor`) shows log output over USB at runtime.

### IFTTT setup

1. Go to [IFTTT](https://ifttt.com) and create a new applet
2. **If This**: Choose "Webhooks" → "Receive a web request"
3. Set the event name to match `IFTTT_EVENT` (default: `rfid_door`)
4. **Then That**: Choose your action (e.g., unlock a smart lock, send a notification)
5. Go to [IFTTT Webhooks settings](https://ifttt.com/maker_webhooks) → "Documentation" to find your key
6. Put that key in `IFTTT_KEY`

The webhook payload sends the tag ID as `value1`.

---

## Project structure

```
rfid-door/
├── Cargo.toml              # Dependencies: esp-idf-svc, anyhow, log
├── build.rs                # ESP-IDF build system integration
├── sdkconfig.defaults      # ESP-IDF config (stack size, WiFi, TLS certs)
├── .cargo/
│   └── config.toml         # Build target, linker, runner config
├── src/
│   ├── main.rs             # WiFi, IFTTT webhook, main scan loop
│   └── rfiduino.rs         # Ported RFIDuino library (Manchester decode)
└── RFIDuino/               # Original Arduino library (reference only)
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
