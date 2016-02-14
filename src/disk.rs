use super::ConcurrentDevice;

use z80e_core_rust::{ Z80IODevice };

use std::sync::{ Arc, Condvar, Mutex };
use std::sync::atomic::{ AtomicBool, AtomicUsize, Ordering };
use std::io::{ self, Read, Write, Seek, SeekFrom, ErrorKind };
use std::fs::{ OpenOptions, File };
use std::path::Path;
use std::thread;
use std::str;

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

pub struct Disk {
    file: File,
    spt: u16,
    bsh: u8,
    blm: u8,
    tracks: u16,
}

impl Disk {
    pub fn open<T: AsRef<Path>>(path: &T) -> io::Result<Disk> {
        let mut file = try!(OpenOptions::new().read(true).write(true).open(path));
        {
            let mut magic: [u8; 10] = [0; 10];
            let mut read = 0;
            while read < magic.len() {
                read += try!(file.read(&mut magic[read..]));
            }
            if match str::from_utf8(&magic) {
                Ok(x) => x,
                Err(err) => return Err(io::Error::new(ErrorKind::InvalidData, "Not a valid disk image.")),
            } != "<CPM_Disk>" {
                return Err(io::Error::new(ErrorKind::InvalidData, "Not a valid disk image."));
            }
        }
        try!(file.seek(SeekFrom::Start(32)));
        let mut spt: [u8; 2] = [0; 2];
        let mut read = 0;
        while read < spt.len() {
            read += try!(file.read(&mut spt[read..]));
        }
        let spt: u16 = (spt[0] as u16) | ((spt[1] as u16) << 8);
        let mut bsh = [u8; 1] = [0; 1];
        while try!(file.read(&mut bsh)) == 0 {};
        let bsh = bsh[0];
        let mut blm: [u8; 1] = [0; 1];
        while try!(file.read(&mut blm)) == 0 {};
        let blm = blm[0];
        let bls = blm + 1;
        try!(file.seek(SeekFrom::Current(1)));
        let mut dsm: [u8; 2] = [0; 2];
        let mut read = 0;
        while read < dsm.len() {
            read += try!(file.read(&mut dsm[read..]));
        }
        let dsm: u16 = (dsm[0] as u16) | ((dsm[1] as u16) << 8);
        Ok(Disk {
            file: file,
            spt: spt,
            bsh: bsh,
            blm: blm,
            tracks: ((bls as u16 * (dsm + 1)) / spt),
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
        
    }
}

struct Buffer {
    bytes: [u8; 0x80],
    i: u8,
}

impl Buffer {
    fn new() -> Buffer {
        Buffer {
            bytes: [0; 0x80],
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

impl Z80IODevice for StatusPort {
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

impl Z80IODevice for DataPort {
    fn read_in (&self) -> u8 {
        if (self.controller.status.load(Ordering::SeqCst) & DATA_READY) != 0 {
            let mut buffer = self.controller.buffer.lock().unwrap();
            let i = (buffer.i & !0x80) as usize;
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
            let i = (buffer.i & !0x80) as usize;
            buffer.i += 1;
            buffer.bytes[i] = value;
        } else {
            let _ = writeln!(io::stderr(), "disk: Attempted to write data register when not ready.");
        }
    }
}
