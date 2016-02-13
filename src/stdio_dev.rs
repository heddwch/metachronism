use super::ConcurrentDevice;

use z80e_core_rust::{ Z80IODevice };

use std::sync::{ Arc, Condvar, Mutex };
use std::sync::atomic::{ AtomicBool, AtomicUsize, Ordering };
use std::io::{ self, Read, Write };
use std::thread;

const STATUS_READY_READ: usize = 1;
const STATUS_READY_WRITE: usize = 2;

const BUF_LENGTH: usize = 0x100;

pub struct StdioControl {
    device: StdioDevice,
}

impl StdioControl {
    fn new(device: StdioDevice) -> StdioControl {
        StdioControl {
            device: device,
        }
    }
}

impl Z80IODevice for StdioControl {
    fn read_in(&self) -> u8 {
        (self.device.status.load(Ordering::SeqCst) & 0xff) as u8
    }
    fn write_out(&mut self, value: u8) {
    }
}

pub struct StdioData {
    device: StdioDevice,
}

impl StdioData {
    fn new(device: StdioDevice) -> StdioData {
        StdioData {
            device: device,
        }
    }
}

impl Z80IODevice for StdioData {
    fn read_in(&self) -> u8 {
        let status = self.device.status.load(Ordering::SeqCst);
        if (status & STATUS_READY_READ) != 0 {
            let mut buffer = match self.device.read_buffer.lock() {
                Ok(x) => x,
                Err(err) => {
                    panic!("StdioDevice buffer mutex poisoned: {}", err);
                }
            };
            if buffer.len() == 1 {
                self.device.status.fetch_and(!STATUS_READY_READ, Ordering::SeqCst);
            }
            let byte = buffer.remove(0);
            byte
        } else {
            let _ = writeln!(io::stderr(), "Attempted to read StdioDevice when it wasn't ready.");
            0
        }
    }
    fn write_out(&mut self, value: u8) {
        let status = self.device.status.load(Ordering::SeqCst);
        if (status & STATUS_READY_WRITE) != 0 {
            let mut buffer = match self.device.writer_state.buffer.lock() {
                Ok(x) => x,
                Err(err) => {
                    panic!("StdioDevice buffer mutex poisoned: {}", err);
                },
            };
            buffer.push(value);
            let mut have_data = self.device.writer_state.have_data.lock().unwrap();
            self.device.status.fetch_and(!STATUS_READY_WRITE, Ordering::SeqCst);
            *have_data = true;
            self.device.writer_state.have_data_cond.notify_one();
        } else {
            let _ = writeln!(io::stderr(), "StdioDevice written to when not ready.");
        }
    }
}

pub struct StdioReader {
    device: StdioDevice,
    stdin: io::Stdin,
}

impl StdioReader {
    fn new(device: StdioDevice) -> StdioReader {
        StdioReader {
            device: device,
            stdin: io::stdin(),
        }
    }
}

impl ConcurrentDevice for StdioReader {
    fn run(&mut self, die: Arc<AtomicBool>) {
        let mut buf: [u8; BUF_LENGTH] = [0; BUF_LENGTH];
        loop {
            if die.load(Ordering::Acquire) { break };
            let count = match self.stdin.read(&mut buf) {
                Ok(x) => x,
                Err(err) => {
                    let _ = writeln!(io::stderr(), "Error reading from stdin: {}", err);
                    0
                },
            };
            if count > 0 {
                let mut buffer = match self.device.read_buffer.lock() {
                    Ok(x) => x,
                    Err(err) => panic!("StdioDevice read buffer mutex poisoned: {}", err),
                };
                for i in 0..count {
                    buffer.push(buf[i]);
                }
                self.device.status.fetch_or(STATUS_READY_READ, Ordering::SeqCst);
            }
        }
    }
}

pub struct StdioWriter {
    device: StdioDevice,
    stdout: io::Stdout,
}

impl StdioWriter {
    fn new(device: StdioDevice) -> StdioWriter {
        StdioWriter {
            device: device,
            stdout: io::stdout(),
        }
    }
}

impl ConcurrentDevice for StdioWriter {
    fn run(&mut self, die: Arc<AtomicBool>) {
        let mut have_data = self.device.writer_state.have_data.lock().unwrap();
        self.device.status.fetch_or(STATUS_READY_WRITE, Ordering::SeqCst);
        loop {
            while !*have_data {
                have_data = self.device.writer_state.have_data_cond.wait(have_data).unwrap();
            }
            if die.load(Ordering::Acquire) { break };
            let mut buffer = self.device.writer_state.buffer.lock().unwrap();
            let mut buf: [u8; BUF_LENGTH] = [0; BUF_LENGTH];
            loop {
                let mut count = 0;
                {
                    let mut items = buffer.iter();
                    for i in 0..BUF_LENGTH {
                        match items.next() {
                            Some(x) => {
                                buf[i] = *x;
                                count += 1;
                            },
                            None => {
                                break
                            },
                        }
                    };
                }
                match self.stdout.write_all(&buf[..count]) {
                    Ok(()) => {
                        if buffer.len() >= count{
                            *buffer = buffer[count..].to_vec();
                        } else {
                            *buffer = Vec::new();
                        }
                    },
                    Err(err) => {
                        let _ = writeln!(io::stderr(), "Error writing to stdout: {}", err);
                    },
                }
                self.stdout.flush().unwrap();
                if count == 0 { break; }
            }
            *have_data = false;
            self.device.status.fetch_or(STATUS_READY_WRITE, Ordering::SeqCst);
        }
    }
}

struct StdioWriterState {
    buffer: Mutex<Vec<u8>>,
    have_data: Mutex<bool>,
    have_data_cond: Condvar,
}

impl StdioWriterState {
    fn new() -> StdioWriterState {
        StdioWriterState {
            buffer: Mutex::new(Vec::new()),
            have_data: Mutex::new(false),
            have_data_cond: Condvar::new(),
        }
    }
}

#[derive(Clone)]
pub struct StdioDevice {
    pub status: Arc<AtomicUsize>,
    read_buffer: Arc<Mutex<Vec<u8>>>,
    writer_state: Arc<StdioWriterState>,
}

impl StdioDevice {
    pub fn new() -> StdioDevice {
        StdioDevice {
            status: Arc::new(AtomicUsize::new(0)),
            read_buffer: Arc::new(Mutex::new(Vec::new())),
            writer_state: Arc::new(StdioWriterState::new()),
        }
    }
    pub fn get_control_port(&self) -> StdioControl {
        StdioControl::new(self.clone())
    }
    pub fn get_data_port(&self) -> StdioData {
        StdioData::new(self.clone())
    }
    pub fn get_reader(&self) -> StdioReader {
        StdioReader::new(self.clone())
    }
    pub fn get_writer(&self) -> StdioWriter {
        StdioWriter::new(self.clone())
    }
}

