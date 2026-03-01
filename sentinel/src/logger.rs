//! Dual-drain logger: writes to both the ESP-IDF serial console and a TCP
//! stream to panopticon (as `LOG: [LEVEL target] message\n`).

use std::io::Write;
use std::net::TcpStream;
use std::sync::Mutex;

use log::{Level, Log, Metadata, Record};

/// Shared TCP stream handle. `None` when not yet connected or after disconnect.
pub type TcpHandle = &'static Mutex<Option<TcpStream>>;

/// A logger that writes to two destinations:
/// 1. ESP-IDF serial output (always)
/// 2. A shared `TcpStream` to panopticon (when connected)
pub struct DualLogger {
    tcp: TcpHandle,
    serial: esp_idf_svc::log::EspLogger,
}

impl DualLogger {
    /// Create and register as the global logger. Returns the shared TCP handle
    /// so the caller can later store a connected `TcpStream` into it.
    pub fn init() -> TcpHandle {
        static TCP_STREAM: Mutex<Option<TcpStream>> = Mutex::new(None);

        let logger = Box::new(DualLogger {
            tcp: &TCP_STREAM,
            serial: esp_idf_svc::log::EspLogger::new(),
        });

        // Safety: we only call this once during init
        log::set_logger(Box::leak(logger)).expect("logger already set");
        log::set_max_level(log::LevelFilter::Info);

        &TCP_STREAM
    }
}

impl Log for DualLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Info
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        // Always write to serial
        self.serial.log(record);

        // Try to write to TCP (silently skip on failure to avoid recursion)
        if let Ok(mut guard) = self.tcp.try_lock() {
            if let Some(ref mut stream) = *guard {
                // Sanitize newlines so a single LOG line can't be split/injected
                let msg = format!(
                    "LOG: [{} {}] {}",
                    record.level(),
                    record.target(),
                    record.args()
                );
                let line = msg.replace('\r', "\\r").replace('\n', "\\n") + "\n";
                if stream.write_all(line.as_bytes()).is_err() {
                    // Connection lost — clear it so main loop can detect & reconnect
                    *guard = None;
                }
            }
        }
    }

    fn flush(&self) {
        if let Ok(mut guard) = self.tcp.try_lock() {
            if let Some(ref mut stream) = *guard {
                let _ = stream.flush();
            }
        }
    }
}
