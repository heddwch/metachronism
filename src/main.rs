extern crate z80e_core_rust;
extern crate goss;
mod mmu;
use z80e_core_rust::{ Z80, Z80IODevice };
use mmu::{ Memory, MMU };
use std::io::{ self, Read, Write };
use std::env;
use std::str::FromStr;
use std::fs::File;
use std::sync::{ Arc, Condvar, Mutex };
use std::sync::atomic::{ AtomicBool, AtomicUsize, Ordering };
use std::thread;

trait ConcurrentDevice {
    fn run(&mut self, die: Arc<AtomicBool>);
}

const STATUS_READY_READ: usize = 1;
const STATUS_READY_WRITE: usize = 2;

const BUF_LENGTH: usize = 0x100;

struct StdioControl {
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

struct StdioData {
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

struct StdioReader {
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

struct StdioWriter {
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
struct StdioDevice {
    pub status: Arc<AtomicUsize>,
    read_buffer: Arc<Mutex<Vec<u8>>>,
    writer_state: Arc<StdioWriterState>,
}

impl StdioDevice {
    fn new() -> StdioDevice {
        StdioDevice {
            status: Arc::new(AtomicUsize::new(0)),
            read_buffer: Arc::new(Mutex::new(Vec::new())),
            writer_state: Arc::new(StdioWriterState::new()),
        }
    }
    fn get_control_port(&self) -> StdioControl {
        StdioControl::new(self.clone())
    }
    fn get_data_port(&self) -> StdioData {
        StdioData::new(self.clone())
    }
    fn get_reader(&self) -> StdioReader {
        StdioReader::new(self.clone())
    }
    fn get_writer(&self) -> StdioWriter {
        StdioWriter::new(self.clone())
    }
}

struct BankImage {
    name: String,
    bank: usize,
    file: File,
}

const NUM_BANKS: u8 = 1;

fn main() {
    let mut num_banks = NUM_BANKS;
    let mut stderr = std::io::stderr();
    let memory;
    let mut mmu;
    {
        let mut images: Vec<BankImage> = Vec::new();
        match goss::getopt(env::args(), "l:n:") {
            Ok(got_opt) => {
                for opt in got_opt.opts {
                    match opt.switch {
                        'n' => {
                            let arg = opt.argument.unwrap();
                            num_banks = match usize::from_str(&arg[..]) {
                                Ok(x) => {
                                    if x > u8::max_value() as usize {
                                        let _ = writeln!(stderr, "-n: Bad argument: {}", arg);
                                        panic!("Bank too big. (max: {})", u8::max_value() as usize);
                                    }
                                    x as u8
                                },
                                Err(err) => {
                                    let _ = writeln!(stderr, "-n: Bad argument: {} ← {}", arg, err);
                                    panic!("Unable to comprehend number.")
                                }
                            }
                        },
                        'l' => {
                            let mut bank = 0;
                            let arg = opt.argument.unwrap();
                            let subopts: Vec<&str> = arg.split('=').collect();
                            let file_name;
                            if subopts.len() > 1 {
                                match usize::from_str(&(subopts[0])[..]) {
                                    Ok(x) => bank = x,
                                    Err(err) => {
                                        let _ = writeln!(stderr, "-l: Bad argument: {} → {} ← {}", arg, subopts[0], err);
                                        panic!("Unable to comprehend number.");
                                    },
                                }
                                file_name = subopts[1].clone();
                            } else {
                                file_name = subopts[0].clone();
                            }
                            let mut file = match File::open(file_name) {
                                Ok(file) => file,
                                Err(err) => {
                                    let _ = writeln!(stderr, "-l: Unable to open image: {} → {}", file_name, err);
                                    panic!("I/O error.");
                                },
                            };
                            images.push(BankImage {
                                name: file_name.to_string(),
                                bank: bank,
                                file: file,
                            });
                        },
                        switch @ _ => { let _ = writeln!(stderr, "Unhandled switch: -{}", switch); },
                    }
                }
            },
            Err(err) => {
                match err {
                    goss::Error::BadOptionString => {
                        let _ = writeln!(stderr, "GOSS is broken.");
                        panic!("The option parsing library claims we're using it wrong.'");
                    },
                    goss::Error::UnknownSwitch(switch) => {
                        let _ = writeln!(stderr, "Unknown switch: {}", switch);
                        panic!("You have specified an unrecognized switch.");
                    },
                    goss::Error::MissingArgument(switch) => {
                        let _ = writeln!(stderr, "Missing argument to -{}.", switch);
                        panic!("You didn't specify a required optarg or you messed up your switch order.");
                    },
                }
            },
        }
        memory = Memory::new(num_banks);
        mmu = MMU::new(memory.clone());
        let mut bank_0_initialized = false;
        for image in images.iter_mut() {
            if image.bank >= num_banks as usize {
                let _ = writeln!(stderr, "Invalid bank: {} for image {}", image.bank, image.name);
                panic!("Total number of banks too low for index. (Number of banks: {})", num_banks);
            }
            let mut image_temp: Vec<u8> = Vec::new();
            match image.file.read_to_end(&mut image_temp) {
                Ok(_) => {
                    if image_temp.len() > mmu::BANK_SIZE {
                        let _ = writeln!(stderr, "Invalid image: {}", image.name);
                        panic!("The file is too large for the bank size ({} bytes).", mmu::BANK_SIZE);
                    }
                },
                Err(err) => {
                    let _ = writeln!(stderr, "-l: Read error: {}", err);
                    panic!("Unable to read bank image.");
                }
            }
            for i in 0..image_temp.len() {
                let mut bank = match memory.banks[image.bank].lock() {
                    Ok(x) => x,
                    Err(err) => {
                        let _ = writeln!(stderr, "-l: Mutex error: {}", err);
                        panic!("Unable to acquire mutex for bank {}.", image.bank);
                    }
                };
                bank[i] = image_temp[i];
            }
            if image.bank == 0 { bank_0_initialized = true; };
        }
        if !bank_0_initialized {
            let _ = writeln!(stderr, "Memory not ready.");
            panic!("You must load an image for bank 0. (-l)")
        }
    }
    let mut cpu = Z80::new(&mut mmu);
    cpu.install_device(0, &mut mmu.bank_registers[0]);
    cpu.install_device(1, &mut mmu.bank_registers[1]);
    cpu.install_device(2, &mut mmu.bank_registers[2]);
    cpu.install_device(3, &mut mmu.bank_registers[3]);

    let device = StdioDevice::new();
    cpu.install_device(4, &mut device.get_control_port());
    cpu.install_device(5, &mut device.get_data_port());

    let die = Arc::new(AtomicBool::new(false));
    let mut device_threads = Vec::new();
    {
        let mut reader = device.get_reader();
        let die = die.clone();
        device_threads.push(thread::spawn(move || reader.run(die)));
    }
    {
        let mut writer = device.get_writer();
        let die = die.clone();
        device_threads.push(thread::spawn(move || writer.run(die)));
    }

    let cpu = Arc::new(&cpu);
    let _ = cpu.execute(0);
    die.store(true, Ordering::Release);
    for thread in device_threads {
        let _ = thread.join().unwrap();
    }
    println!("Exiting successfully.")
}
