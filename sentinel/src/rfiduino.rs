//! Rust port of the RFIDuino Library v1.2 by TrossenRobotics / RobotGeek.
//!
//! Decodes 125kHz EM4100/EM4102 RFID tags via the EM4095 reader chip on the
//! RFIDuino Shield. The EM4095 handles all analog RF; this module reads its
//! digital `demod_out` line and Manchester-decodes the 64-bit tag frame into
//! a 5-byte tag ID.

use esp_idf_svc::hal::delay::Ets;
use esp_idf_svc::hal::gpio::{AnyInputPin, AnyOutputPin, Input, Output, PinDriver};

/// Manchester decode bit period in microseconds.
const DELAY_VAL: u32 = 320;

/// Timeout loop count for waiting on signal transitions.
const TIMEOUT: u16 = 1000;

/// A 5-byte EM4100 tag ID.
pub type TagId = [u8; 5];

/// Driver for the RFIDuino Shield v1.2, communicating with the EM4095 chip.
pub struct RFIDuino<'a> {
    demod_out: PinDriver<'a, AnyInputPin, Input>,
    shd: PinDriver<'a, AnyOutputPin, Output>,
    mod_pin: PinDriver<'a, AnyOutputPin, Output>,
    _rdy_clk: PinDriver<'a, AnyInputPin, Input>,
    scan_buffer: TagId,
    read_count: u8,
}

impl<'a> RFIDuino<'a> {
    /// Create a new RFIDuino driver.
    ///
    /// `demod_out` and `rdy_clk` are inputs from the EM4095.
    /// `shd` (shutdown) and `mod_pin` (modulation) are outputs held LOW for reading.
    pub fn new(
        demod_out: AnyInputPin,
        rdy_clk: AnyInputPin,
        shd: AnyOutputPin,
        mod_pin: AnyOutputPin,
    ) -> anyhow::Result<Self> {
        let demod_out = PinDriver::input(demod_out)?;
        let _rdy_clk = PinDriver::input(rdy_clk)?;
        let mut shd = PinDriver::output(shd)?;
        let mut mod_pin = PinDriver::output(mod_pin)?;

        // Hold SHD and MOD low to enable continuous reading
        shd.set_low()?;
        mod_pin.set_low()?;

        Ok(Self {
            demod_out,
            shd,
            mod_pin,
            _rdy_clk,
            scan_buffer: [0u8; 5],
            read_count: 0,
        })
    }

    /// Attempt to decode a single tag frame from the EM4095 demod output.
    ///
    /// Returns `Some(tag_id)` if a valid EM4100 frame (with correct parity) was
    /// received, or `None` on timeout / parity failure.
    ///
    /// This is a direct port of the C++ `decodeTag()` function. It busy-waits
    /// on GPIO transitions with microsecond timing — do not call from an async
    /// context or with interrupts that take >100µs.
    pub fn decode_tag(&self) -> Option<TagId> {
        let mut buf = [0u8; 5];

        // Wait for demod_out to go LOW (start of transmission)
        let mut time_count: u16 = 0;
        while self.demod_out.is_low() {
            if time_count >= TIMEOUT {
                break;
            }
            time_count += 1;
        }
        if time_count >= 600 {
            return None;
        }

        // Delay one bit period then check for HIGH
        Ets::delay_us(DELAY_VAL);
        if !self.demod_out.is_high() {
            return None;
        }

        // Read 8 header bits (should all be 1 in Manchester encoding)
        let mut header_ok = true;
        let mut i = 0u8;
        while i < 8 {
            time_count = 0;
            while self.demod_out.is_high() {
                if time_count == TIMEOUT {
                    header_ok = false;
                    break;
                }
                time_count += 1;
            }
            if !header_ok {
                break;
            }
            Ets::delay_us(DELAY_VAL);
            if self.demod_out.is_low() {
                break;
            }
            i += 1;
        }

        if !header_ok {
            return None;
        }
        if i != 8 {
            return None;
        }

        // All 8 header bits received — now read the data payload
        // Wait for current HIGH to end
        time_count = 0;
        while self.demod_out.is_high() {
            if time_count == TIMEOUT {
                return None;
            }
            time_count += 1;
        }

        // Read 11 rows × 5 columns (10 data rows + 1 parity row, 4 data cols + 1 parity col)
        let mut col_parity = [0u8; 5];

        for row in 0..11u8 {
            let mut row_parity: u8 = 0;
            let j = (row >> 1) as usize;

            for col in 0..5u8 {
                Ets::delay_us(DELAY_VAL);
                let dat: u8 = if self.demod_out.is_high() { 1 } else { 0 };

                // Store data bits (not parity column, not parity row)
                if col < 4 && row < 10 {
                    buf[j] <<= 1;
                    buf[j] |= dat;
                }

                row_parity += dat;
                col_parity[col as usize] += dat;

                // Wait for signal transition
                time_count = 0;
                let current = dat != 0;
                while self.demod_out.is_high() == current {
                    if time_count == TIMEOUT {
                        return None;
                    }
                    time_count += 1;
                }
            }

            // Check row parity (even parity for data rows)
            if row < 10 && (row_parity & 0x01) != 0 {
                return None;
            }
        }

        // Check column parity
        if (col_parity[0] & 0x01) != 0
            || (col_parity[1] & 0x01) != 0
            || (col_parity[2] & 0x01) != 0
            || (col_parity[3] & 0x01) != 0
        {
            return None;
        }

        Some(buf)
    }

    /// Scan for a tag with double-read verification (anti-ghosting).
    ///
    /// Requires two consecutive identical reads to return a tag ID. This prevents
    /// spurious single-read false positives. Call this in a loop.
    pub fn scan_for_tag(&mut self) -> Option<TagId> {
        let tag_data = self.decode_tag()?;

        self.read_count += 1;

        if self.read_count == 1 {
            // First read — store in buffer for verification
            self.scan_buffer = tag_data;
            None
        } else if self.read_count >= 2 {
            self.read_count = 0;
            // Second read — compare against buffer
            if tag_data == self.scan_buffer {
                Some(tag_data)
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Reset the double-read verification state.
    pub fn reset_scan(&mut self) {
        self.read_count = 0;
        self.scan_buffer = [0u8; 5];
    }

    /// Shut down the EM4095 reader (low power mode).
    pub fn shutdown(&mut self) -> anyhow::Result<()> {
        self.shd.set_high()?;
        Ok(())
    }

    /// Wake the EM4095 reader from shutdown.
    pub fn wake(&mut self) -> anyhow::Result<()> {
        self.shd.set_low()?;
        Ok(())
    }
}

/// Format a 5-byte tag ID as a human-readable string: "128,0,72,35,76"
pub fn format_tag_id(tag: &TagId) -> String {
    format!("{},{},{},{},{}", tag[0], tag[1], tag[2], tag[3], tag[4])
}

/// Format a 5-byte tag ID as colon-separated uppercase hex: "80:00:48:23:4C"
pub fn format_tag_id_hex(tag: &TagId) -> String {
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        tag[0], tag[1], tag[2], tag[3], tag[4]
    )
}

/// Convert the 4 data bytes of a tag ID (bytes 1-4) to a u32.
/// Byte 0 is the manufacturer/version byte and is excluded.
pub fn tag_id_to_u32(tag: &TagId) -> u32 {
    ((tag[1] as u32) << 24) | ((tag[2] as u32) << 16) | ((tag[3] as u32) << 8) | (tag[4] as u32)
}
