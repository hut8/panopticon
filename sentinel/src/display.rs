//! ST7789 1.3" 320×320 SPI display driver for sentinel status UI.
//!
//! Pin assignments (directly wired to ESP32, no conflict with RFIDuino shield):
//!   SCLK  → GPIO5
//!   MOSI  → GPIO23
//!   CS    → GPIO26
//!   DC    → GPIO27
//!   RST   → GPIO25
//!   BL    → GPIO32

use embedded_graphics::mono_font::ascii::{FONT_10X20, FONT_6X10};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use esp_idf_svc::hal::delay::Ets;
use esp_idf_svc::hal::gpio::{AnyOutputPin, OutputPin, PinDriver};
use esp_idf_svc::hal::peripheral::Peripheral;
use esp_idf_svc::hal::spi::config::Config as SpiConfig;
use esp_idf_svc::hal::spi::{SpiDeviceDriver, SpiDriverConfig};
use esp_idf_svc::hal::units::FromValueType;
use log::warn;
use mipidsi::interface::SpiInterface;
use mipidsi::models::ST7789;
use mipidsi::options::{Orientation, Rotation};
use mipidsi::Builder;

/// SPI transfer buffer size in bytes. Larger = faster bulk draws.
const SPI_BUFFER_SIZE: usize = 4096;

/// Colors used in the UI.
const BG_COLOR: Rgb565 = Rgb565::BLACK;
const HEADER_COLOR: Rgb565 = Rgb565::new(0, 20, 31); // blue-ish
const TEXT_COLOR: Rgb565 = Rgb565::WHITE;
const DIM_COLOR: Rgb565 = Rgb565::new(16, 32, 16); // dim grey-green
const GREEN: Rgb565 = Rgb565::new(0, 63, 0);
const RED: Rgb565 = Rgb565::new(31, 0, 0);
const YELLOW: Rgb565 = Rgb565::new(31, 63, 0);

/// Display width and height after orientation is applied.
const WIDTH: u16 = 320;
const HEIGHT: u16 = 320;

/// Wrapper around the ST7789 display with a status UI.
pub struct StatusDisplay<'a> {
    display: mipidsi::Display<
        SpiInterface<'a, SpiDeviceDriver<'a>, PinDriver<'a, AnyOutputPin, esp_idf_svc::hal::gpio::Output>>,
        ST7789,
        PinDriver<'a, AnyOutputPin, esp_idf_svc::hal::gpio::Output>,
    >,
    backlight: PinDriver<'a, AnyOutputPin, esp_idf_svc::hal::gpio::Output>,
    // Cached state so we only redraw what changed
    ip_addr: heapless_string::HString,
    hostname: heapless_string::HString,
    server_connected: bool,
    wifi_connected: bool,
    last_tag: heapless_string::HString,
    last_result: heapless_string::HString,
    scan_count: u32,
    needs_full_redraw: bool,
}

/// Tiny fixed-capacity string to avoid heap allocation for cached display strings.
mod heapless_string {
    #[derive(Clone)]
    pub struct HString {
        buf: [u8; 64],
        len: usize,
    }

    impl HString {
        pub const fn new() -> Self {
            Self {
                buf: [0u8; 64],
                len: 0,
            }
        }

        pub fn as_str(&self) -> &str {
            // Safety: we only ever write valid UTF-8 via set()
            unsafe { core::str::from_utf8_unchecked(&self.buf[..self.len]) }
        }

        pub fn set(&mut self, s: &str) -> bool {
            let changed = self.as_str() != s;
            if changed {
                let copy_len = s.len().min(self.buf.len());
                self.buf[..copy_len].copy_from_slice(&s.as_bytes()[..copy_len]);
                self.len = copy_len;
            }
            changed
        }
    }
}

