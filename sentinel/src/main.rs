mod rfiduino;

use std::time::Duration;

use anyhow::{bail, Result};
use embedded_svc::http::client::Client as HttpClient;
use embedded_svc::io::Write;
use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};
use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::http::client::{Configuration as HttpConfig, EspHttpConnection};
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition};
use log::{error, info, warn};

use rfiduino::{format_tag_id, format_tag_id_hex, RFIDuino, TagId};

// ── Configuration ──────────────────────────────────────────────────────────

/// WiFi credentials. Set these before building.
const WIFI_SSID: &str = "YOUR_WIFI_SSID";
const WIFI_PASS: &str = "YOUR_WIFI_PASSWORD";

/// Panopticon server URL (HTTP, LAN only).
const PANOPTICON_URL: &str = "http://192.168.1.100:1337";

/// Shared secret for authenticating with panopticon.
const SENTINEL_SECRET: &str = "changeme";

/// Cooldown between successful scans of the same tag (prevents rapid re-triggering).
const SCAN_COOLDOWN: Duration = Duration::from_secs(5);

// ── Pin assignments ────────────────────────────────────────────────────────
// Adjust these to match your wiring from the ESP32-C3 to the RFIDuino Shield.
// These are GPIO numbers on the ESP32-C3, NOT the Arduino pin numbers.

/// GPIO connected to EM4095 DEMOD_OUT (shield D3 pad)
const PIN_DEMOD_OUT: u32 = 2;
/// GPIO connected to EM4095 RDY_CLK (shield D2 pad)
const PIN_RDY_CLK: u32 = 3;
/// GPIO connected to EM4095 SHD (shield D7 pad)
const PIN_SHD: u32 = 4;
/// GPIO connected to EM4095 MOD (shield D6 pad)
const PIN_MOD: u32 = 5;

// ── Main ───────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    // ESP-IDF boilerplate
    esp_idf_svc::sys::link_patches();
    EspLogger::initialize_default();

    info!("rfid-door starting up");

    // Take peripherals
    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    // ── WiFi ───────────────────────────────────────────────────────────────
    info!("Connecting to WiFi...");
    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs))?,
        sys_loop,
    )?;
    connect_wifi(&mut wifi)?;
    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;
    info!("WiFi connected — IP: {}", ip_info.ip);

    // ── RFID reader ────────────────────────────────────────────────────────
    info!("Initializing RFIDuino...");

    // Map our pin constants to the actual peripheral pins.
    // The `pins` struct has fields like gpio0, gpio1, ... — we pick them by number.
    let pins = peripherals.pins;
    let mut reader = RFIDuino::new(
        get_input_pin(&pins, PIN_DEMOD_OUT)?,
        get_input_pin(&pins, PIN_RDY_CLK)?,
        get_output_pin(&pins, PIN_SHD)?,
        get_output_pin(&pins, PIN_MOD)?,
    )?;
    info!("RFIDuino ready — scan a tag");

    // ── Main loop ──────────────────────────────────────────────────────────
    let mut last_scan: Option<(TagId, std::time::Instant)> = None;

    loop {
        if let Some(tag) = reader.scan_for_tag() {
            let tag_str = format_tag_id(&tag);
            info!("Tag scanned: {}", tag_str);

            // Cooldown check — don't re-trigger for the same tag within SCAN_COOLDOWN
            let should_trigger = match &last_scan {
                Some((prev_tag, when)) => {
                    *prev_tag != tag || when.elapsed() >= SCAN_COOLDOWN
                }
                None => true,
            };

            if should_trigger {
                let hex_id = format_tag_id_hex(&tag);
                match report_scan(&hex_id) {
                    Ok(true) => info!("Access granted or enrolled for {}", hex_id),
                    Ok(false) => warn!("Access denied for {}", hex_id),
                    Err(e) => error!("Failed to report scan: {e}"),
                }
                last_scan = Some((tag, std::time::Instant::now()));
            }
        }

        // Small delay to avoid busy-spinning the CPU at 100%.
        // The decode_tag function itself has internal waits, but if no tag is
        // present it returns quickly, so this prevents a tight hot loop.
        FreeRtos::delay_ms(50);
    }
}

// ── WiFi ───────────────────────────────────────────────────────────────────

