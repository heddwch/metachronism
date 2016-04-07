use super::ConcurrentDevice;

extern crate memmap;
use z80e_core_rust::{ IoDevice };
use self::memmap::{ Mmap };
pub use self::memmap::{ MmapView, Protection };

use std::sync::{ Arc, Condvar, Mutex };
use std::sync::atomic::{ AtomicBool, AtomicUsize, Ordering };
use std::io::{ self, ErrorKind, Write };
use std::path::Path;
use std::{ str, mem, ptr };

// Sector size must be a power of two
const SECTOR_SIZE: u16 = 128;

// Max disk must be a power of two <= 16. If changed, you must insert/delete Nones in DiskController::run()
const MAX_DISK: u8 = 16;

// Status port bitflags.
//     8-bit values, but "must" be usize to avoid overly-verbose casts for AtomicUsize calls.
const COMMAND_READY: usize = 1 << 0;
const DATA_READY: usize = 1 << 1;
const ERROR: usize = 1 << 7;

// Commands.
const NOP: u8 = 0;
const SEL_DSK: u8 = 1;
const SEL_TRK: u8 = 2;
const SEL_SEC: u8 = 3;
const READ: u8 = 4;
const WRITE: u8 = 5;
const RESET: u8 = 6;
const OPEN: u8 = 7;
const CLOSE: u8 = 8;
const DPB: u8 = 9;

#[derive(Clone)]
pub struct DiskController {
    pub status: Arc<AtomicUsize>,
    command_cond: Arc<Condvar>,
    buffer: Arc<Mutex<Buffer>>,
    parameters: Arc<Mutex<Parameters>>,
}

impl DiskController {
    pub fn new() -> DiskController {
        DiskController {
            status: Arc::new(AtomicUsize::new(DATA_READY)),
            command_cond: Arc::new(Condvar::new()),
            buffer: Arc::new(Mutex::new(Buffer::new())),
            parameters: Arc::new(Mutex::new(Parameters::new())),
        }
    }
    pub fn status_port(&self) -> StatusPort {
        StatusPort::new(self.clone())
    }
    pub fn data_port(&self) -> DataPort {
        DataPort::new(self.clone())
    }
}

struct Buffer {
    bytes: [u8; SECTOR_SIZE as usize],
    i: u16,
}

impl Buffer {
    fn new() -> Buffer {
        Buffer {
            bytes: [0; SECTOR_SIZE as usize],
            i: 0,
        }
    }
}

struct Parameters {
    disk: u8,
    track: u16,
    sector: u16,
    command: u8,
    do_command: bool,
}

impl Parameters {
    fn new() -> Parameters {
        Parameters {
            disk: 0,
            track: 0,
            sector: 0,
            command: NOP,
            do_command: false,
        }
    }
}

struct StatusPort {
    controller: DiskController,
}

impl StatusPort {
    fn new(controller: DiskController) -> StatusPort {
        StatusPort {
            controller: controller,
        }
    }
}

impl IoDevice for StatusPort {
    fn read_in(&self) -> u8 {
        (self.controller.status.load(Ordering::SeqCst) & 0xff) as u8
    }
    fn write_out(&mut self, value: u8) {
        if (self.controller.status.fetch_and(!COMMAND_READY, Ordering::SeqCst) & COMMAND_READY) != 0 {
            let mut params = self.controller.parameters.lock().unwrap();
            params.command = value;
            params.do_command = true;
            self.controller.command_cond.notify_one();
        } else {
            self.controller.status.fetch_or(COMMAND_READY | ERROR, Ordering::SeqCst);
            let _ = writeln!(io::stderr(), "disk: Attempted to write command register when not ready.");
        }
    }
}
        
struct DataPort {
    controller: DiskController,
}

impl DataPort {
    fn new(controller: DiskController) -> DataPort {
        DataPort {
            controller: controller,
        }
    }
}

