mod buzzer;
mod leds;
mod logger;
mod rfiduino;

use std::io::Write;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};
use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition};
use log::{error, info, warn};

use rfiduino::{format_tag_id, format_tag_id_hex, RFIDuino, TagId};

// ── Configuration ──────────────────────────────────────────────────────────

const WIFI_SSID: &str = env!("WIFI_SSID", "missing WIFI_SSID — copy sentinel/.env.example to sentinel/.env and fill in values");
const WIFI_PASS: &str = env!("WIFI_PASS", "missing WIFI_PASS — copy sentinel/.env.example to sentinel/.env and fill in values");
const PANOPTICON_HOST: &str = env!("PANOPTICON_HOST", "missing PANOPTICON_HOST — copy sentinel/.env.example to sentinel/.env and fill in values");
const PANOPTICON_PORT: &str = env!("PANOPTICON_PORT", "missing PANOPTICON_PORT — copy sentinel/.env.example to sentinel/.env and fill in values");
const SENTINEL_SECRET: &str = env!("SENTINEL_SECRET", "missing SENTINEL_SECRET — copy sentinel/.env.example to sentinel/.env and fill in values");
const SENTINEL_HOSTNAME: &str = env!("SENTINEL_HOSTNAME", "missing SENTINEL_HOSTNAME — copy sentinel/.env.example to sentinel/.env and fill in values");

/// Cooldown between successful scans of the same tag (prevents rapid re-triggering).
const SCAN_COOLDOWN: Duration = Duration::from_secs(5);

