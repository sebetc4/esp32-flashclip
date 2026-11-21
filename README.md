# flashclip

A serprog SPI flash programmer for the ESP32-C3 SuperMini, written in Rust:
`flashrom -p serprog` reads and writes a 3.3 V SPI flash chip through it, the
chip held by a clip.

**Status:** early. The firmware boots; the protocol is not written yet.

## Build

Rust stable 1.95 or later, through rustup, which installs the chip's target
from `rust-toolchain.toml`.

```bash
make build   # the firmware
make test    # the protocol's tests, on the host
make check   # format, lints, tests
```