impl<'a> StatusDisplay<'a> {
    /// Initialize the ST7789 display over SPI.
    ///
    /// Takes ownership of the SPI bus and GPIO pins used for the display.
    pub fn new(
        spi: esp_idf_svc::hal::spi::SPI2,
        sclk: impl Peripheral<P = impl OutputPin> + 'a,
        mosi: impl Peripheral<P = impl OutputPin> + 'a,
        cs: AnyOutputPin,
        dc: AnyOutputPin,
        rst: AnyOutputPin,
        bl: AnyOutputPin,
        spi_buffer: &'a mut [u8; SPI_BUFFER_SIZE],
    ) -> anyhow::Result<Self> {
        let config = SpiConfig::new()
            .baudrate(40.MHz().into())
            .data_mode(embedded_hal::spi::MODE_3);

        let device = SpiDeviceDriver::new_single(
            spi,
            sclk,
            mosi,
            Option::<esp_idf_svc::hal::gpio::AnyIOPin>::None,
            Some(cs),
            &SpiDriverConfig::new(),
            &config,
        )?;

        let dc_pin = PinDriver::output(dc)?;
        let rst_pin = PinDriver::output(rst)?;
        let mut backlight = PinDriver::output(bl)?;

        let di = SpiInterface::new(device, dc_pin, spi_buffer);

        let mut delay = Ets;
        let display = Builder::new(ST7789, di)
            .display_size(WIDTH, HEIGHT)
            .orientation(Orientation::new().rotate(Rotation::Deg0))
            .reset_pin(rst_pin)
            .init(&mut delay)
            .map_err(|e| anyhow::anyhow!("Display init failed: {:?}", e))?;

        backlight.set_high()?;

        let mut s = Self {
            display,
            backlight,
            ip_addr: heapless_string::HString::new(),
            hostname: heapless_string::HString::new(),
            server_connected: false,
            wifi_connected: false,
            last_tag: heapless_string::HString::new(),
            last_result: heapless_string::HString::new(),
            scan_count: 0,
            needs_full_redraw: true,
        };

        s.draw_full();
        Ok(s)
    }

    // ── Public update methods ────────────────────────────────────────────

    /// Update the displayed IP address.
    pub fn set_ip(&mut self, ip: &str) {
        if self.ip_addr.set(ip) {
            self.draw_network_section();
        }
    }

    /// Update the displayed hostname.
    pub fn set_hostname(&mut self, name: &str) {
        if self.hostname.set(name) {
            self.draw_network_section();
        }
    }

    /// Update the WiFi connection status indicator.
    pub fn set_wifi_connected(&mut self, connected: bool) {
        if self.wifi_connected != connected {
            self.wifi_connected = connected;
            self.draw_network_section();
        }
    }

    /// Update the server connection status indicator.
    pub fn set_server_connected(&mut self, connected: bool) {
        if self.server_connected != connected {
            self.server_connected = connected;
            self.draw_network_section();
        }
    }

    /// Show the result of the last scan.
    pub fn set_last_scan(&mut self, tag_id: &str, result: &str) {
        self.last_tag.set(tag_id);
        self.last_result.set(result);
        self.scan_count += 1;
        self.draw_scan_section();
    }

    // ── Drawing helpers ─────────────────────────────────────────────────

    /// Full screen redraw.
    fn draw_full(&mut self) {
        if let Err(e) = self.display.clear(BG_COLOR) {
            warn!("Display clear failed: {:?}", e);
            return;
        }
        self.draw_header();
        self.draw_network_section();
        self.draw_scan_section();
        self.needs_full_redraw = false;
    }

