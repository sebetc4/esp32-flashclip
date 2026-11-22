//! The serprog protocol, as flashrom's description of it defines it: the
//! commands a host sends, their parameters and the answers the programmer
//! gives.
//!
//! The crate knows no hardware. It reads bytes from a link, decides what each
//! command asks, drives a [`Bus`] and writes the answer back, so that every
//! command is tested on the host. The firmware binds the link to the USB
//! Serial/JTAG peripheral and the bus to the SPI peripheral.
//!
//! Only the SPI side of the protocol is served: the operation buffer and the
//! byte reads belong to parallel, LPC and FWH chips, and the command map does
//! not offer them.

#![no_std]

use embedded_io::{Read, ReadExactError, Write};

/// The answer to a command that succeeded, before its return bytes.
pub const ACK: u8 = 0x06;
/// The answer to a command that failed or that the programmer does not know.
pub const NAK: u8 = 0x15;

/// The commands of the protocol, by their opcode.
pub mod command {
    pub const NOP: u8 = 0x00;
    pub const Q_IFACE: u8 = 0x01;
    pub const Q_CMDMAP: u8 = 0x02;
    pub const Q_PGMNAME: u8 = 0x03;
    pub const Q_SERBUF: u8 = 0x04;
    pub const Q_BUSTYPE: u8 = 0x05;
    pub const Q_CHIPSIZE: u8 = 0x06;
    pub const Q_OPBUF: u8 = 0x07;
    pub const Q_WRNMAXLEN: u8 = 0x08;
    pub const R_BYTE: u8 = 0x09;
    pub const R_NBYTES: u8 = 0x0A;
    pub const O_INIT: u8 = 0x0B;
    pub const O_WRITEB: u8 = 0x0C;
    pub const O_WRITEN: u8 = 0x0D;
    pub const O_DELAY: u8 = 0x0E;
    pub const O_EXEC: u8 = 0x0F;
    pub const SYNCNOP: u8 = 0x10;
    pub const Q_RDNMAXLEN: u8 = 0x11;
    pub const S_BUSTYPE: u8 = 0x12;
    pub const O_SPIOP: u8 = 0x13;
    pub const S_SPI_FREQ: u8 = 0x14;
    pub const S_PIN_STATE: u8 = 0x15;
    pub const S_SPI_CS: u8 = 0x16;
    pub const S_SPI_MODE: u8 = 0x17;
    pub const S_CS_MODE: u8 = 0x18;

    /// The commands this programmer serves. The others are answered NAK.
    pub const SUPPORTED: [u8; 15] = [
        NOP,
        Q_IFACE,
        Q_CMDMAP,
        Q_PGMNAME,
        Q_SERBUF,
        Q_BUSTYPE,
        Q_WRNMAXLEN,
        SYNCNOP,
        Q_RDNMAXLEN,
        S_BUSTYPE,
        O_SPIOP,
        S_SPI_FREQ,
        S_PIN_STATE,
        S_SPI_CS,
        S_CS_MODE,
    ];
}

/// The version of the protocol, the answer to `Q_IFACE`.
pub const INTERFACE_VERSION: u16 = 1;

/// The programmer's name, the answer to `Q_PGMNAME`, padded with zeros to 16
/// bytes.
pub const NAME: &[u8] = b"flashclip";

/// The bus type bit of SPI, in `Q_BUSTYPE` and `S_BUSTYPE`.
pub const BUS_SPI: u8 = 1 << 3;

/// The serial buffer size the programmer reports. The link has flow control
/// (USB, or TCP in the tests), so the description asks for a large bogus
/// value.
pub const SERIAL_BUFFER: u16 = 0xFFFF;

/// The longest read one `O_SPIOP` may ask for, as `Q_RDNMAXLEN` reports it.
/// A read of any length is streamed through the buffer; this only sets how
/// often flashrom reports its progress.
pub const READ_MAX: u32 = 64 * 1024;

/// The SPI bus with one chip on it, as the protocol needs it.
///
/// The protocol drives the chip select, not the bus, so that a read longer
/// than the buffer stays one SPI transaction.
pub trait Bus {
    /// Drives the chip select low.
    fn select(&mut self);
    /// Drives the chip select high.
    fn deselect(&mut self);
    /// Clocks `data` out; what comes back is dropped.
    fn write(&mut self, data: &[u8]);
    /// Clocks `data.len()` bytes in.
    fn read(&mut self, data: &mut [u8]);
    /// Sets the clock to the fastest frequency the bus can make that does not
    /// exceed `hz`, or to its slowest when none is that slow, and returns it.
    /// `hz` is never zero.
    fn set_frequency(&mut self, hz: u32) -> u32;
    /// Drives the bus's pins, the chip select high, or releases them to high
    /// impedance so that another master can reach the chip.
    fn set_drivers(&mut self, enabled: bool);
}

