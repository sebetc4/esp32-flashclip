# flashclip

A serprog SPI flash programmer for the ESP32-C3 SuperMini, written in Rust:
`flashrom -p serprog` reads and writes a 3.3 V SPI flash chip through it, the
chip held by a clip.

**Status:** the protocol is written and passes flashrom's probe, read, write
and erase on a simulated chip; the firmware builds. Not yet tried on the
board.

## Build

Rust stable 1.95 or later, through rustup, which installs the chip's target
from `rust-toolchain.toml`. Flashing needs `espflash` (`cargo install espflash
--locked`, which needs libudev's development files: `systemd-devel` on Fedora).

```bash
make build   # the firmware
make test    # the protocol's tests, on the host, flashrom's among them
make check   # format, lints, tests
make flash   # the firmware onto the board, PORT=/dev/ttyACM0 by default
```

## Use

The board is checked before each command: `PORT` must be an ESP32-C3's USB
Serial/JTAG, `303a:1001`. `SPEED` is the SPI clock, `1M` by default; `CHIP`
names the chip when flashrom hesitates.

```bash
make probe                                  # the programmer and the chip on the clip
make read OUT=chip-1.bin                    # never overwrites a file
make compare A=chip-1.bin B=chip-2.bin      # each byte that differs, its address and values
make block ADDR=0x220351 N=10 [REF=…]       # the 64 KB block holding ADDR, read N times
make write IMAGE=… A=… B=… READBACK=…       # erase, write, verify, read back; asks for WRITE
```

`write` refuses unless `A` and `B` are two identical reads of the chip on the
clip and `IMAGE` has their size.

## Dry run without hardware

`protocol/examples/simulator.rs` serves the protocol on a local TCP port over
a simulated W25Q80.V (1 MiB), which flashrom reaches as
`serprog:ip=127.0.0.1:PORT`. The scripts take the programmer from the
environment, so a command can be tried on it first:

```bash
cargo run -p flashclip-protocol --example simulator -- 7700 &
flashrom -p serprog:ip=127.0.0.1:7700 -r a.bin
flashrom -p serprog:ip=127.0.0.1:7700 -r b.bin
PROGRAMMER=serprog:ip=127.0.0.1:7700 IMAGE=… A=a.bin B=b.bin READBACK=… scripts/write.sh
```

## Wiring

CS 10, MISO 5, MOSI 7, CLK 6. The board drives the pins only while flashrom
runs: before and after, they are inputs, at high impedance.

- 3.3 V only: never the SuperMini's 5V pin on the clip, never a 1.8 V chip
  without a level shifter. Measure the clip's teeth before it goes on a chip:
  nothing above 3.6 V.
- In-circuit, the target board has no power at all: mains, battery, coin cell.
  The clip's VCC comes from a separate 3.3 V supply, its ground joined to the
  SuperMini's G; the SuperMini's 3.3 pin stays free, so that two regulators
  never share a rail.
- Another pin choice avoids the strapping pins (2, 8, 9), USB (18, 19) and the
  module's flash (11 to 17).
