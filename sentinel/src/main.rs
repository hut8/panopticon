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

const WIFI_SSID: &str = env!("WIFI_SSID");
const WIFI_PASS: &str = env!("WIFI_PASS");
const PANOPTICON_URL: &str = env!("PANOPTICON_URL");
const SENTINEL_SECRET: &str = env!("SENTINEL_SECRET");

/// Cooldown between successful scans of the same tag (prevents rapid re-triggering).
const SCAN_COOLDOWN: Duration = Duration::from_secs(5);

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

    // Pin assignments — adjust to match your wiring from ESP32-C3 to RFIDuino Shield.
    // GPIO numbers are ESP32-C3 GPIOs, NOT Arduino pin numbers.
    let pins = peripherals.pins;
    let mut reader = RFIDuino::new(
        pins.gpio2.into(), // DEMOD_OUT (shield D3 pad)
        pins.gpio3.into(), // RDY_CLK  (shield D2 pad)
        pins.gpio4.into(), // SHD      (shield D7 pad)
        pins.gpio5.into(), // MOD      (shield D6 pad)
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
                Some((prev_tag, when)) => *prev_tag != tag || when.elapsed() >= SCAN_COOLDOWN,
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
    let body = format!(
        r#"{{"tag_id":"{}","secret":"{}"}}"#,
        tag_id, SENTINEL_SECRET
    );
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