impl IoDevice for DataPort {
    fn read_in (&self) -> u8 {
        if (self.controller.status.fetch_and(!DATA_READY, Ordering::SeqCst) & DATA_READY) != 0 {
            let byte;
            {
                let mut buffer = self.controller.buffer.lock().unwrap();
                buffer.i = ((buffer.i as usize + 1) & (SECTOR_SIZE - 1) as usize) as u16;
                byte = buffer.bytes[buffer.i as usize];
            }
            self.controller.status.fetch_or(DATA_READY, Ordering::SeqCst);
            byte
        } else {
            self.controller.status.fetch_or(ERROR, Ordering::SeqCst);
            let _ = writeln!(io::stderr(), "disk: Attempted to read data register when not ready.");
            0
        }
    }
    fn write_out(&mut self, value: u8) {
        if (self.controller.status.fetch_and(!DATA_READY, Ordering::SeqCst) & DATA_READY) != 0 {
            {
                let mut buffer = self.controller.buffer.lock().unwrap();
                buffer.bytes[buffer.i as usize] = value;
                buffer.i = ((buffer.i as usize + 1) & (SECTOR_SIZE - 1) as usize) as u16;
            }
            self.controller.status.fetch_or(DATA_READY, Ordering::SeqCst);
        } else {
            self.controller.status.fetch_or(ERROR, Ordering::SeqCst);
            let _ = writeln!(io::stderr(), "disk: Attempted to write data register when not ready.");
        }
    }
}

pub struct Disk {
    view: MmapView,
    pub tracks: u16,
    pub spt: u16,
    dpb: [u8; 16],
}

impl Disk {
    pub fn open<T: AsRef<Path>>(path: &T, protection: Protection) -> io::Result<Disk> {
        let mut file = try!(Mmap::open_path(path, protection)).into_view();
        let (header, image) = try!(file.split_at(128));
        let header = unsafe { header.as_slice() };
        if match str::from_utf8(&header[0..10]) {
            Ok(x) => x,
            Err(err) => return Err(io::Error::new(ErrorKind::InvalidData, "Invalid image header encoding.")),
        }!= "<CPM_Disk>" {
            return Err(io::Error::new(ErrorKind::InvalidData, "Not a valid disk image."));
        }
        let mut dpb: [u8; 16] = [0; 16];
        for i in 0..16 {
            dpb[i] = header[32 + i];
        }
        let spt: u16 = (dpb[0] as u16) | ((dpb[1] as u16) << 8);
        let bsh: u16 = (dpb[2] as u16);
        let dsm: u16 = (dpb[5] as u16) | ((dpb[6] as u16) << 8);
        let off: u16 = (dpb[13] as u16) | ((dpb[14] as u16) << 8);
        let tracks: u16 = (dsm + 1) * (1 << bsh) / spt + off;
        Ok(Disk {
            view: image,
            tracks: tracks,
            spt: spt,
            dpb: dpb
        })
    }
}

