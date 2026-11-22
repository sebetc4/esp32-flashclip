//! A programmer without hardware: the protocol served on a local TCP port over
//! a simulated W25Q80.V (1 MiB), for a dry run of the host scripts.
//!
//! ```text
//! cargo run -p flashclip-protocol --example simulator -- [PORT] [CONTENT]
//! flashrom -p serprog:ip=127.0.0.1:PORT -r read.bin
//! ```
//!
//! PORT defaults to 7700. CONTENT, a 1 MiB file, is what the chip holds at
//! start; a pseudo-random pattern otherwise. The chip keeps what it is
//! written from one flashrom run to the next, until the simulator stops.

#[path = "../tests/common/mod.rs"]
mod common;

use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use common::{CHIP_SIZE, Chip, Stream, pattern};
use flashclip_protocol::{Error, Programmer};

fn main() {
    let mut args = std::env::args().skip(1);
    let port: u16 = args
        .next()
        .map_or(7700, |p| p.parse().expect("PORT is a number"));
    let content = match args.next() {
        Some(path) => std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}")),
        None => pattern(1, CHIP_SIZE),
    };
    assert_eq!(
        content.len(),
        CHIP_SIZE,
        "CONTENT must hold {CHIP_SIZE} bytes"
    );

    let listener = TcpListener::bind(("127.0.0.1", port)).expect("the port is free");
    let mut programmer = Programmer::<_, 4096>::new(Chip::new(Arc::new(Mutex::new(content))));
    eprintln!("simulator: W25Q80.V on serprog:ip=127.0.0.1:{port}");
    for connection in listener.incoming() {
        let connection = connection.expect("a connection");
        connection.set_nodelay(true).expect("TCP_NODELAY");
        let mut link = Stream(connection);
        loop {
            match programmer.serve(&mut link) {
                Ok(()) => {}
                Err(Error::Closed) => break,
                Err(Error::Link(e)) => {
                    eprintln!("simulator: {e}");
                    break;
                }
            }
        }
    }
}
