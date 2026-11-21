//! The serprog protocol, as flashrom's description of it defines it: the
//! commands a host sends, their parameters and the answers the programmer
//! gives.
//!
//! The crate knows no hardware. It reads bytes, decides what each command
//! asks, and writes the answer through traits the firmware implements over
//! its USB link and its SPI bus, so that every command is tested on the host.

#![no_std]
