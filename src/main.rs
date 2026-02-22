mod rfiduino;

use std::thread;
use std::time::Duration;

use anyhow::{bail, Result};
use embedded_svc::http::client::Client as HttpClient;
use embedded_svc::http::Method;
use embedded_svc::io::Write;
use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};
use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::http::client::{Configuration as HttpConfig, EspHttpConnection};
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition};
use log::{error, info, warn};

use rfiduino::{format_tag_id, RFIDuino, TagId};

// ── Configuration ──────────────────────────────────────────────────────────

/// WiFi credentials. Set these before building.
const WIFI_SSID: &str = "YOUR_WIFI_SSID";
const WIFI_PASS: &str = "YOUR_WIFI_PASSWORD";

/// IFTTT Webhook configuration.
/// Your webhook URL is: https://maker.ifttt.com/trigger/{EVENT}/with/key/{KEY}
const IFTTT_EVENT: &str = "rfid_door";
const IFTTT_KEY: &str = "YOUR_IFTTT_KEY";

/// Allowlist of authorised RFID tag IDs.
/// Each entry is [manufacturer_byte, id_byte_1, id_byte_2, id_byte_3, id_byte_4].
/// Find your tag IDs by scanning them and reading the log output.
const ALLOWED_TAGS: &[TagId] = &[
    [128, 0, 72, 35, 76],  // Example — replace with your actual tag IDs
    [128, 0, 12, 99, 200], // Example — replace with your actual tag IDs
];

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
                if is_allowed(&tag) {
                    info!("Tag AUTHORISED — triggering IFTTT webhook");
                    match trigger_ifttt(&tag_str) {
                        Ok(()) => info!("Webhook fired successfully"),
                        Err(e) => error!("Webhook failed: {e}"),
                    }
                } else {
                    warn!("Tag DENIED: {}", tag_str);
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

// ── IFTTT webhook ──────────────────────────────────────────────────────────

fn trigger_ifttt(tag_str: &str) -> Result<()> {
    let url = format!(
        "https://maker.ifttt.com/trigger/{}/with/key/{}",
        IFTTT_EVENT, IFTTT_KEY
    );

    // JSON payload with the tag ID as value1
    let body = format!(r#"{{"value1":"{}"}}"#, tag_str);
    let content_length = body.len().to_string();

    let headers = [
        ("Content-Type", "application/json"),
        ("Content-Length", content_length.as_str()),
    ];

    let mut client = HttpClient::wrap(EspHttpConnection::new(&HttpConfig {
        crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
        ..Default::default()
    })?);

    let mut request = client.post(&url, &headers)?;
    request.write_all(body.as_bytes())?;
    request.flush()?;

    let response = request.submit()?;
    let status = response.status();

    if !(200..300).contains(&(status as i32)) {
        bail!("IFTTT returned HTTP {status}");
    }
    Ok(())
}

// ── Allowlist check ────────────────────────────────────────────────────────

fn is_allowed(tag: &TagId) -> bool {
    ALLOWED_TAGS.iter().any(|allowed| allowed == tag)
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