    /// Draw the "SENTINEL" header bar.
    fn draw_header(&mut self) {
        let header_rect = Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, 36));
        let _ = header_rect
            .into_styled(PrimitiveStyle::with_fill(HEADER_COLOR))
            .draw(&mut self.display);

        let style = MonoTextStyle::new(&FONT_10X20, TEXT_COLOR);
        let _ = Text::new("SENTINEL", Point::new(100, 24), style).draw(&mut self.display);
    }

    /// Draw the network/connection status block (y: 50..160).
    fn draw_network_section(&mut self) {
        // Clear section background
        let section = Rectangle::new(Point::new(0, 46), Size::new(WIDTH as u32, 120));
        let _ = section
            .into_styled(PrimitiveStyle::with_fill(BG_COLOR))
            .draw(&mut self.display);

        let label_style = MonoTextStyle::new(&FONT_6X10, DIM_COLOR);
        let value_style = MonoTextStyle::new(&FONT_10X20, TEXT_COLOR);

        // WiFi status
        let _ = Text::new("WIFI", Point::new(10, 62), label_style).draw(&mut self.display);
        let wifi_color = if self.wifi_connected { GREEN } else { RED };
        let wifi_text = if self.wifi_connected {
            "Connected"
        } else {
            "Disconnected"
        };
        let ws = MonoTextStyle::new(&FONT_10X20, wifi_color);
        let _ = Text::new(wifi_text, Point::new(10, 82), ws).draw(&mut self.display);

        // IP / hostname
        let _ = Text::new("IP", Point::new(10, 106), label_style).draw(&mut self.display);
        let _ = Text::new(self.ip_addr.as_str(), Point::new(10, 126), value_style)
            .draw(&mut self.display);

        // Server status
        let _ = Text::new("SERVER", Point::new(10, 148), label_style).draw(&mut self.display);
        let srv_color = if self.server_connected { GREEN } else { YELLOW };
        let srv_text = if self.server_connected {
            "Connected"
        } else {
            "Waiting..."
        };
        let ss = MonoTextStyle::new(&FONT_10X20, srv_color);
        let _ = Text::new(srv_text, Point::new(10, 166), ss).draw(&mut self.display);
    }

    /// Draw the last scan result block (y: 180..310).
    fn draw_scan_section(&mut self) {
        // Clear section background
        let section = Rectangle::new(Point::new(0, 180), Size::new(WIDTH as u32, 140));
        let _ = section
            .into_styled(PrimitiveStyle::with_fill(BG_COLOR))
            .draw(&mut self.display);

        let label_style = MonoTextStyle::new(&FONT_6X10, DIM_COLOR);
        let value_style = MonoTextStyle::new(&FONT_10X20, TEXT_COLOR);

        // Divider
        let div = Rectangle::new(Point::new(10, 182), Size::new(300, 1));
        let _ = div
            .into_styled(PrimitiveStyle::with_fill(DIM_COLOR))
            .draw(&mut self.display);

        // Last scan
        let _ = Text::new("LAST SCAN", Point::new(10, 200), label_style).draw(&mut self.display);

        if self.last_tag.as_str().is_empty() {
            let dim_value = MonoTextStyle::new(&FONT_10X20, DIM_COLOR);
            let _ = Text::new("No scans yet", Point::new(10, 222), dim_value)
                .draw(&mut self.display);
        } else {
            let _ = Text::new(self.last_tag.as_str(), Point::new(10, 222), value_style)
                .draw(&mut self.display);

            // Result with color
            let _ = Text::new("RESULT", Point::new(10, 248), label_style).draw(&mut self.display);
            let result_str = self.last_result.as_str();
            let result_color = match result_str {
                "granted" | "enrolled" => GREEN,
                "denied" => RED,
                _ => YELLOW,
            };
            let rs = MonoTextStyle::new(&FONT_10X20, result_color);
            let _ = Text::new(result_str, Point::new(10, 268), rs).draw(&mut self.display);
        }

        // Scan count
        let _ = Text::new("TOTAL SCANS", Point::new(10, 296), label_style)
            .draw(&mut self.display);
        let mut count_buf = [0u8; 16];
        let count_str = format_u32(self.scan_count, &mut count_buf);
        let _ = Text::new(count_str, Point::new(10, 316), value_style).draw(&mut self.display);
    }
}

/// Format a u32 into a provided buffer, returning a &str. Avoids alloc.
fn format_u32(n: u32, buf: &mut [u8; 16]) -> &str {
    use core::fmt::Write;
    struct BufWriter<'a> {
        buf: &'a mut [u8],
        pos: usize,
    }
    impl<'a> core::fmt::Write for BufWriter<'a> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let bytes = s.as_bytes();
            let end = (self.pos + bytes.len()).min(self.buf.len());
            let copy_len = end - self.pos;
            self.buf[self.pos..end].copy_from_slice(&bytes[..copy_len]);
            self.pos = end;
            Ok(())
        }
    }
    let mut w = BufWriter { buf: &mut buf[..], pos: 0 };
    let _ = write!(w, "{}", n);
    let len = w.pos;
    core::str::from_utf8(&buf[..len]).unwrap_or("?")
}
