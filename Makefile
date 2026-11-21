# flashclip: a serprog SPI flash programmer for the ESP32-C3 SuperMini.
# The firmware builds from firmware/, whose .cargo/config.toml sets the chip's
# target; the protocol builds and is tested on the host, from here.

FIRMWARE := target/riscv32imc-unknown-none-elf/release/flashclip

.PHONY: help build test check clean

help:
	@echo "make build   the firmware: $(FIRMWARE)"
	@echo "make test    the protocol's tests, on the host"
	@echo "make check   format, lints and tests, the check every commit passes"
	@echo "make clean   remove what cargo built"

build:
	cd firmware && cargo build --release

test:
	cargo test -p flashclip-protocol

check:
	cargo fmt --all --check
	cargo clippy -p flashclip-protocol --all-targets -- -D warnings
	cd firmware && cargo clippy --release -- -D warnings
	cargo test -p flashclip-protocol

clean:
	cargo clean