// ── Main ───────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    // ESP-IDF boilerplate
    esp_idf_svc::sys::link_patches();

    // Set up dual-drain logger (serial + TCP to panopticon)
    let tcp_handle = logger::DualLogger::init();

    info!("sentinel starting up");

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
    // Set hostname for mDNS/DHCP identification
    {
        use esp_idf_svc::handle::RawHandle;
        use std::ffi::CString;
        let hostname = CString::new(SENTINEL_HOSTNAME)
            .expect("SENTINEL_HOSTNAME must not contain NUL bytes");
        let netif = wifi.wifi().sta_netif();
        let err = unsafe {
            esp_idf_svc::sys::esp_netif_set_hostname(
                netif.handle(),
                hostname.as_ptr(),
            )
        };
        if err != esp_idf_svc::sys::ESP_OK {
            error!("Failed to set hostname: ESP error {err}");
        }
    }
    connect_wifi(&mut wifi)?;
    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;
    info!("WiFi connected — IP: {}", ip_info.ip);

    // ── Startup melody ────────────────────────────────────────────────────
    let pins = peripherals.pins;
    info!("Playing startup melody...");
    buzzer::play_startup_melody(
        peripherals.ledc.timer0,
        peripherals.ledc.channel0,
        pins.gpio19,
    )?;

    // ── RFID reader ────────────────────────────────────────────────────────
    info!("Initializing RFIDuino...");
    let mut reader = RFIDuino::new(
        pins.gpio13.into(), // DEMOD_OUT (shield D3 pad)
        pins.gpio14.into(), // RDY_CLK  (shield D2 pad)
        pins.gpio15.into(), // SHD      (shield D7 pad)
        pins.gpio18.into(), // MOD      (shield D6 pad)
    )?;
    info!("RFIDuino ready — scan a tag");

    // ── LEDs ──────────────────────────────────────────────────────────────
    let mut leds = leds::Leds::new(
        pins.gpio21.into(), // Red LED   (shield D8 pad)
        pins.gpio22.into(), // Green LED (shield D4 pad)
    )?;
    info!("LEDs initialized");

    // ── Connect to panopticon ─────────────────────────────────────────────
    connect_panopticon(tcp_handle);

    // ── Main loop ──────────────────────────────────────────────────────────
    let mut last_scan: Option<(TagId, std::time::Instant)> = None;
    let mut last_reconnect_check = std::time::Instant::now();
    const RECONNECT_INTERVAL: Duration = Duration::from_secs(30);

    loop {
        // Periodically ensure we're connected so logs resume without a scan
        if last_reconnect_check.elapsed() >= RECONNECT_INTERVAL {
            ensure_connected(tcp_handle);
            last_reconnect_check = std::time::Instant::now();
        }

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
                match send_scan(tcp_handle, &hex_id) {
                    Some(action) if action == "granted" || action == "enrolled" => {
                        leds.flash_green(500);
                    }
                    Some(action) if action == "denied" => {
                        leds.flash_red(500);
                    }
                    _ => {} // No response or unrecognized action — no LED feedback
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

// ── Panopticon TCP connection ─────────────────────────────────────────────

/// Guards against overlapping background connection attempts.
static CONNECTING: AtomicBool = AtomicBool::new(false);

/// Connect to panopticon (blocking). Resolves the host, opens a TCP socket,
/// sends the AUTHZ handshake, and stores the stream in the shared handle.
fn connect_panopticon(tcp_handle: logger::TcpHandle) {
    let addr = format!("{}:{}", PANOPTICON_HOST, PANOPTICON_PORT);
    info!("Connecting to panopticon at {addr}...");

    let sock_addr: std::net::SocketAddr = match addr.parse() {
        Ok(a) => a,
        Err(_) => {
            // Resolve hostname manually for non-IP addresses
            use std::net::ToSocketAddrs;
            match addr.to_socket_addrs() {
                Ok(mut addrs) => match addrs.next() {
                    Some(a) => a,
                    None => {
                        error!("DNS resolution returned no addresses for {addr}");
                        return;
                    }
                },
                Err(e) => {
                    error!("Failed to resolve {addr}: {e}");
                    return;
                }
            }
        }
    };

    match TcpStream::connect_timeout(&sock_addr, Duration::from_secs(10)) {
        Ok(mut stream) => {
            // Send authentication
            let authz = format!("AUTHZ: {}\n", SENTINEL_SECRET);
            if let Err(e) = stream.write_all(authz.as_bytes()) {
                error!("Failed to send AUTHZ: {e}");
                return;
            }

            info!("Connected to panopticon");

            // Store in shared handle (logger will start sending LOG messages)
            match tcp_handle.lock() {
                Ok(mut guard) => {
                    *guard = Some(stream);
                }
                Err(e) => {
                    error!("Failed to acquire TCP handle lock: {e}");
                }
            }
        }
        Err(e) => {
            error!("Failed to connect to panopticon: {e}");
        }
    }
}

/// Spawn a background thread to reconnect without blocking the main loop.
/// No-op if a connection attempt is already in progress.
fn connect_panopticon_nonblocking(tcp_handle: logger::TcpHandle) {
    if CONNECTING.swap(true, Ordering::SeqCst) {
        return; // already connecting
    }
    std::thread::spawn(move || {
        connect_panopticon(tcp_handle);
        CONNECTING.store(false, Ordering::SeqCst);
    });
}

/// Trigger a non-blocking reconnect if disconnected.
fn ensure_connected(tcp_handle: logger::TcpHandle) {
    let connected = tcp_handle
        .lock()
        .map(|g| g.is_some())
        .unwrap_or(false);

    if !connected {
        connect_panopticon_nonblocking(tcp_handle);
    }
}

/// Send a SCAN message over the TCP connection and wait for the RESULT
/// response. Returns the action string (e.g. "granted", "denied", "enrolled")
/// or `None` on timeout/error. If the write fails or no stream is available,
/// triggers a background reconnect for the next attempt.
fn send_scan(tcp_handle: logger::TcpHandle, tag_id: &str) -> Option<String> {
    let msg = format!("SCAN: {}\n", tag_id);

    match tcp_handle.lock() {
        Ok(mut guard) => {
            if let Some(ref mut stream) = *guard {
                if let Err(e) = stream.write_all(msg.as_bytes()) {
                    warn!("TCP write failed for SCAN {tag_id}: {e}");
                    *guard = None;
                    drop(guard);
                    connect_panopticon_nonblocking(tcp_handle);
                    return None;
                }

                // Read the RESULT response with a 2-second timeout.
                // Read byte-by-byte to avoid BufReader buffering issues.
                let prev_timeout = stream.read_timeout().ok().flatten();
                if let Err(e) = stream.set_read_timeout(Some(Duration::from_secs(2))) {
                    warn!("Failed to set read timeout for RESULT: {e}");
                    return None;
                }

                let mut response = Vec::with_capacity(64);
                let result = loop {
                    let mut byte = [0u8; 1];
                    match std::io::Read::read(stream, &mut byte) {
                        Ok(0) => break Err("connection closed"),
                        Ok(_) => {
                            if byte[0] == b'\n' {
                                break Ok(());
                            }
                            response.push(byte[0]);
                            if response.len() > 128 {
                                break Err("response too long");
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
                              || e.kind() == std::io::ErrorKind::TimedOut => {
                            break Err("timeout");
                        }
                        Err(_) => break Err("read error"),
                    }
                };

                // Restore previous timeout
                let _ = stream.set_read_timeout(prev_timeout);

                match result {
                    Ok(()) => {
                        let line = String::from_utf8_lossy(&response);
                        if let Some(action) = line.trim().strip_prefix("RESULT: ") {
                            info!("RESULT for {tag_id}: {action}");
                            Some(action.to_string())
                        } else {
                            warn!("Unexpected response from panopticon: {line}");
                            None
                        }
                    }
                    Err(reason) => {
                        warn!("Failed to read RESULT for {tag_id}: {reason}");
                        if reason == "connection closed" || reason == "response too long" {
                            *guard = None;
                            drop(guard);
                            connect_panopticon_nonblocking(tcp_handle);
                        }
                        None
                    }
                }
            } else {
                warn!("Cannot send SCAN: no TCP stream available");
                drop(guard);
                connect_panopticon_nonblocking(tcp_handle);
                None
            }
        }
        Err(e) => {
            error!("Cannot send SCAN: TCP lock poisoned: {e}");
            None
        }
    }
}
