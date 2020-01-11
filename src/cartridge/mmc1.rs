use super::{Cartridge, Mapper, Mirror};

pub struct Mmc1 {
    cart: Cartridge,
    step: u8,
    shift_register: u8,
    mirroring: Mirror,
    control: u8,

    prg_ram_bank: Vec<u8>, // CPU $6000-$7FFF
    prg_ram_enabled: bool,

    prg_bank_mode: u8,
    prg_bank_select: usize, // selects among the PRG-RAM chunks in the cartridge

    chr_ram_bank: Vec<u8>, // used if cartridge doesn't have any CHR-ROM, 8KB, $0000-$1FFF
    chr_low_bank: usize,   // PPU $0000-$0FFF
    chr_high_bank: usize,  // PPU $1000-$1FFF
    chr_bank_mode: bool,   // false: switch 8 KB at a time; true: switch two separate 4 KB banks
}

impl Mmc1 {
    pub fn new(cart: Cartridge) -> Self {
        let m = cart.mirroring;
        Mmc1 {
            cart: cart,
            step: 0,
            shift_register: 0,
            mirroring: m,
            control: 0,
            prg_ram_bank: vec![0; 0x2000],
            prg_ram_enabled: false,
            prg_bank_mode: 0,
            prg_bank_select: 0,
            chr_ram_bank: vec![0; 0x2000],
            chr_low_bank: 0,
            chr_high_bank: 0,
            chr_bank_mode: false,
        }
    }

    fn write_serial_port(&mut self, address: usize, value: u8) {
        // if reset flag is on, reset
        if value & 0b1000_0000 != 0 {
            self.step = 0;
            self.shift_register = 0;
            // locking PRG ROM at $C000-$FFFF to the last bank
            // self.prg_high_bank = self.cart.prg_rom_size - 1;
            self.write_control(self.control | 0xC)
        } else {
            // otherwise perform normal write
            self.shift_register >>= 1;
            self.shift_register |= (value & 1) << 7;
            if self.step == 4 {
                // shift register values will be in top 5 bits, so cut it down to size before moving on to where it's used
                self.shift_register >>= 3;
                match address {
                    0x8000..=0x9FFF => self.write_control(self.shift_register),
                    0xA000..=0xBFFF => self.write_chr_bank_low(self.shift_register),
                    0xC000..=0xDFFF => self.write_chr_bank_high(self.shift_register),
                    0xE000..=0xFFFF => self.write_prg_bank(self.shift_register),
                    _ => panic!("bad address write to MMC1: 0x{:X}", address),
                }
                self.step = 0;
                self.shift_register = 0;
            } else {
                self.step += 1;
            }
        }
    }

    fn write_control(&mut self, value: u8) {
        self.control = value;
        self.mirroring = match value & 0b11 {
            0 => Mirror::LowBank,
            1 => Mirror::HighBank,
            2 => Mirror::Vertical,
            3 => Mirror::Horizontal,
            _ => panic!("invalid mirroring value"),
        };
        self.prg_bank_mode = (value >> 2) & 0b11;
        self.chr_bank_mode = if value & (1<<4) == 0 {false} else {true};
    }

    fn write_chr_bank_low(&mut self, value: u8) {
        if self.chr_bank_mode { // 4 KB mode
            self.chr_low_bank = value as usize;
        } else { // 8 KB mode
            let v = value & (0xFF - 1); // turn off low bit
            self.chr_low_bank = v as usize;
            self.chr_high_bank = (v + 1) as usize;
        }
    }

    fn write_chr_bank_high(&mut self, value: u8) {
        if self.chr_bank_mode { // 4 KB mode only, ignored in 8 KB mode
            self.chr_high_bank = value as usize;
        }
    }

    fn write_prg_bank(&mut self, value: u8) {
        self.prg_bank_select = (value & 0b1111) as usize;
        self.prg_ram_enabled = value & 0b10000 != 0;
    }
}

impl Mapper for Mmc1 {
    fn read(&mut self, address: usize) -> u8 {
        match address {
            0x0000..=0x0FFF => {
                if self.cart.chr_rom_size > self.chr_low_bank {
                    self.cart.chr_rom[self.chr_low_bank][address % 0x1000]
                } else {
                    self.chr_ram_bank[address]
                }
            },
            0x1000..=0x1FFF => {
                if self.cart.chr_rom_size > self.chr_high_bank {
                    self.cart.chr_rom[self.chr_high_bank][address % 0x1000]
                } else {
                    self.chr_ram_bank[address]
                }
            },
            0x6000..=0x7FFF => self.prg_ram_bank[address % 0x2000],
            0x8000..=0xBFFF => {
                match self.prg_bank_mode {
                    0 | 1 => { // switch 32 KB at $8000, ignoring low bit of bank number
                        let low_bank = self.prg_bank_select & (0xFF - 1);
                        self.cart.prg_rom[low_bank][address % 0x4000]
                    },
                    2 => self.cart.prg_rom[0][address % 0x4000],
                    3 => self.cart.prg_rom[self.prg_bank_select][address % 0x4000],
                    _ => panic!("invalid PRG bank mode"),
                }
            },
            0xC000..=0xFFFF => {
                match self.prg_bank_mode {
                    0 | 1 => { // switch 32 KB at $8000, ignoring low bit of bank number
                        let high_bank = (self.prg_bank_select & (0xFF - 1)) + 1;
                        self.cart.prg_rom[high_bank][address % 0x4000]
                    },
                    2 => self.cart.prg_rom[self.prg_bank_select][address % 0x4000],
                    3 => self.cart.prg_rom[self.cart.prg_rom_size - 1][address % 0x4000],
                    _ => panic!("invalid PRG bank mode"),
                }
            },
            _ => panic!("invalid address passed to MMC1: 0x{:X}", address),
        }
    }

    fn write(&mut self, address: usize, value: u8) {
        match address {
            0x0000..=0x1FFF => { // if we don't have CHR-ROM, write to CHR-RAM
                if self.cart.chr_rom_size == 0 {
                    self.chr_ram_bank[address] = value;
                }
            },
            0x6000..=0x7FFF => self.prg_ram_bank[address % 0x2000] = value,
            0x8000..=0xFFFF => self.write_serial_port(address, value),
            _ => panic!("bad address write to MMC1: 0x{:X}", address),
        }
    }

    fn get_mirroring(&mut self) -> Mirror {
        self.mirroring
    }
}