use super::ConcurrentDevice;

extern crate memmap;
use z80e_core_rust::{ IoDevice };
use self::memmap::{ Mmap };
pub use self::memmap::{ MmapViewSync, Protection };

use std::sync::{ Arc, Condvar, Mutex };
use std::sync::atomic::{ AtomicBool, AtomicUsize, Ordering };
use std::io::{ self, ErrorKind, Write };
use std::path::Path;
use std::thread;
use std::str;

// Sector size must be a power of two.
const SECTOR_SIZE: u16 = 128;

// Status port bitflags.
//     8-bit values, but "must" be usize to avoid overly-verbose casts for AtomicUsize calls.
const COMMAND_READY: usize = 1 << 4;
const DATA_READY: usize = 1 << 5;
const RESERVED: usize = 1 << 6;
const ERROR: usize = 1 << 7;
const DISK_MASK: usize = 0x0F;

// Commands.
const NOP: u8 = 0;
const SEL_DSK: u8 = 1;
const SEL_TRK: u8 = 2;
const SEL_SEC: u8 = 3;
const READ: u8 = 4;
const WRITE: u8 = 5;
const FLUSH: u8 = 6;
const OPEN: u8 = 7;
const DPB: u8 = 8;

pub struct Disk {
    view: MmapView,
    tracks: u16,
    spt: u16,
    dpb: [u8; 17],
}

impl Disk {
    pub fn open<T: AsRef<Path>>(path: &T, protection: Protection) -> io::Result<Disk> {
        let mut file = try!(Mmap::open_path(path, protection)).into_view_sync();
        let (header, image) = try!(file.split_at(128));
        let header = unsafe { header.as_slice() };
        if match str::from_utf8(&header[0..9]) {
            Ok(x) => x,
            Err(err) => return Err(io::Error::new(ErrorKind::InvalidData, "Not a valid disk image.")),
        } != "<CPM_Disk>" {
            return Err(io::Error::new(ErrorKind::InvalidData, "Not a valid disk image."));
        }
        let mut dpb: [u8; 17] = [0; 17];
        for i in 0..17 {
            dpb[i] = header[32 + i];
        }
        let spt: u16 = (dpb[0] as u16) & ((dpb[1] as u16) << 8);
        let bsh: u16 = (dpb[2] as u16);
        let dsm: u16 = (dpb[5] as u16) & ((dpb[6] as u16) << 8);
        let off: u16 = (dpb[13] as u16) & ((dpb[14] as u16) << 8);
        let tracks: u16 = dsm * (1 << bsh) / spt + off;
        Ok(Disk {
            view: image,
            tracks: tracks,
            spt: spt,
            dpb: dpb
        })
    }
}

#[derive(Clone)]
pub struct DiskController {
    pub status: Arc<AtomicUsize>,
    do_command: Arc<Condvar>,
    buffer: Arc<Mutex<Buffer>>,
    parameters: Arc<Mutex<Parameters>>,
}

impl DiskController {
    fn new() -> DiskController {
        DiskController {
            status: Arc::new(AtomicUsize::new(0)),
            do_command: Arc::new(Condvar::new()),
            buffer: Arc::new(Mutex::new(Buffer::new())),
            parameters: Arc::new(Mutex::new(Parameters::new())),
        }
    }
}

impl ConcurrentDevice for DiskController {
    fn run(&mut self, die: Arc<AtomicBool>) {
//        let disks = Vec::new();
    }
}

struct Buffer {
    bytes: [u8; SECTOR_SIZE],
    i: u8,
}

impl Buffer {
    fn new() -> Buffer {
        Buffer {
            bytes: [0; SECTOR_SIZE],
            i: 0,
        }
    }
}

struct Parameters {
    disk: u8,
    track: u16,
    sector: u16,
    command: u8,
}

impl Parameters {
    fn new() -> Parameters {
        Parameters {
            disk: 0,
            track: 0,
            sector: 0,
            command: NOP,
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
        self.controller.status.fetch_and(!(COMMAND_READY | DATA_READY), Ordering::SeqCst);
        let mut params = self.controller.parameters.lock().unwrap();
        params.command = value;
        self.controller.do_command.notify_one();
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
        if (self.controller.status.load(Ordering::SeqCst) & DATA_READY) != 0 {
            let mut buffer = self.controller.buffer.lock().unwrap();
            let i = (buffer.i & !SECTOR_SIZE) as usize;
            buffer.i += 1;
            buffer.bytes[i]
        } else {
            let _ = writeln!(io::stderr(), "disk: Attempted to read data register when not ready.");
            0
        }
    }
    fn write_out(&mut self, value: u8) {
        if (self.controller.status.load(Ordering::SeqCst) & DATA_READY) != 0 {
            let mut buffer = self.controller.buffer.lock().unwrap();
            let i = (buffer.i & !SECTOR_SIZE) as usize;
            buffer.i += 1;
            buffer.bytes[i] = value;
        } else {
            let _ = writeln!(io::stderr(), "disk: Attempted to write data register when not ready.");
        }
    }
}