/// How the chip select behaves, as `S_CS_MODE` sets it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CsMode {
    /// Selected for each `O_SPIOP`, deselected after it: the default.
    Auto,
    /// Held selected until another mode is set.
    Selected,
    /// Held deselected until another mode is set.
    Deselected,
}

/// Why serving a command stopped.
#[derive(Debug, PartialEq, Eq)]
pub enum Error<E> {
    /// The host closed the link.
    Closed,
    /// The link failed.
    Link(E),
}

impl<E> From<ReadExactError<E>> for Error<E> {
    fn from(error: ReadExactError<E>) -> Self {
        match error {
            ReadExactError::UnexpectedEof => Error::Closed,
            ReadExactError::Other(e) => Error::Link(e),
        }
    }
}

/// A serprog programmer over one bus, with a buffer of `N` bytes: the longest
/// `O_SPIOP` write it takes, and the size of the chunks a read is streamed in.
pub struct Programmer<B, const N: usize> {
    bus: B,
    buffer: [u8; N],
    cs_mode: CsMode,
    drivers: bool,
}

impl<B: Bus, const N: usize> Programmer<B, N> {
    /// A programmer whose pins stay released until the host drives them, by
    /// `S_PIN_STATE` or by its first SPI operation.
    pub fn new(bus: B) -> Self {
        const {
            assert!(N >= 2, "the buffer holds an answer's ACK and a byte");
            assert!(N <= 0xFF_FFFF, "Q_WRNMAXLEN reports the buffer on 24 bits");
        }
        Self {
            bus,
            buffer: [0; N],
            cs_mode: CsMode::Auto,
            drivers: false,
        }
    }

    /// The bus, for a test to look at what the commands did.
    pub fn bus(&mut self) -> &mut B {
        &mut self.bus
    }

    /// Reads one command from `link`, serves it and writes its answer.
    pub fn serve<L: Read + Write>(&mut self, link: &mut L) -> Result<(), Error<L::Error>> {
        let mut opcode = [0u8];
        link.read_exact(&mut opcode)?;
        self.dispatch(opcode[0], link)?;
        link.flush().map_err(Error::Link)
    }

    fn dispatch<L: Read + Write>(
        &mut self,
        opcode: u8,
        link: &mut L,
    ) -> Result<(), Error<L::Error>> {
        use command::*;
        match opcode {
            NOP => answer(link, &[]),
            Q_IFACE => answer(link, &INTERFACE_VERSION.to_le_bytes()),
            Q_CMDMAP => answer(link, &command_map()),
            Q_PGMNAME => {
                let mut name = [0u8; 16];
                name[..NAME.len()].copy_from_slice(NAME);
                answer(link, &name)
            }
            Q_SERBUF => answer(link, &SERIAL_BUFFER.to_le_bytes()),
            Q_BUSTYPE => answer(link, &[BUS_SPI]),
            Q_WRNMAXLEN => answer(link, &u24(N as u32)),
            Q_RDNMAXLEN => answer(link, &u24(READ_MAX)),
            SYNCNOP => write(link, &[NAK, ACK]),
            S_BUSTYPE => {
                let [buses] = params::<1, _>(link)?;
                if buses & BUS_SPI != 0 {
                    answer(link, &[])
                } else {
                    nak(link)
                }
            }
            S_SPI_FREQ => {
                let hz = u32::from_le_bytes(params(link)?);
                if hz == 0 {
                    return nak(link);
                }
                let set = self.bus.set_frequency(hz);
                answer(link, &set.to_le_bytes())
            }
            S_PIN_STATE => {
                let [state] = params::<1, _>(link)?;
                self.set_drivers(state != 0);
                answer(link, &[])
            }
            S_SPI_CS => {
                let [cs] = params::<1, _>(link)?;
                if cs == 0 {
                    answer(link, &[])
                } else {
                    nak(link)
                }
            }
            S_CS_MODE => {
                let [mode] = params::<1, _>(link)?;
                let mode = match mode {
                    0 => CsMode::Auto,
                    1 => CsMode::Selected,
                    2 => CsMode::Deselected,
                    _ => return nak(link),
                };
                self.set_cs_mode(mode);
                answer(link, &[])
            }
            O_SPIOP => self.spi_operation(link),
            _ => nak(link),
        }
    }

