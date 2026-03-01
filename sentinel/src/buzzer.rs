//! Piezo buzzer driver using the ESP32 LEDC (PWM) peripheral.
//!
//! The RFIDuino Shield v1.2 has a passive piezo buzzer on Arduino D5,
//! wired to ESP32 GPIO19. A passive piezo needs a square wave to
//! produce sound — we use the LEDC peripheral at 50% duty cycle and
//! vary the frequency for different notes.

use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::gpio::OutputPin;
use esp_idf_svc::hal::ledc::{config::TimerConfig, LedcDriver, LedcTimerDriver, CHANNEL0, TIMER0};
use esp_idf_svc::hal::peripheral::Peripheral;
use esp_idf_svc::hal::prelude::*;

/// A note: frequency in Hz and duration in ms. Frequency 0 = rest (silence).
struct Note(u32, u32);

/// Tetris Theme A (Korobeiniki) — first 4 measures.
const KOROBEINIKI: &[Note] = &[
    // Measure 1: E5 B4 C5 | D5 C5 B4
    Note(659, 300),
    Note(494, 150),
    Note(523, 150),
    Note(587, 300),
    Note(523, 150),
    Note(494, 150),
    // Measure 2: A4 A4 C5 | E5 D5 C5
    Note(440, 300),
    Note(440, 150),
    Note(523, 150),
    Note(659, 300),
    Note(587, 150),
    Note(523, 150),
    // Measure 3: B4. C5 | D5 E5
    Note(494, 450),
    Note(523, 150),
    Note(587, 300),
    Note(659, 300),
    // Measure 4: C5 A4 | A4—
    Note(523, 300),
    Note(440, 300),
    Note(440, 300),
    Note(0, 300),
];

/// Play the Tetris theme on the piezo buzzer at startup.
///
/// Takes ownership of the LEDC timer0, channel0, and the buzzer GPIO pin.
pub fn play_startup_melody(
    timer: TIMER0,
    channel: CHANNEL0,
    pin: impl Peripheral<P = impl OutputPin> + 'static,
) -> anyhow::Result<()> {
    // Start with an arbitrary frequency — we'll change it per note
    let mut timer_driver = LedcTimerDriver::new(
        timer,
        &TimerConfig::default().frequency(1000.Hz().into()),
    )?;

    let mut driver = LedcDriver::new(channel, &timer_driver, pin)?;
    let max_duty = driver.get_max_duty();

    for &Note(freq, duration_ms) in KOROBEINIKI {
        if freq == 0 {
            driver.set_duty(0)?;
        } else {
            timer_driver.set_frequency(Hertz(freq))?;
            driver.set_duty(max_duty / 2)?;
        }
        FreeRtos::delay_ms(duration_ms);
    }

    // Silence when done
    driver.set_duty(0)?;
    Ok(())
}
