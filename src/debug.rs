use z80e_core_rust::{ self as z80, IoDevice };
use std::io::{ self, Stdout, Write };

pub struct DebugDevice;

impl DebugDevice {
    pub fn new() -> Self {
        DebugDevice
    }
}

impl IoDevice for DebugDevice {
    fn read_in(&self) -> u8 {
        0
    }
    fn write_out(&mut self, byte: u8) {
        println!("Debug device written to: {:02X}", byte);
    }
}
