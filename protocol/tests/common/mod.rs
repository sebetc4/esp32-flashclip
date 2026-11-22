//! What the tests drive the protocol with: a link over bytes or a stream, a
//! bus that records what it is asked, and a simulated SPI flash chip.

#![allow(dead_code, reason = "each test binary uses part of this module")]

use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use flashclip_protocol::Bus;

/// A link whose host sent `input`, all at once, then closed; the answers
/// gather in `output`.
pub struct Bytes {
    input: Vec<u8>,
    position: usize,
    pub output: Vec<u8>,
}

impl Bytes {
    pub fn new(input: &[u8]) -> Self {
        Self {
            input: input.to_vec(),
            position: 0,
            output: Vec::new(),
        }
    }
}

impl embedded_io::ErrorType for Bytes {
    type Error = Infallible;
}

impl embedded_io::Read for Bytes {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Infallible> {
        let n = buf.len().min(self.input.len() - self.position);
        buf[..n].copy_from_slice(&self.input[self.position..self.position + n]);
        self.position += n;
        Ok(n)
    }
}

impl embedded_io::Write for Bytes {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Infallible> {
        self.output.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}

/// A link over a `std::io` stream, a TCP connection.
pub struct Stream<T>(pub T);

impl<T> embedded_io::ErrorType for Stream<T> {
    type Error = std::io::Error;
}

impl<T: std::io::Read> embedded_io::Read for Stream<T> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl<T: std::io::Write> embedded_io::Write for Stream<T> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

/// What a [`Recorder`] was asked.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Select,
    Deselect,
    Write(Vec<u8>),
    Read(usize),
    Frequency(u32),
    Drivers(bool),
}

/// A bus that records each call, reads bytes counting up from 0, and makes
/// frequencies between 100 kHz and 40 MHz.
#[derive(Default)]
pub struct Recorder {
    pub events: Vec<Event>,
    counter: u8,
}

impl Bus for Recorder {
    fn select(&mut self) {
        self.events.push(Event::Select);
    }

    fn deselect(&mut self) {
        self.events.push(Event::Deselect);
    }

    fn write(&mut self, data: &[u8]) {
        self.events.push(Event::Write(data.to_vec()));
    }

    fn read(&mut self, data: &mut [u8]) {
        for byte in data.iter_mut() {
            *byte = self.counter;
            self.counter = self.counter.wrapping_add(1);
        }
        self.events.push(Event::Read(data.len()));
    }

    fn set_frequency(&mut self, hz: u32) -> u32 {
        self.events.push(Event::Frequency(hz));
        hz.clamp(100_000, 40_000_000)
    }

    fn set_drivers(&mut self, enabled: bool) {
        self.events.push(Event::Drivers(enabled));
    }
}

/// A Winbond W25Q80.V, 1 MiB, as flashrom drives it: identification, status
/// registers, reads, page programs and erases, each operation finished at
/// once. A program or an erase runs at the rising edge of the chip select,
/// and only after a write enable, as on a real chip. Unknown commands read
/// 0xFF, the level of an undriven MISO.
pub struct Chip {
    pub memory: Arc<Mutex<Vec<u8>>>,
    pub frequency: u32,
    /// Whether the programmer drives the pins, shared with the test.
    pub drivers: Arc<AtomicBool>,
    status: [u8; 3],
    write_enabled: bool,
    selected: bool,
    command: Vec<u8>,
    clocked_in: usize,
}

pub const CHIP_SIZE: usize = 1 << 20;
const JEDEC_ID: [u8; 3] = [0xEF, 0x40, 0x14];

impl Chip {
    pub fn new(memory: Arc<Mutex<Vec<u8>>>) -> Self {
        assert_eq!(memory.lock().unwrap().len(), CHIP_SIZE);
        Self {
            memory,
            frequency: 0,
            drivers: Arc::new(AtomicBool::new(false)),
            status: [0; 3],
            write_enabled: false,
            selected: false,
            command: Vec::new(),
            clocked_in: 0,
        }
    }

    fn driven(&self) -> bool {
        self.drivers.load(Ordering::SeqCst)
    }

