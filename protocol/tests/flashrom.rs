//! flashrom itself, the programmer's real host, against the protocol: flashrom
//! reaches a serprog programmer over TCP as well as over a serial port, so the
//! test serves the protocol on a local socket, over a simulated chip, and runs
//! flashrom's probe, read, write and erase on it.
//!
//! Skipped, with a message, where flashrom is not installed.

mod common;

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use common::{CHIP_SIZE, Chip, Stream, pattern};
use flashclip_protocol::{Error, Programmer};

/// The firmware's buffer size.
const N: usize = 4096;

struct Server {
    port: u16,
    memory: Arc<Mutex<Vec<u8>>>,
    drivers: Arc<AtomicBool>,
}

/// Serves the protocol on a free local port, one flashrom run after another,
/// over a chip holding `content`.
fn serve(content: Vec<u8>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let memory = Arc::new(Mutex::new(content));
    let chip = Chip::new(memory.clone());
    let drivers = chip.drivers.clone();
    let mut programmer = Programmer::<_, N>::new(chip);
    thread::spawn(move || {
        for connection in listener.incoming() {
            let connection = connection.unwrap();
            connection.set_nodelay(true).unwrap();
            let mut link = Stream(connection);
            loop {
                match programmer.serve(&mut link) {
                    Ok(()) => {}
                    Err(Error::Closed) => break,
                    Err(Error::Link(e)) => panic!("link: {e}"),
                }
            }
        }
    });
    Server {
        port,
        memory,
        drivers,
    }
}

impl Server {
    /// Runs flashrom on this programmer with `args`, and returns its output;
    /// fails the test if flashrom fails.
    fn flashrom(&self, args: &[&str]) -> String {
        let programmer = format!("serprog:ip=127.0.0.1:{},spispeed=1M", self.port);
        let output = Command::new("flashrom")
            .args(["-p", &programmer])
            .args(args)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&output.stdout).into_owned()
            + &String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "flashrom {args:?} failed:\n{text}");
        assert!(
            !self.drivers.load(Ordering::SeqCst),
            "flashrom {args:?} left the pins driven"
        );
        text
    }

    fn memory(&self) -> Vec<u8> {
        self.memory.lock().unwrap().clone()
    }
}

fn flashrom_installed() -> bool {
    match Command::new("flashrom").arg("--version").output() {
        Ok(output) => output.status.success(),
        Err(_) => {
            eprintln!("flashrom is not installed: the test against flashrom is skipped");
            false
        }
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("flashrom");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

#[test]
fn flashrom_finds_the_programmer_and_the_chip() {
    if !flashrom_installed() {
        return;
    }
    let server = serve(pattern(1, CHIP_SIZE));
    let output = server.flashrom(&[]);
    assert!(
        output.contains("Programmer name is \"flashclip\""),
        "{output}"
    );
    assert!(output.contains("W25Q80.V"), "{output}");
}

#[test]
fn flashrom_reads_writes_and_erases_the_chip() {
    if !flashrom_installed() {
        return;
    }
    let original = pattern(2, CHIP_SIZE);
    let server = serve(original.clone());
    let chip = ["-c", "W25Q80.V"];

    let read = scratch("read.bin");
    server.flashrom(&[&chip[..], &["-r", read.to_str().unwrap()]].concat());
    assert!(
        std::fs::read(&read).unwrap() == original,
        "the read differs from the chip"
    );

    // The image keeps the chip's first half and changes the second, so that
    // flashrom erases and writes only part of the chip.
    let mut image = original.clone();
    image[CHIP_SIZE / 2..].copy_from_slice(&pattern(3, CHIP_SIZE / 2));
    let image_path = scratch("image.bin");
    std::fs::write(&image_path, &image).unwrap();
    let output = server.flashrom(&[&chip[..], &["-w", image_path.to_str().unwrap()]].concat());
    assert!(output.contains("VERIFIED"), "{output}");
    assert!(server.memory() == image, "the chip does not hold the image");

    let read_back = scratch("read-back.bin");
    server.flashrom(&[&chip[..], &["-r", read_back.to_str().unwrap()]].concat());
    assert!(
        std::fs::read(&read_back).unwrap() == image,
        "the read-back differs"
    );

    server.flashrom(&[&chip[..], &["-E"]].concat());
    assert!(
        server.memory().iter().all(|&b| b == 0xFF),
        "the chip is not blank"
    );
}
