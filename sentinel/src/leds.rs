//! LED driver for the RFIDuino Shield v1.2 access feedback LEDs.
//!
//! The shield has two LEDs:
//! - Red:   Arduino D8 → ESP32 GPIO21
//! - Green: Arduino D4 → ESP32 GPIO22

use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::gpio::{AnyOutputPin, Output, PinDriver};

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
        let _ = self.green.set_high();
        FreeRtos::delay_ms(duration_ms);
        let _ = self.green.set_low();
    }

    pub fn flash_red(&mut self, duration_ms: u32) {
        let _ = self.red.set_high();
        FreeRtos::delay_ms(duration_ms);
        let _ = self.red.set_low();
    }
}
