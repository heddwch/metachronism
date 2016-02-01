use z80e_core_rust::{ Z80IODevice, Z80Memory };

pub const BANK_SIZE: usize = 0x10000;

#[derive(Clone, Copy)]
pub struct MMUBankRegister {
    bank: u8,
}

impl Z80IODevice for MMUBankRegister {
    fn read_in(&mut self) -> u8 {
        self.bank
    }
    fn write_out(&mut self, bank: u8) {
        self.bank = bank;
    }
}

pub struct MMU {
    pub bank_registers: [MMUBankRegister; 4],
    pub banks: Vec<[u8; BANK_SIZE]>,
}

impl MMU {
    pub fn new(num_banks: u8) -> MMU {
        let mut banks = Vec::<[u8; BANK_SIZE]>::new();
        for _ in 0..num_banks {
            banks.push([0; BANK_SIZE]);
        }
        MMU {
            bank_registers: [MMUBankRegister { bank: 0 }; 4],
            banks: banks,        
        }
    }
}

impl Z80Memory for MMU {
    fn read_byte(&mut self, address: u16) -> u8 {
        let bank_selector = (address >> 14) as usize;
        let bank = self.bank_registers[bank_selector].bank as usize;
        if bank >= self.banks.len() {
            0
        } else {
            self.banks[bank][(address & 0x3FFF) as usize]
        }
    }
    fn write_byte(&mut self, address: u16, value: u8) {
        let bank_selector = (address >> 14) as usize;
        let bank = self.bank_registers[bank_selector].bank as usize;
        if bank < self.banks.len() {
            self.banks[bank][(address & 0x3FFF) as usize] = value;
        }
    }
}