fn connect_wifi(wifi: &mut BlockingWifi<EspWifi<'static>>) -> Result<()> {
    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: WIFI_SSID
            .try_into()
            .map_err(|_| anyhow::anyhow!("SSID too long"))?,
        password: WIFI_PASS
            .try_into()
            .map_err(|_| anyhow::anyhow!("Password too long"))?,
        auth_method: AuthMethod::WPA2Personal,
        ..Default::default()
    }))?;

    wifi.start()?;
    info!("WiFi started");
    wifi.connect()?;
    info!("WiFi associated");
    wifi.wait_netif_up()?;
    info!("WiFi network interface up");
    Ok(())
}

// ── Panopticon scan report ─────────────────────────────────────────────────

/// POST the scanned tag to panopticon. Returns true if action is "granted" or "enrolled".
fn report_scan(tag_id: &str) -> Result<bool> {
    let url = format!("{}/api/sentinel/scan", PANOPTICON_URL);
    let body = format!(r#"{{"tag_id":"{}","secret":"{}"}}"#, tag_id, SENTINEL_SECRET);
    let content_length = body.len().to_string();

    let headers = [
        ("Content-Type", "application/json"),
        ("Content-Length", content_length.as_str()),
    ];

    let mut client = HttpClient::wrap(EspHttpConnection::new(&HttpConfig {
        ..Default::default()
    })?);

    let mut request = client.post(&url, &headers)?;
    request.write_all(body.as_bytes())?;
    request.flush()?;

    let response = request.submit()?;
    let status = response.status();

    if !(200..300).contains(&(status as i32)) {
        bail!("Panopticon returned HTTP {status}");
    }

    // Read response body to check the action
    let mut buf = [0u8; 256];
    let mut reader = response;
    let len = embedded_svc::io::Read::read(&mut reader, &mut buf).unwrap_or(0);
    let body_str = core::str::from_utf8(&buf[..len]).unwrap_or("");

    // Simple check — look for "granted" or "enrolled" in the response
    let success = body_str.contains("granted") || body_str.contains("enrolled");
    Ok(success)
}

// ── Pin mapping helpers ────────────────────────────────────────────────────
// These helpers pick a GPIO by number from the Peripherals pins struct.
// This keeps the pin assignments as simple constants at the top of the file.

use esp_idf_svc::hal::gpio::{AnyInputPin, AnyOutputPin};

macro_rules! match_gpio_input {
    ($pins:expr, $num:expr, [ $($n:literal => $field:ident),+ $(,)? ]) => {
        match $num {
            $( $n => Ok($pins.$field.into()), )+
            other => Err(anyhow::anyhow!("GPIO{other} is not a valid input pin")),
        }
    }
}

macro_rules! match_gpio_output {
    ($pins:expr, $num:expr, [ $($n:literal => $field:ident),+ $(,)? ]) => {
        match $num {
            $( $n => Ok($pins.$field.into()), )+
            other => Err(anyhow::anyhow!("GPIO{other} is not a valid output pin")),
        }
    }
}

fn get_input_pin(
    pins: &esp_idf_svc::hal::gpio::Pins,
    num: u32,
) -> Result<AnyInputPin> {
    // ESP32-C3 has GPIO 0-10, 18-21 (not all available on every board)
    match_gpio_input!(pins, num, [
        0 => gpio0, 1 => gpio1, 2 => gpio2, 3 => gpio3,
        4 => gpio4, 5 => gpio5, 6 => gpio6, 7 => gpio7,
        8 => gpio8, 9 => gpio9, 10 => gpio10,
        18 => gpio18, 19 => gpio19, 20 => gpio20, 21 => gpio21,
    ])
}

fn get_output_pin(
    pins: &esp_idf_svc::hal::gpio::Pins,
    num: u32,
) -> Result<AnyOutputPin> {
    match_gpio_output!(pins, num, [
        0 => gpio0, 1 => gpio1, 2 => gpio2, 3 => gpio3,
        4 => gpio4, 5 => gpio5, 6 => gpio6, 7 => gpio7,
        8 => gpio8, 9 => gpio9, 10 => gpio10,
        18 => gpio18, 19 => gpio19, 20 => gpio20, 21 => gpio21,
    ])
}
