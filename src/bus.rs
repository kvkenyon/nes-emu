pub trait Memory {
    fn read(&self, address: u16) -> u8;
    fn write(&mut self, address: u16, value: u8);
}

pub struct Bus {
    pub ram: [u8; 0x800],
    pub ppu: [u8; 0x7],
}

impl Bus {
    pub fn new() -> Self {
        Bus {
            ram: [0u8; 0x800],
            ppu: [0u8; 7],
        }
    }
}

impl Memory for Bus {
    fn read(&self, address: u16) -> u8 {
        match address {
            0x0000..=0x07ff => self.ram[(address & 0x07FF) as usize],
            0x2000..=0x2007 => self.ppu[(address & 0x0001) as usize],
            _ => panic!("Not implemented yet."),
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        match address {
            0x0000..=0x07ff => self.ram[(address & 0x07FF) as usize] = value,
            0x2000..=0x2007 => self.ppu[(address & 0x000F) as usize] = value,
            _ => panic!("Not implemented yet."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_construct_bus() {
        let bus = Bus::new();
        assert_eq!(bus.ram.len(), 2048);
        assert_eq!(bus.ppu.len(), 7);
    }
}
