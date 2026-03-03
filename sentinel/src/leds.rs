//! LED driver for the RFIDuino Shield v1.2 access feedback LEDs.
//!
//! The shield has two LEDs:
//! - Red:   Arduino D8 → ESP32 GPIO21
//! - Green: Arduino D4 → ESP32 GPIO22

use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::gpio::{AnyOutputPin, Output, PinDriver};
use log::warn;

pub struct Leds<'a> {
    red: PinDriver<'a, AnyOutputPin, Output>,
    green: PinDriver<'a, AnyOutputPin, Output>,
}

impl<'a> Leds<'a> {
    pub fn new(red_pin: AnyOutputPin, green_pin: AnyOutputPin) -> anyhow::Result<Self> {
        let mut red = PinDriver::output(red_pin)?;
        let mut green = PinDriver::output(green_pin)?;
        red.set_low()?;
        green.set_low()?;
        Ok(Self { red, green })
    }

    pub fn flash_green(&mut self, duration_ms: u32) {
        if let Err(e) = self.green.set_high() {
            warn!("Failed to set green LED high: {e}");
        }
        FreeRtos::delay_ms(duration_ms);
        if let Err(e) = self.green.set_low() {
            warn!("Failed to set green LED low: {e}");
        }
    }

    pub fn flash_red(&mut self, duration_ms: u32) {
        if let Err(e) = self.red.set_high() {
            warn!("Failed to set red LED high: {e}");
        }
        FreeRtos::delay_ms(duration_ms);
        if let Err(e) = self.red.set_low() {
            warn!("Failed to set red LED low: {e}");
        }
    }
}
