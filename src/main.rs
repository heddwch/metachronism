extern crate z80e_core_rust;
extern crate goss;
mod mmu;
use z80e_core_rust::{ Z80, Z80IODevice };
use mmu::{ Memory, MMU };
use std::io::{ self, Read, Write };
use std::env;
use std::str::FromStr;
use std::fs::File;
use std::sync::{ Arc, Mutex };

struct StdioDevice {
    stdin: Mutex<io::Stdin>,
    stdout: Mutex<io::Stdout>,
}

impl StdioDevice {
    fn new() -> StdioDevice {
        StdioDevice {
            stdin: Mutex::new(io::stdin()),
            stdout: Mutex::new(io::stdout())
        }
    }
}

impl Z80IODevice for StdioDevice {
    fn read_in(&self) -> u8 {
        let mut buf: [u8; 1] = [0];
        let mut stdin = match self.stdin.lock() {
            Ok(x) => x,
            Err(err) => {
                panic!("StdioDevice mutex poisoned: {}", err);
            },
        };
        let _ = stdin.read(&mut buf);
        buf[0]
    }
    fn write_out(&mut self, value: u8) {
        let mut stdout = match self.stdout.lock() {
            Ok(x) => x,
            Err(err) => {
                panic!("StdioDevice mutex poisoned: {}", err);
            },
        };
        let _ = stdout.write(&[value]);
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
    let mut device = StdioDevice::new();
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
    cpu.install_device(0, &mut device);
    cpu.install_device(1, &mut mmu.bank_registers[0]);
    cpu.install_device(2, &mut mmu.bank_registers[1]);
    cpu.install_device(3, &mut mmu.bank_registers[2]);
    cpu.install_device(4, &mut mmu.bank_registers[3]);
    let cpu = Arc::new(&cpu);
    
    let _ = cpu.execute(0);
}
