fn main() {
    // esp-hal's linker script, which pulls in the chip's memory map.
    println!("cargo:rustc-link-arg=-Tlinkall.x");
}