impl ConcurrentDevice for DiskController {
    fn run(&mut self, die: Arc<AtomicBool>) {
        let mut disks = unsafe {
            let mut disks: [Option<Disk>; MAX_DISK as usize] = mem::uninitialized();
            for disk in disks.iter_mut() {
                ptr::write(disk, None);
            };
            disks
        };
        let mut parameters = self.parameters.lock().unwrap();
        loop {
            self.status.fetch_or(COMMAND_READY, Ordering::SeqCst);
            while !(*parameters).do_command {
                parameters = self.command_cond.wait(parameters).unwrap();
            }
            if die.load(Ordering::Acquire) { break; }
            let status = self.status.fetch_and(!DATA_READY, Ordering::SeqCst);
            if (status & ERROR) != 0 && parameters.command != RESET {
                self.status.fetch_or(DATA_READY, Ordering::SeqCst);
                continue;
            };
            {
                let mut buffer = self.buffer.lock().unwrap();
                buffer.i = 0;
                match parameters.command {
                    NOP => (),
                    SEL_DSK => {
                        if buffer.bytes[0] < MAX_DISK {
                            parameters.disk = buffer.bytes[0];
                        } else {
                            self.status.fetch_or(ERROR, Ordering::SeqCst);
                        }
                    },
                    SEL_TRK => {
                        let track = buffer.bytes[0] as u16 & ((buffer.bytes[1] as u16) << 8);
                        match disks[parameters.disk as usize] {
                            Some(ref disk) => {
                                if track < disk.tracks {
                                    parameters.track = track;
                                } else {
                                    self.status.fetch_or(ERROR, Ordering::SeqCst);
                                }
                            },
                            None => {
                                self.status.fetch_or(ERROR, Ordering::SeqCst);
                            }
                        }
                    },
                    SEL_SEC => {
                        match disks[parameters.disk as usize] {
                            Some(ref disk) => {
                                let sector = buffer.bytes[0] as u16 & ((buffer.bytes[1] as u16) << 8);
                                if sector < disk.spt {
                                    parameters.sector = sector;
                                } else {
                                    self.status.fetch_or(ERROR, Ordering::SeqCst);
                                }
                            },
                            None => {
                                self.status.fetch_or(ERROR, Ordering::SeqCst);
                            },
                        }
                    },
                    READ => {
                        match disks[parameters.disk as usize] {
                            Some(ref disk) => {
                                let off = ((parameters.track as usize * disk.spt as usize)
                                           + parameters.sector as usize) * SECTOR_SIZE as usize;
                                let bytes = unsafe { disk.view.as_slice() };
                                for (i, byte) in buffer.bytes.iter_mut().enumerate() {
                                    *byte = bytes[off + i];
                                }
                            },
                            None => {
                                self.status.fetch_or(ERROR, Ordering::SeqCst);
                            },
                        }
                    },
                    WRITE => {
                        match disks[parameters.disk as usize] {
                            Some(ref mut disk) => {
                                let off = ((parameters.track as usize * disk.spt as usize)
                                           + parameters.sector as usize) * SECTOR_SIZE as usize;
                                let bytes = unsafe { disk.view.as_mut_slice() };
                                for (i, byte) in buffer.bytes.iter().enumerate() {
                                    bytes[i] = *byte;
                                };
                            },
                            None => {
                                self.status.fetch_or(ERROR, Ordering::SeqCst);
                            },
                        }

                    }
                    RESET => {
                        parameters.disk = 0;
                        parameters.track = 0;
                        parameters.sector = 0;
                        parameters.command = NOP;
                        self.status.fetch_and(!ERROR, Ordering::SeqCst);
                    },
                    OPEN => {
                        match str::from_utf8(buffer.bytes.split(|a| *a == 0).next().unwrap()) {
                            Ok(file_name) => {
                                match Disk::open(&file_name, Protection::ReadWrite) {
                                    Ok(disk) => {
                                        disks[parameters.disk as usize] = Some(disk);
                                    },
                                    Err(err) => {
                                        let mut stderr = io::stderr();
                                        let _ = writeln!(stderr, "disk: Failed to open file: {}", file_name);
                                        let _ = writeln!(stderr, "Error:\n\t{}", err);
                                        self.status.fetch_or(ERROR, Ordering::SeqCst);
                                    },
                                }
                            },
                            Err(err) => {
                                let _ = write!(io::stderr(), "disk: Bad UTF-8 in file name.\nError:\n\t{}\n", err);
                                self.status.fetch_or(ERROR, Ordering::SeqCst);
                            }
                        }
                    },
                    CLOSE => {
                        disks[parameters.disk as usize] = None;
                    },
                    DPB => {
                        match disks[parameters.disk as usize] {
                            Some(ref disk) => {
                                for (a, b) in disk.dpb.iter().zip(buffer.bytes.iter_mut()) {
                                    *b = *a;
                                }
                            },
                            None => {
                                self.status.fetch_or(ERROR, Ordering::SeqCst);
                            }
                        }
                    },
                    _ => {
                        self.status.fetch_or(ERROR, Ordering::SeqCst);
                        let _ = write!(io::stderr(), "disk: System sent bad command: {:02X}\n", parameters.command);
                    },
                }
            }
            self.status.fetch_or(DATA_READY, Ordering::SeqCst);
        }
    }
}
