use crate::bus::Memory;
use bitflags::bitflags;

const STACK_SIZE_IN_BYTES: usize = 255;

bitflags! {
    #[repr(transparent)]
    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    struct CpuFlags: u8 {
        const CARRY = 1;
        const ZERO = 1 << 1;
        const INTERRUPT_DISABLE = 1 << 2;
        const DECIMAL_MODE = 1 << 3;
        const BREAK = 1 << 4;
        const UNUSED = 1 << 5;
        const OVERFLOW = 1 << 6;
        const NEGATIVE = 1 << 7;
    }
}

/**
* 6502 Microprocessor
* -------------------
* The CPU is a modified 6502:
*
* 1.79 MHz clock speed
* 8-bit CPU (registers + data bus size)
*  - Accumulator and Index register is 8-bits wide
*  - size of data it can process in one instruction (8-bits)
*
* 16-bit address bus
*  - 2^16 = 65,536 bytes (or 64 Kb)
*
* In essence, an 8-bit CPU like the 6502 is efficient
* at processing 8-bit data but can handle
* larger memory addressing by utilizing a wider
* address bus and addressing instructions
* that handle 16-bit addresses by combining two 8-bit components.
*
* Little Endian (LSB first)
*
**/
pub struct CPU<M: Memory> {
    ac: u8,
    x: u8,
    y: u8,
    pc: u16,                          // Program Counter
    sp: u8,                           // Stack Pointer
    stack: [u8; STACK_SIZE_IN_BYTES], // 0x0100 - 0x01FF
    sr: CpuFlags,
    pub bus: M,
}

impl<M: Memory> CPU<M> {
    pub fn new(bus: M) -> Self {
        CPU {
            pc: 0u16,
            sp: 0xFD,
            stack: [0; 255],
            ac: 0u8,
            x: 0u8,
            y: 0u8,
            sr: CpuFlags::UNUSED | CpuFlags::INTERRUPT_DISABLE,
            bus,
        }
    }

    fn get_flag(&self, flag: CpuFlags) -> bool {
        self.sr.contains(flag)
    }

    fn set_flag(&mut self, flag: CpuFlags, value: bool) {
        self.sr.set(flag, value);
    }

    fn set_zero_and_negative_flag(&mut self, value: u8) {
        self.set_flag(CpuFlags::ZERO, value == 0);
        self.set_flag(CpuFlags::NEGATIVE, (value & 0x80) != 0);
    }

    fn inc_pc(&mut self) {
        self.pc = self.pc.wrapping_add(1);
    }

    fn get_address(lsb: u8, msb: u8) -> u16 {
        ((msb as u16) << 8) | lsb as u16
    }

    fn addr_absolute(&mut self) -> u16 {
        let lsb = self.bus.read(self.pc);
        self.inc_pc();
        let msb = self.bus.read(self.pc);
        self.inc_pc();
        let address: u16 = Self::get_address(lsb, msb);
        address
    }

    fn cross_page_boundary_cycle_penalty(base_addr: u16, effective_addr: u16) -> u64 {
        if (base_addr & 0xFF00) != (effective_addr & 0xFF00) {
            1
        } else {
            0
        }
    }

    fn addr_absolute_x(&mut self) -> (u16, u64) {
        let base_addr = self.addr_absolute();
        let effective_addr = base_addr.wrapping_add(self.x as u16);
        (
            effective_addr,
            Self::cross_page_boundary_cycle_penalty(base_addr, effective_addr),
        )
    }

    fn addr_absolute_y(&mut self) -> (u16, u64) {
        let base_addr = self.addr_absolute();
        let effective_addr = base_addr.wrapping_add(self.y as u16);
        (
            effective_addr,
            Self::cross_page_boundary_cycle_penalty(base_addr, effective_addr),
        )
    }

    fn addr_absolute_indirect(&mut self) {
        let base_addr = self.addr_absolute();
        let lsb = self.bus.read(base_addr);
        // When the inc crosses a page boundary we don't add 1
        let msb = if (base_addr & 0x00FF) == 0x00FF {
            self.bus.read(base_addr & 0xFF00)
        } else {
            self.bus.read(base_addr.wrapping_add(1))
        };
        self.pc = Self::get_address(lsb, msb);
    }

