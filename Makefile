# flashclip: a serprog SPI flash programmer for the ESP32-C3 SuperMini.
# The firmware builds from firmware/, whose .cargo/config.toml sets the chip's
# target; the protocol builds and is tested on the host, from here. The chip
# on the clip is read and written by flashrom, on the host, through the board.

SHELL    := /bin/bash

FIRMWARE := target/riscv32imc-unknown-none-elf/release/flashclip

PORT     ?= /dev/ttyACM0
SPEED    ?= 1M
FLASHROM ?= flashrom

# The baud rate means nothing to the USB Serial/JTAG link; flashrom wants one.
PROGRAMMER = serprog:dev=$(PORT):115200,spispeed=$(SPEED)
CHIP_ARG   = $(if $(CHIP),-c "$(CHIP)")

.PHONY: help build test check clean board flash probe read compare write block

help: ## list the targets
	@grep -E '^[a-z]+:.*## ' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  make %-8s %s\n", $$1, $$2}'

build: ## the firmware: target/riscv32imc-unknown-none-elf/release/flashclip
	cd firmware && cargo build --release

test: ## the protocol's tests on the host, flashrom's among them
	cargo test -p flashclip-protocol

check: ## format, lints and tests, the check every commit passes
	cargo fmt --all --check
	cargo clippy -p flashclip-protocol --all-targets -- -D warnings
	cd firmware && cargo clippy --release -- -D warnings
	cargo test -p flashclip-protocol

clean: ## remove what cargo built
	cargo clean

board: ## check that PORT is the board: an ESP32-C3's USB Serial/JTAG, 303a:1001
	@props=$$(udevadm info -q property -n "$(PORT)" 2>/dev/null) || \
	  { echo "$(PORT): no such device; is the board plugged in?"; exit 2; }; \
	grep -qx 'ID_VENDOR_ID=303a' <<<"$$props" && grep -qx 'ID_MODEL_ID=1001' <<<"$$props" || \
	  { echo "$(PORT) is not the board (303a:1001):"; grep -E '^ID_(VENDOR|MODEL)(_ID)?=' <<<"$$props"; exit 2; }

flash: build board ## write the firmware to the board on PORT
	espflash flash --chip esp32c3 --port $(PORT) $(FIRMWARE)

probe: board ## ask flashrom for the programmer and the chip on the clip
	$(FLASHROM) -p $(PROGRAMMER) $(CHIP_ARG)

read: board ## read the chip into OUT: make read OUT=u52-4.bin [CHIP="EN25Q64"]
	@test -n "$(OUT)" || { echo "OUT= is required, e.g. make read OUT=u52-4.bin"; exit 2; }
	@test ! -e "$(OUT)" || { echo "$(OUT) exists: refusing to overwrite a dump"; exit 2; }
	$(FLASHROM) -p $(PROGRAMMER) $(CHIP_ARG) -r "$(OUT)"
	sha256sum "$(OUT)"

compare: ## the bytes that differ between two files: make compare A=u52-1.bin B=u52-2.bin
	@A="$(A)" B="$(B)" SHOW="$(SHOW)" scripts/compare.sh

write: board ## erase, write IMAGE, verify, read back: make write IMAGE=… A=… B=… READBACK=…
	@IMAGE="$(IMAGE)" A="$(A)" B="$(B)" READBACK="$(READBACK)" CONFIRM="$(CONFIRM)" FORCE="$(FORCE)" \
	  FLASHROM="$(FLASHROM)" PROGRAMMER="$(PROGRAMMER)" CHIP="$(CHIP)" scripts/write.sh

block: board ## read the 64 KB block holding ADDR N times: make block ADDR=0x220351 [N=10]
	@ADDR="$(ADDR)" N="$(N)" REF="$(REF)" OUT="$(OUT)" \
	  FLASHROM="$(FLASHROM)" PROGRAMMER="$(PROGRAMMER)" CHIP="$(CHIP)" scripts/block.sh
