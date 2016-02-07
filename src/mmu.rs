use z80e_core_rust::{ Z80IODevice, Z80Memory };
use std::sync::{ Arc, Mutex };

pub const BANK_SIZE: usize = 0x10000;

#[derive(Clone, Copy)]
pub struct MMUBankRegister {
    bank: u8,
}

impl Z80IODevice for MMUBankRegister {
    fn read_in(&self) -> u8 {
        self.bank
    }
    fn write_out(&mut self, bank: u8) {
        self.bank = bank;
    }
}

pub struct Memory {
    pub banks: Arc<Vec<Mutex<[u8; BANK_SIZE]>>>,
}

impl Memory {
    pub fn new(num_banks: u8) -> Memory {
        let mut banks = Vec::new();
        for _ in 0..num_banks {
            banks.push(Mutex::new([0; BANK_SIZE]));
        }
        Memory { banks: Arc::new(banks) }
    }
}

impl Clone for Memory {
    fn clone(&self) -> Memory {
        Memory {
            banks: self.banks.clone()
        }
    }
}

pub struct MMU {
    pub bank_registers: [MMUBankRegister; 4],
    memory: Memory,
}

impl MMU {
    pub fn new(memory: Memory) -> MMU {
        MMU {
            bank_registers: [MMUBankRegister { bank: 0 }; 4],
            memory: memory,
        }
    }
}

impl Z80Memory for MMU {
    fn read_byte(&self, address: u16) -> u8 {
        let bank_selector = (address >> 14) as usize;
        let bank_num = self.bank_registers[bank_selector].bank as usize;
        if bank_num >= self.memory.banks.len() {
            0
        } else {
            let bank = match self.memory.banks[bank_num].lock() {
                Ok(x) => x,
                Err(err) => panic!("Bank {} mutex poisoned: {}", bank_num, err),
            };
            bank[(address & 0x3FFF) as usize]
        }
    }
    fn write_byte(&mut self, address: u16, value: u8) {
        let bank_selector = (address >> 14) as usize;
        let bank_num = self.bank_registers[bank_selector].bank as usize;
        if bank_num < self.memory.banks.len() {
            let mut bank = match self.memory.banks[bank_num].lock() {
                Ok(x) => x,
                Err(err) => panic!("Bank {} mutex poisoned: {}", bank_num, err),
            };
            bank[(address & 0x3FFF) as usize] = value;
        }
    }
}