    fn address(&self) -> Option<usize> {
        let a = self.command.get(1..4)?;
        Some((usize::from(a[0]) << 16 | usize::from(a[1]) << 8 | usize::from(a[2])) % CHIP_SIZE)
    }

    fn output(&self, n: usize) -> u8 {
        let Some(&opcode) = self.command.first() else {
            return 0xFF;
        };
        let memory = self.memory.lock().unwrap();
        match opcode {
            0x9F => JEDEC_ID.get(n).copied().unwrap_or(0xFF),
            0x05 => self.status[0] | u8::from(self.write_enabled) << 1,
            0x35 => self.status[1],
            0x15 => self.status[2],
            0x03 if self.command.len() >= 4 => memory[(self.address().unwrap() + n) % CHIP_SIZE],
            0x0B if self.command.len() >= 5 => memory[(self.address().unwrap() + n) % CHIP_SIZE],
            0x90 if self.command.len() >= 4 => [0xEF, 0x13][n % 2],
            0xAB if self.command.len() >= 4 => 0x13,
            _ => 0xFF,
        }
    }

    fn execute(&mut self) {
        let Some(&opcode) = self.command.first() else {
            return;
        };
        match opcode {
            0x06 => self.write_enabled = true,
            0x04 => self.write_enabled = false,
            _ if !self.write_enabled => {}
            0x02 if self.command.len() > 4 => {
                let address = self.address().unwrap();
                let page = address & !0xFF;
                let mut memory = self.memory.lock().unwrap();
                for (i, byte) in self.command[4..].iter().enumerate() {
                    memory[page | ((address + i) & 0xFF)] &= byte;
                }
                self.write_enabled = false;
            }
            0x20 | 0x52 | 0xD8 if self.command.len() >= 4 => {
                let size = match opcode {
                    0x20 => 4 << 10,
                    0x52 => 32 << 10,
                    _ => 64 << 10,
                };
                let start = self.address().unwrap() & !(size - 1);
                self.memory.lock().unwrap()[start..start + size].fill(0xFF);
                self.write_enabled = false;
            }
            0x60 | 0xC7 => {
                self.memory.lock().unwrap().fill(0xFF);
                self.write_enabled = false;
            }
            0x01 if self.command.len() >= 2 => {
                self.status[0] = self.command[1] & 0xFC;
                if let Some(&s2) = self.command.get(2) {
                    self.status[1] = s2;
                }
                self.write_enabled = false;
            }
            0x31 if self.command.len() >= 2 => {
                self.status[1] = self.command[1];
                self.write_enabled = false;
            }
            0x11 if self.command.len() >= 2 => {
                self.status[2] = self.command[1];
                self.write_enabled = false;
            }
            _ => {}
        }
    }
}

impl Bus for Chip {
    fn select(&mut self) {
        assert!(self.driven(), "the chip is selected through released pins");
        self.selected = true;
        self.command.clear();
        self.clocked_in = 0;
    }

    fn deselect(&mut self) {
        if self.selected {
            self.execute();
        }
        self.selected = false;
    }

    fn write(&mut self, data: &[u8]) {
        assert!(self.driven(), "the chip is written through released pins");
        if self.selected {
            self.command.extend_from_slice(data);
        }
    }

    fn read(&mut self, data: &mut [u8]) {
        assert!(self.driven(), "the chip is read through released pins");
        for byte in data.iter_mut() {
            *byte = if self.selected {
                self.output(self.clocked_in)
            } else {
                0xFF
            };
            self.clocked_in += 1;
        }
    }

    fn set_frequency(&mut self, hz: u32) -> u32 {
        self.frequency = hz.clamp(100_000, 40_000_000);
        self.frequency
    }

    fn set_drivers(&mut self, enabled: bool) {
        self.drivers.store(enabled, Ordering::SeqCst);
    }
}

/// `len` bytes of a deterministic pseudo-random sequence, seeded by `seed`.
pub fn pattern(seed: u32, len: usize) -> Vec<u8> {
    let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state >> 24) as u8
        })
        .collect()
}
