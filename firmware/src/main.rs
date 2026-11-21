//! flashclip's firmware for the ESP32-C3 SuperMini: it boots, and waits.
//!
//! The serprog commands arrive over the USB Serial/JTAG link and drive the
//! SPI bus; the protocol itself lives in `flashclip-protocol`.

#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is not safe with esp-hal types, especially those \
    holding buffers for the duration of a transfer"
)]

use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::main;

// The application descriptor the ESP-IDF bootloader requires.
esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let _peripherals = esp_hal::init(config);

    loop {
        core::hint::spin_loop();
    }
}