    pub fn step(&mut self) -> u8 {
        let opcode = self.bus.read(self.pc);
        self.inc_pc();
        let mut cycles = 0u8;

        match opcode {
            0xA9 => {
                // LDA #$nn (LDA Immediate)
                let value = self.bus.read(self.pc);
                self.inc_pc();
                self.ac = value;
                self.set_zero_and_negative_flag(self.ac);
                cycles = 2;
            }
            0xAD => {
                // LDA #&nnnn (LDA Absolute addressing)
                let address: u16 = self.addr_absolute();
                let value = self.bus.read(address);
                self.ac = value;
                self.set_zero_and_negative_flag(value);
                cycles = 4;
            }

            other => panic!("Invalid opcode: {other}"),
        }

        cycles
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Bus;
    // A mock memory bus for testing. It's just a simple RAM array.
    struct MockBus {
        mem: [u8; 0xFFFF],
    }

    impl MockBus {
        fn new() -> Self {
            MockBus { mem: [0; 0xFFFF] }
        }
    }

    impl Memory for MockBus {
        fn read(&self, addr: u16) -> u8 {
            self.mem[addr as usize]
        }

        fn write(&mut self, addr: u16, value: u8) {
            self.mem[addr as usize] = value;
        }
    }

    fn setup_cpu() -> CPU<MockBus> {
        let bus = MockBus::new();
        CPU::new(bus)
    }

    #[test]
    fn test_construct_cpu() {
        let bus = Bus::new();
        let cpu = CPU::new(bus);

        assert_eq!(cpu.pc, 0u16);
        assert_eq!(cpu.sp, 0xFD);
        assert_eq!(cpu.y, 0u8);
        assert_eq!(cpu.x, 0u8);
        assert_eq!(cpu.ac, 0u8);
        assert_eq!(cpu.sr, CpuFlags::INTERRUPT_DISABLE | CpuFlags::UNUSED);
    }

    #[test]
    fn test_addr_absolute() {
        let mut cpu = setup_cpu();
        cpu.pc = 0x1000;

        // The address $1234 stored in little-endian format
        cpu.bus.write(0x1000, 0x34);
        cpu.bus.write(0x1001, 0x12);

        let addr = cpu.addr_absolute();

        assert_eq!(addr, 0x1234);
        assert_eq!(cpu.pc, 0x1002); // PC should advance by 2
    }

    #[test]
    fn test_addr_absolute_x_no_page_cross() {
        let mut cpu = setup_cpu();
        cpu.pc = 0x1000;
        cpu.bus.write(0x1000, 0xFD);
        cpu.bus.write(0x1001, 0x00);
        cpu.x = 0x01;

        let (addr, extra_cycle) = cpu.addr_absolute_x();

        assert_eq!(addr, 0x00FE);
        assert_eq!(extra_cycle, 0);
        assert_eq!(cpu.pc, 0x1002);
    }

    #[test]
    fn test_addr_absolute_x_page_cross() {
        let mut cpu = setup_cpu();
        cpu.pc = 0x1000;
        cpu.bus.write(0x1000, 0xFF);
        cpu.bus.write(0x1001, 0x00);
        cpu.x = 0x01;

        let (addr, extra_cycle) = cpu.addr_absolute_x();

        assert_eq!(addr, 0x0100);
        assert_eq!(extra_cycle, 1);
        assert_eq!(cpu.pc, 0x1002);
    }

    #[test]
    fn test_addr_absolute_y_no_page_cross() {
        let mut cpu = setup_cpu();
        cpu.pc = 0x1000;
        cpu.bus.write(0x1000, 0xFD);
        cpu.bus.write(0x1001, 0x00);
        cpu.y = 0x01;

        let (addr, extra_cycle) = cpu.addr_absolute_y();

        assert_eq!(addr, 0x00FE);
        assert_eq!(extra_cycle, 0);
        assert_eq!(cpu.pc, 0x1002);
    }

    #[test]
    fn test_addr_absolute_y_page_cross() {
        let mut cpu = setup_cpu();
        cpu.pc = 0x1000;
        cpu.bus.write(0x1000, 0xFF);
        cpu.bus.write(0x1001, 0x00);
        cpu.y = 0x01;

        let (addr, extra_cycle) = cpu.addr_absolute_y();

        assert_eq!(addr, 0x0100);
        assert_eq!(extra_cycle, 1);
        assert_eq!(cpu.pc, 0x1002);
    }

    #[test]
    fn test_addr_absolute_indirect_normal() {
        let mut cpu = setup_cpu();
        cpu.pc = 0x1000;
        cpu.bus.write(0x1000, 0xFD);
        cpu.bus.write(0x1001, 0x12);
        // LSB
        cpu.bus.write(0x12FD, 0x21);
        // MSB
        cpu.bus.write(0x12FE, 0x23);

        cpu.addr_absolute_indirect();
        assert_eq!(cpu.pc, 0x2321);
    }

    #[test]
    fn test_addr_absolute_indirect_bug() {
        let mut cpu = setup_cpu();
        cpu.pc = 0x1000;
        cpu.bus.write(0x1000, 0xFF);
        cpu.bus.write(0x1001, 0x12);
        // LSB
        cpu.bus.write(0x12FF, 0x21);
        // MSB
        cpu.bus.write(0x1200, 0x23);

        cpu.addr_absolute_indirect();
        assert_eq!(cpu.pc, 0x2321);
    }
}