    /// `O_SPIOP`: writes `slen` bytes, then reads `rlen`, the chip selected
    /// throughout in the automatic mode. A write longer than the buffer is
    /// read off the link and refused, so that the next command starts at the
    /// right byte; a read of any length is streamed.
    fn spi_operation<L: Read + Write>(&mut self, link: &mut L) -> Result<(), Error<L::Error>> {
        let header: [u8; 6] = params(link)?;
        let slen = from_u24(&header[..3]) as usize;
        let rlen = from_u24(&header[3..]) as usize;

        if slen > N {
            let mut left = slen;
            while left > 0 {
                let n = left.min(N);
                link.read_exact(&mut self.buffer[..n])?;
                left -= n;
            }
            return nak(link);
        }
        link.read_exact(&mut self.buffer[..slen])?;

        if !self.drivers {
            self.set_drivers(true);
        }
        let auto = self.cs_mode == CsMode::Auto;
        if auto {
            self.bus.select();
        }
        let result = self.transfer(slen, rlen, link);
        if auto {
            self.bus.deselect();
        }
        result
    }

    /// Clocks out the `slen` bytes the buffer holds, then streams `rlen`
    /// bytes from the bus to the link, the ACK in front of them.
    fn transfer<L: Write>(
        &mut self,
        slen: usize,
        mut rlen: usize,
        link: &mut L,
    ) -> Result<(), Error<L::Error>> {
        self.bus.write(&self.buffer[..slen]);

        // The ACK goes out with the first chunk, in one write.
        self.buffer[0] = ACK;
        let first = rlen.min(N - 1);
        self.bus.read(&mut self.buffer[1..=first]);
        write(link, &self.buffer[..=first])?;
        rlen -= first;

        while rlen > 0 {
            let n = rlen.min(N);
            self.bus.read(&mut self.buffer[..n]);
            write(link, &self.buffer[..n])?;
            rlen -= n;
        }
        Ok(())
    }

    fn set_drivers(&mut self, enabled: bool) {
        self.bus.set_drivers(enabled);
        self.drivers = enabled;
        if enabled && self.cs_mode == CsMode::Selected {
            self.bus.select();
        }
    }

    /// Sets the mode, and the chip select's level when the pins are driven;
    /// released, they take it when driven again.
    fn set_cs_mode(&mut self, mode: CsMode) {
        self.cs_mode = mode;
        if !self.drivers {
            return;
        }
        match mode {
            CsMode::Selected => self.bus.select(),
            CsMode::Auto | CsMode::Deselected => self.bus.deselect(),
        }
    }
}

/// The answer to `Q_CMDMAP`: bit `n % 8` of byte `n / 8` set for each command
/// `n` served.
pub fn command_map() -> [u8; 32] {
    let mut map = [0u8; 32];
    for opcode in command::SUPPORTED {
        map[usize::from(opcode / 8)] |= 1 << (opcode % 8);
    }
    map
}

fn params<const P: usize, L: Read>(link: &mut L) -> Result<[u8; P], Error<L::Error>> {
    let mut bytes = [0u8; P];
    link.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// Writes ACK and `data`, at most 32 bytes, in one write: one USB packet.
fn answer<L: Write>(link: &mut L, data: &[u8]) -> Result<(), Error<L::Error>> {
    let mut bytes = [0u8; 33];
    bytes[0] = ACK;
    bytes[1..=data.len()].copy_from_slice(data);
    write(link, &bytes[..=data.len()])
}

fn nak<L: Write>(link: &mut L) -> Result<(), Error<L::Error>> {
    write(link, &[NAK])
}

fn write<L: Write>(link: &mut L, bytes: &[u8]) -> Result<(), Error<L::Error>> {
    link.write_all(bytes).map_err(Error::Link)
}

fn u24(value: u32) -> [u8; 3] {
    let [a, b, c, _] = value.to_le_bytes();
    [a, b, c]
}

fn from_u24(bytes: &[u8]) -> u32 {
    u32::from(bytes[0]) | u32::from(bytes[1]) << 8 | u32::from(bytes[2]) << 16
}
