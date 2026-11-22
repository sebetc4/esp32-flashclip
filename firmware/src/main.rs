//! flashclip's firmware for the ESP32-C3 SuperMini: a serprog programmer.
//!
//! The commands arrive over the USB Serial/JTAG link and drive the chip on
//! the SPI bus; what each command does lives in `flashclip-protocol`. This
//! file only binds the protocol's bus to the SPI peripheral and its pins.

#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is not safe with esp-hal types, especially those \
    holding buffers for the duration of a transfer"
)]

use esp_backtrace as _;
use esp_hal::Blocking;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::main;
use esp_hal::peripherals::{GPIO5, GPIO6, GPIO7, GPIO10, SPI2};
use esp_hal::spi::Mode;
use esp_hal::spi::master::{Config, Spi};
use esp_hal::time::Rate;
use esp_hal::usb::usb_serial_jtag::UsbSerialJtag;
use flashclip_protocol::{Bus, Programmer};

// The application descriptor the ESP-IDF bootloader requires.
esp_bootloader_esp_idf::esp_app_desc!();

/// The longest SPI write one command takes, and the chunk a read streams in.
const BUFFER: usize = 4096;

/// The SPI peripheral's clock source, the APB clock.
const SOURCE_HZ: u32 = 80_000_000;
/// The fastest clock offered: the pins go through the GPIO matrix.
const MAX_HZ: u32 = 40_000_000;
/// The slowest clock the divider makes: the source divided by 16 × 64.
const MIN_HZ: u32 = SOURCE_HZ / 1024;
/// The clock until the host sets one.
const DEFAULT_HZ: u32 = 1_000_000;

/// The SPI peripheral and the clip's pins, the C firmware's wiring.
struct Pins {
    spi: SPI2<'static>,
    clk: GPIO6<'static>,
    miso: GPIO5<'static>,
    mosi: GPIO7<'static>,
    cs: GPIO10<'static>,
}

/// The drivers that hold the pins while the bus is driven.
struct Driven {
    spi: Spi<'static, Blocking>,
    cs: Output<'static>,
}

/// The protocol's bus over SPI2. Driving the pins makes the SPI driver and
/// the chip select's output; releasing them drops both and leaves every pin
/// an input without pull, at high impedance.
struct Board {
    pins: Pins,
    config: Config,
    driven: Option<Driven>,
}

impl Board {
    fn new(pins: Pins) -> Self {
        let mut board = Self {
            pins,
            config: Config::default()
                .with_mode(Mode::_0)
                .with_frequency(Rate::from_hz(DEFAULT_HZ)),
            driven: None,
        };
        board.release();
        board
    }

    fn drive(&mut self) {
        self.driven = None;
        let p = &self.pins;
        // SAFETY: each peripheral has one driver at a time: the previous ones
        // were dropped above, and `Pins` itself never drives anything.
        let (spi, clk, miso, mosi, cs) = unsafe {
            (
                p.spi.clone_unchecked(),
                p.clk.clone_unchecked(),
                p.miso.clone_unchecked(),
                p.mosi.clone_unchecked(),
                p.cs.clone_unchecked(),
            )
        };
        let spi = Spi::new(spi, self.config)
            .expect("the frequency was checked when it was set")
            .with_sck(clk)
            .with_miso(miso)
            .with_mosi(mosi);
        let cs = Output::new(cs, Level::High, OutputConfig::default());
        self.driven = Some(Driven { spi, cs });
    }

    fn release(&mut self) {
        // Dropping the SPI driver disconnects its outputs from the pins.
        self.driven = None;
        let p = &self.pins;
        let input = InputConfig::default().with_pull(Pull::None);
        // SAFETY: no driver holds these pins any more; each input driver only
        // sets its pin's mode and is dropped at once.
        unsafe {
            Input::new(p.clk.clone_unchecked(), input);
            Input::new(p.miso.clone_unchecked(), input);
            Input::new(p.mosi.clone_unchecked(), input);
            Input::new(p.cs.clone_unchecked(), input);
        }
    }
}

impl Bus for Board {
    fn select(&mut self) {
        if let Some(driven) = &mut self.driven {
            driven.cs.set_low();
        }
    }

    fn deselect(&mut self) {
        if let Some(driven) = &mut self.driven {
            driven.cs.set_high();
        }
    }

    fn write(&mut self, data: &[u8]) {
        if let Some(driven) = &mut self.driven
            && !data.is_empty()
        {
            driven.spi.write(data).expect("SPI write");
        }
    }

    fn read(&mut self, data: &mut [u8]) {
        if let Some(driven) = &mut self.driven
            && !data.is_empty()
        {
            driven.spi.read(data).expect("SPI read");
        }
    }

    fn set_frequency(&mut self, hz: u32) -> u32 {
        let hz = hz.clamp(MIN_HZ, MAX_HZ);
        self.config = self.config.with_frequency(Rate::from_hz(hz));
        if let Some(driven) = &mut self.driven {
            driven
                .spi
                .apply_config(&self.config)
                .expect("the frequency is within the divider's range");
        }
        SOURCE_HZ / divider(hz)
    }

    fn set_drivers(&mut self, enabled: bool) {
        if enabled {
            self.drive();
        } else {
            self.release();
        }
    }
}

/// The divider esp-hal picks for `hz`, between `MIN_HZ` and `MAX_HZ`: the
/// smallest `pre × n`, `pre` up to 16 and `n` from 2 to 64, that does not run
/// the bus faster than asked.
fn divider(hz: u32) -> u32 {
    let wanted = SOURCE_HZ.div_ceil(hz).max(2);
    if wanted <= 64 {
        return wanted;
    }
    let mut best = 16 * 64;
    for pre in wanted.div_ceil(64)..=16 {
        let made = pre * wanted.div_ceil(pre);
        best = best.min(made);
        if made == wanted {
            break;
        }
    }
    best
}

#[main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let board = Board::new(Pins {
        spi: peripherals.SPI2,
        clk: peripherals.GPIO6,
        miso: peripherals.GPIO5,
        mosi: peripherals.GPIO7,
        cs: peripherals.GPIO10,
    });
    let mut programmer = Programmer::<_, BUFFER>::new(board);
    let mut usb = UsbSerialJtag::new(peripherals.USB_DEVICE);

    loop {
        // The USB link never closes, and its errors are those of a host gone
        // away: the next command is waited for in both cases.
        let _ = programmer.serve(&mut usb);
    }
}
