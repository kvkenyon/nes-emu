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
            sp: 0xFF,
            stack: [0; 255],
            ac: 0u8,
            x: 0u8,
            y: 0u8,
            sr: CpuFlags::UNUSED | CpuFlags::INTERRUPT_DISABLE,
            bus,
        }
    }

    pub fn get_a(&self) -> u8 {
        self.ac
    }
    pub fn get_x(&self) -> u8 {
        self.x
    }
    pub fn get_y(&self) -> u8 {
        self.y
    }
    pub fn get_pc(&self) -> u16 {
        self.pc
    }
    pub fn get_sp(&self) -> u8 {
        self.sp
    }
    pub fn get_p(&self) -> u8 {
        self.sr.bits()
    }

    pub fn set_a(&mut self, v: u8) {
        self.ac = v;
    }
    pub fn set_x(&mut self, v: u8) {
        self.x = v;
    }
    pub fn set_y(&mut self, v: u8) {
        self.y = v;
    }
    pub fn set_pc(&mut self, v: u16) {
        self.pc = v;
    }
    pub fn set_sp(&mut self, v: u8) {
        self.sp = v;
    }
    pub fn set_p(&mut self, v: u8) {
        self.sr = CpuFlags::from_bits_truncate(v);
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

    fn peek_stack(&self) -> u8 {
        self.bus.read(self.sp as u16 + 0x0100u16 + 1)
    }

    fn push_stack(&mut self, value: u8) {
        self.bus.write(self.sp as u16 + 0x0100u16, value);
        self.sp -= 1;
    }

    fn pull_stack(&mut self) -> u8 {
        self.sp += 1;
        let val = self.bus.read(self.sp as u16 + 0x0100u16);
        val
    }

    fn addr_absolute(&mut self) -> u16 {
        let lsb = self.bus.read(self.pc);
        self.inc_pc();
        let msb = self.bus.read(self.pc);
        self.inc_pc();
        Self::get_address(lsb, msb)
    }

    fn cross_page_boundary_cycle_penalty(base_addr: u16, effective_addr: u16) -> u64 {
        // if the upper bytes are different there's a page cross
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

    fn addr_zero_page(&mut self) -> u16 {
        let lsb = self.bus.read(self.pc);
        self.inc_pc();
        Self::get_address(lsb, 0x00)
    }

    fn addr_zero_page_x(&mut self) -> u16 {
        let lsb = self.bus.read(self.pc);
        self.inc_pc();

        lsb.wrapping_add(self.x) as u16
    }

    fn addr_zero_page_y(&mut self) -> u16 {
        let lsb = self.bus.read(self.pc);
        self.inc_pc();

        lsb.wrapping_add(self.y) as u16
    }

    fn addr_zero_page_x_indirect(&mut self) -> u16 {
        let zp = self.bus.read(self.pc);
        self.inc_pc();

        let ptr = zp.wrapping_add(self.x) as u16;

        let lsb = self.bus.read(ptr);
        let msb = self.bus.read((ptr.wrapping_add(1) & 0x00FF) as u16);

        Self::get_address(lsb, msb)
    }

    fn addr_zero_page_y_indirect(&mut self) -> (u16, u64) {
        let zp_addr = self.bus.read(self.pc) as u16;
        self.inc_pc();

        let lsb = self.bus.read(zp_addr);
        let msb = self.bus.read((zp_addr.wrapping_add(1) & 0x00FF) as u16);

        let base_addr = Self::get_address(lsb, msb);

        let (new_lsb, overflow) = self.y.overflowing_add(lsb);
        let new_msb = msb.wrapping_add(if overflow == true { 1 } else { 0 });

        let effective_addr = u16::from_le_bytes([new_lsb, new_msb]);

        let page_crossed = Self::cross_page_boundary_cycle_penalty(base_addr, effective_addr);

        (effective_addr, page_crossed)
    }

    fn addr_relative(&mut self) -> (u16, u64) {
        let offset: i8 = self.bus.read(self.pc) as i8;
        self.inc_pc();

        println!("Offset: {offset}");

        let base_addr = self.pc;

        let effective_addr = base_addr.wrapping_add_signed(offset as i16);
        let page_crossed = Self::cross_page_boundary_cycle_penalty(base_addr, effective_addr);

        self.pc = effective_addr;

        (effective_addr, page_crossed)
    }

    pub fn step(&mut self) -> u64 {
        let opcode = self.bus.read(self.pc);
        self.inc_pc();
        let mut cycles = 0;

        match opcode {
            0xA9 => {
                // LDA #$nn (LDA Immediate)
                let value = self.bus.read(self.pc);
                self.inc_pc();
                cycles = 2;
                self.ac = value;
                self.set_zero_and_negative_flag(self.ac);
            }
            0xAD => {
                // LDA #&nnnn (LDA Absolute addressing)
                let address: u16 = self.addr_absolute();
                let value = self.bus.read(address);
                cycles = 4;
                self.ac = value;
                self.set_zero_and_negative_flag(value);
            }
            0xBD => {
                // LDA $nnnn,X
                // absolute x
                let (address, p) = self.addr_absolute_x();
                cycles += 4 + p;
                let value = self.bus.read(address);
                self.ac = value;
                self.set_zero_and_negative_flag(value);
            }
            0xB9 => {
                // absolute y
                let (address, p) = self.addr_absolute_y();
                cycles += 4 + p;
                let value = self.bus.read(address);
                self.ac = value;
                self.set_zero_and_negative_flag(value);
            }
            0xA5 => {
                // zero-page
                let address = self.addr_zero_page();
                cycles = 3;
                let value = self.bus.read(address);
                self.ac = value;
                self.set_zero_and_negative_flag(value);
            }
            0xB5 => {
                // x-indexed zero page
                let address = self.addr_zero_page_x();
                cycles = 4;
                let value = self.bus.read(address);
                self.ac = value;
                self.set_zero_and_negative_flag(value);
            }

            0xA1 => {
                // x-indexed zero page indirect
                let address = self.addr_zero_page_x_indirect();
                cycles = 6;
                let value = self.bus.read(address);
                self.ac = value;
                self.set_zero_and_negative_flag(value);
            }
            0xB1 => {
                let (address, p) = self.addr_zero_page_y_indirect();
                cycles = 5 + p;
                self.ac = self.bus.read(address);
                self.set_zero_and_negative_flag(self.ac);
            }
            // LDX
            0xA2 => {
                // immediate
                let value = self.bus.read(self.pc);
                self.inc_pc();
                cycles = 2;
                self.x = value;
                self.set_zero_and_negative_flag(self.x);
            }
            0xAE => {
                // absolute
                let address: u16 = self.addr_absolute();
                let value = self.bus.read(address);
                cycles = 4;
                self.x = value;
                self.set_zero_and_negative_flag(value);
            }
            0xBE => {
                // absolute indexed y
                let (address, p) = self.addr_absolute_y();
                cycles += 4 + p;
                let value = self.bus.read(address);
                self.x = value;
                self.set_zero_and_negative_flag(value);
            }
            0xA6 => {
                // zero page
                let address = self.addr_zero_page();
                cycles = 3;
                let value = self.bus.read(address);
                self.x = value;
                self.set_zero_and_negative_flag(value);
            }
            0xB6 => {
                // zero page indexed y
                let address = self.addr_zero_page_y();
                cycles = 4;
                let value = self.bus.read(address);
                self.x = value;
                self.set_zero_and_negative_flag(value);
            }
            // LDY (M -> Y)
            0xA0 => {
                // immediate
                let value = self.bus.read(self.pc);
                self.inc_pc();
                cycles = 2;
                self.y = value;
                self.set_zero_and_negative_flag(self.y);
            }
            0xAC => {
                // absolute
                let address: u16 = self.addr_absolute();
                let value = self.bus.read(address);
                cycles = 4;
                self.y = value;
                self.set_zero_and_negative_flag(value);
            }
            0xBC => {
                // absolute indexed y
                let (address, p) = self.addr_absolute_x();
                cycles += 4 + p;
                let value = self.bus.read(address);
                self.y = value;
                self.set_zero_and_negative_flag(value);
            }
            0xA4 => {
                // zero page
                let address = self.addr_zero_page();
                cycles = 3;
                let value = self.bus.read(address);
                self.y = value;
                self.set_zero_and_negative_flag(value);
            }
            0xB4 => {
                // x indexed zero page
                let address = self.addr_zero_page_x();
                cycles = 4;
                let value = self.bus.read(address);
                self.y = value;
                self.set_zero_and_negative_flag(value);
            }
            // STA - A -> M
            0x8D => {
                let address = self.addr_absolute();
                cycles = 4;
                self.bus.write(address, self.ac);
            } // absolute
            0x9D => {
                let (address, _) = self.addr_absolute_x();
                cycles = 5;
                self.bus.write(address, self.ac);
            } // absolute-x
            0x99 => {
                let (address, _) = self.addr_absolute_y();
                cycles = 5;
                self.bus.write(address, self.ac);
            } // absolute-y
            0x85 => {
                let address = self.addr_zero_page();
                cycles = 3;
                self.bus.write(address, self.ac);
            } // zero page
            0x95 => {
                let address = self.addr_zero_page_x();
                cycles = 4;
                self.bus.write(address, self.ac);
            } // x-indexed zero page
            0x81 => {
                let address = self.addr_zero_page_x_indirect();
                cycles = 6;
                self.bus.write(address, self.ac);
            } // y-indexed zero page
            0x91 => {
                let (address, _) = self.addr_zero_page_y_indirect();
                cycles = 6;
                self.bus.write(address, self.ac);
            } // zero page indirect y-indexed
            // STX: X -> M
            0x8E => {
                let address = self.addr_absolute();
                cycles = 3;
                self.bus.write(address, self.x);
            } // absolute
            0x86 => {
                let address = self.addr_zero_page();
                cycles = 2;
                self.bus.write(address, self.x);
            } // zero page
            0x96 => {
                let address = self.addr_zero_page_y();
                cycles = 2;
                self.bus.write(address, self.x);
            } // y-indexed zero page
            // STY: Y -> M
            0x8C => {
                let address = self.addr_absolute();
                cycles = 4;
                self.bus.write(address, self.y);
            } // absolute
            0x84 => {
                let address = self.addr_zero_page();
                cycles = 3;
                self.bus.write(address, self.y);
            } // zero page
            0x94 => {
                let address = self.addr_zero_page_x();
                cycles = 4;
                self.bus.write(address, self.y);
            } // x-indexed zero page
            // TAX: A -> X
            0xAA => {
                self.x = self.ac;
                self.set_zero_and_negative_flag(self.x);
            }
            // TAY: A -> Y
            0xA8 => {
                self.y = self.ac;
                self.set_zero_and_negative_flag(self.y);
            }
            // TSX: S -> X
            0xBA => {
                self.x = self.sp;
                self.set_zero_and_negative_flag(self.x);
            }
            // TXA: X -> A
            0x8A => {
                self.ac = self.x;
                self.set_zero_and_negative_flag(self.ac);
            }
            // TXS: X -> S
            0x9A => {
                self.sp = self.x;
            }
            // TYA: Y -> A
            0x98 => {
                self.ac = self.y;
                self.set_zero_and_negative_flag(self.ac);
            }
            // PHA: A -> Stack
            0x48 => {
                self.push_stack(self.ac);
            }
            // PHP: S -> Stack
            0x08 => self.push_stack(self.sr.bits()),
            // PLA: Stack[SP+1] -> A
            0x68 => {
                let val = self.pull_stack();
                self.ac = val;
                self.set_zero_and_negative_flag(self.ac);
            }
            // PLP: Stack[SP+1] -> SR
            0x28 => {
                self.sr = CpuFlags::from_bits_truncate(self.pull_stack());
            }
            // ASL A: C <- M7..M0 <- 0
            0x0A => {
                let value = self.ac;
                let (new_value, carry_flag) = Self::asl(value);
                self.ac = new_value;
                self.set_flag(CpuFlags::CARRY, carry_flag);
                self.set_zero_and_negative_flag(new_value);
            }
            // ASL $nnnn: C <- M7..M0 <- 0
            0x0E => {
                let address = self.addr_absolute();
                let value = self.bus.read(address);
                let (new_value, carry_flag) = Self::asl(value);
                self.bus.write(address, new_value);
                self.set_flag(CpuFlags::CARRY, carry_flag);
                self.set_zero_and_negative_flag(new_value);
            }
            // ASL $nnnn, X
            0x1E => {
                let (address, _) = self.addr_absolute_x();
                let value = self.bus.read(address);
                let (new_value, carry_flag) = Self::asl(value);
                self.bus.write(address, new_value);
                self.set_flag(CpuFlags::CARRY, carry_flag);
                self.set_zero_and_negative_flag(new_value);
            }
            // ASL $nn
            0x06 => {
                let address = self.addr_zero_page();
                let value = self.bus.read(address);
                let (new_value, carry_flag) = Self::asl(value);
                self.bus.write(address, new_value);
                self.set_flag(CpuFlags::CARRY, carry_flag);
                self.set_zero_and_negative_flag(new_value);
            }
            // ASL $nn, X
            0x16 => {
                let address = self.addr_zero_page_x();
                let value = self.bus.read(address);
                let (new_value, carry_flag) = Self::asl(value);
                self.bus.write(address, new_value);
                self.set_flag(CpuFlags::CARRY, carry_flag);
                self.set_zero_and_negative_flag(new_value);
            }
            // LSR A
            0x4A => {
                let value = self.get_a();
                let (new_value, carry) = Self::lsr(value);
                self.set_a(new_value);
                self.set_flag(CpuFlags::CARRY, carry);
                self.set_zero_and_negative_flag(new_value);
            } // accumulator
            // LSR $nnnn
            0x4E => {
                let address = self.addr_absolute();
                let value = self.bus.read(address);
                let (new_value, carry) = Self::lsr(value);
                self.bus.write(address, new_value);
                self.set_flag(CpuFlags::CARRY, carry);
                self.set_zero_and_negative_flag(new_value);
            } // absolute
            // LSR $nnnn, X
            0x5E => {
                let (address, _) = self.addr_absolute_x();
                let value = self.bus.read(address);
                let (new_value, carry) = Self::lsr(value);
                self.bus.write(address, new_value);
                self.set_flag(CpuFlags::CARRY, carry);
                self.set_zero_and_negative_flag(new_value);
            } // absolute x
            // LSR $nn
            0x46 => {
                let address = self.addr_zero_page();
                let value = self.bus.read(address);
                let (new_value, carry) = Self::lsr(value);
                self.bus.write(address, new_value);
                self.set_flag(CpuFlags::CARRY, carry);
                self.set_zero_and_negative_flag(new_value);
            } // zero page
            // LSR $nn, X
            0x56 => {
                let address = self.addr_zero_page_x();
                let value = self.bus.read(address);
                let (new_value, carry) = Self::lsr(value);
                self.bus.write(address, new_value);
                self.set_flag(CpuFlags::CARRY, carry);
                self.set_zero_and_negative_flag(new_value);
            } // zero page x-indexed
            other => panic!("Invalid opcode: {other}"),
        }

        cycles
    }

    fn asl(value: u8) -> (u8, bool) {
        let new_value = value << 1;
        let carry_flag = value & 0x80 != 0;
        (new_value, carry_flag)
    }

    fn lsr(value: u8) -> (u8, bool) {
        let new_value = value >> 1;
        let carry_flag = value & 0x01 != 0;
        (new_value, carry_flag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A mock memory bus for testing. It's just a simple RAM array.
    struct MockBus {
        mem: [u8; 0x10000],
    }

    impl MockBus {
        fn new() -> Self {
            MockBus { mem: [0; 0x10000] }
        }
        fn load(&mut self, addr: u16, bytes: &[u8]) {
            let mut a = addr as usize;
            for &b in bytes {
                self.mem[a] = b;
                a += 1;
            }
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
        CPU::new(MockBus::new())
    }

    const START: u16 = 0x8000;

    fn run_one(cpu: &mut CPU<MockBus>, prog: &[u8]) -> u64 {
        cpu.bus.load(START, prog);
        cpu.set_pc(START);
        cpu.step()
    }

    fn flags(p: u8) -> (bool, bool, bool, bool, bool, bool, bool) {
        // N V - B D I Z C
        (
            (p & 0x80) != 0,
            (p & 0x20) != 0,
            (p & 0x10) != 0,
            (p & 0x08) != 0,
            (p & 0x04) != 0,
            (p & 0x02) != 0,
            (p & 0x01) != 0,
        )
    }

    // ------------------------
    // Construction & helpers
    // ------------------------
    #[test]
    fn test_construct_cpu() {
        let cpu = setup_cpu();
        assert_eq!(cpu.get_pc(), 0u16);
        assert_eq!(cpu.get_sp(), 0xFF); // matches CPU::new
        assert_eq!(cpu.get_y(), 0);
        assert_eq!(cpu.get_x(), 0);
        assert_eq!(cpu.get_a(), 0);
        assert_eq!(
            cpu.get_p(),
            (CpuFlags::INTERRUPT_DISABLE | CpuFlags::UNUSED).bits()
        );
    }

    #[test]
    fn test_addr_absolute() {
        let mut cpu = setup_cpu();
        cpu.set_pc(0x1000);
        cpu.bus.write(0x1000, 0x34); // LSB
        cpu.bus.write(0x1001, 0x12); // MSB
        let addr = cpu.addr_absolute();
        assert_eq!(addr, 0x1234);
        assert_eq!(cpu.get_pc(), 0x1002);
    }

    #[test]
    fn test_addr_absolute_x_no_page_cross() {
        let mut cpu = setup_cpu();
        cpu.set_pc(0x1000);
        cpu.bus.write(0x1000, 0xFD);
        cpu.bus.write(0x1001, 0x00);
        cpu.set_x(0x01);
        let (addr, extra) = cpu.addr_absolute_x();
        assert_eq!(addr, 0x00FE);
        assert_eq!(extra, 0);
        assert_eq!(cpu.get_pc(), 0x1002);
    }

    #[test]
    fn test_addr_absolute_x_page_cross() {
        let mut cpu = setup_cpu();
        cpu.set_pc(0x1000);
        cpu.bus.write(0x1000, 0xFF);
        cpu.bus.write(0x1001, 0x00);
        cpu.set_x(0x01);
        let (addr, extra) = cpu.addr_absolute_x();
        assert_eq!(addr, 0x0100);
        assert_eq!(extra, 1);
        assert_eq!(cpu.get_pc(), 0x1002);
    }

    #[test]
    fn test_addr_absolute_y_no_page_cross() {
        let mut cpu = setup_cpu();
        cpu.set_pc(0x1000);
        cpu.bus.write(0x1000, 0xFD);
        cpu.bus.write(0x1001, 0x00);
        cpu.set_y(0x01);
        let (addr, extra) = cpu.addr_absolute_y();
        assert_eq!(addr, 0x00FE);
        assert_eq!(extra, 0);
        assert_eq!(cpu.get_pc(), 0x1002);
    }

    #[test]
    fn test_addr_absolute_y_page_cross() {
        let mut cpu = setup_cpu();
        cpu.set_pc(0x1000);
        cpu.bus.write(0x1000, 0xFF);
        cpu.bus.write(0x1001, 0x00);
        cpu.set_y(0x01);
        let (addr, extra) = cpu.addr_absolute_y();
        assert_eq!(addr, 0x0100);
        assert_eq!(extra, 1);
        assert_eq!(cpu.get_pc(), 0x1002);
    }

    #[test]
    fn test_addr_absolute_indirect_normal() {
        let mut cpu = setup_cpu();
        cpu.set_pc(0x1000);
        cpu.bus.write(0x1000, 0xFD);
        cpu.bus.write(0x1001, 0x12);
        cpu.bus.write(0x12FD, 0x21);
        cpu.bus.write(0x12FE, 0x23);
        cpu.addr_absolute_indirect();
        assert_eq!(cpu.get_pc(), 0x2321);
    }

    #[test]
    fn test_addr_absolute_indirect_bug() {
        let mut cpu = setup_cpu();
        cpu.set_pc(0x1000);
        cpu.bus.write(0x1000, 0xFF);
        cpu.bus.write(0x1001, 0x12);
        cpu.bus.write(0x12FF, 0x21);
        cpu.bus.write(0x1200, 0x23);
        cpu.addr_absolute_indirect();
        assert_eq!(cpu.get_pc(), 0x2321);
    }

    #[test]
    fn test_addr_zero_page() {
        let mut cpu = setup_cpu();
        cpu.set_pc(0x1000);
        cpu.bus.write(0x1000, 0x23);
        let addr = cpu.addr_zero_page();
        assert_eq!(addr, 0x0023);
    }

    #[test]
    fn test_addr_zero_page_x() {
        let mut cpu = setup_cpu();
        cpu.set_pc(0x1000);
        cpu.bus.write(0x1000, 0xFD);
        cpu.set_x(0x04);
        let addr = cpu.addr_zero_page_x();
        assert_eq!(addr, 0x0001);
    }

    #[test]
    fn test_addr_zero_page_y() {
        let mut cpu = setup_cpu();
        cpu.set_pc(0x1000);
        cpu.bus.write(0x1000, 0xFD);
        cpu.set_y(0x04);
        let addr = cpu.addr_zero_page_y();
        assert_eq!(addr, 0x0001);
    }

    #[test]
    fn test_addr_zero_page_x_indirect() {
        let mut cpu = setup_cpu();
        cpu.set_pc(0x1000);
        cpu.set_x(0x02);
        cpu.bus.write(0x1000, 0xFC);
        cpu.bus.write(0x00FE, 0x34); // LSB
        cpu.bus.write(0x00FF, 0x12); // MSB
        let addr = cpu.addr_zero_page_x_indirect();
        assert_eq!(addr, 0x1234);
    }

    #[test]
    fn test_addr_zero_page_y_indirect() {
        let mut cpu = setup_cpu();
        cpu.set_pc(0x0001);
        cpu.set_y(0x01);
        cpu.bus.write(0x0001, 0xAB);
        cpu.bus.write(0x00AB, 0xFF);
        cpu.bus.write(0x00AC, 0x02);
        let (addr, extra) = cpu.addr_zero_page_y_indirect();
        assert_eq!(addr, 0x0300);
        assert_eq!(extra, 1);
    }

    #[test]
    fn test_addr_relative_positive_offset() {
        let mut cpu = setup_cpu();
        cpu.set_pc(0x1000);
        cpu.bus.write(0x1000, 0x0A);
        let (effective, extra) = cpu.addr_relative();
        assert_eq!(effective, cpu.get_pc());
        assert_eq!(extra, 0);
        assert_eq!(cpu.get_pc(), 0x100B);
    }

    #[test]
    fn test_addr_relative_negative_offset() {
        let mut cpu = setup_cpu();
        cpu.set_pc(0x1000);
        cpu.bus.write(0x1000, (-15i8) as u8);
        let (effective, extra) = cpu.addr_relative();
        assert_eq!(effective, cpu.get_pc());
        assert_eq!(extra, 1);
        assert_eq!(cpu.get_pc(), 0x0FF2);
    }

    // ------------------------
    // Instruction tests (documented 6502)
    // We add tests for instructions currently implemented in step();
    // the rest are scaffolded and marked #[ignore] to enable TDD.
    // Reference opcode table: https://www.pagetable.com/c64ref/6502/?tab=2
    // ------------------------

    // LDA
    #[test]
    fn lda_imm_sets_nz() {
        let mut cpu = setup_cpu();
        let _ = run_one(&mut cpu, &[0xA9, 0x80]); // LDA #$80
        assert_eq!(cpu.get_a(), 0x80);
        let (n, _v, _b, _d, _i, z, _c) = flags(cpu.get_p());
        assert!(n && !z);
    }

    #[test]
    fn lda_abs_reads_memory() {
        let mut cpu = setup_cpu();
        cpu.bus.write(0x1234, 0x55);
        let _ = run_one(&mut cpu, &[0xAD, 0x34, 0x12]);
        assert_eq!(cpu.get_a(), 0x55);
    }

    #[test]
    fn lda_abs_x_page_cross_affects_cycles() {
        let mut cpu = setup_cpu();
        cpu.set_x(0x01);
        cpu.bus.write(0x0100, 0x99);
        let cycles = run_one(&mut cpu, &[0xBD, 0xFF, 0x00]); // LDA $00FF,X
        assert_eq!(cpu.get_a(), 0x99);
        assert!(cycles >= 5); // 4 + page-cross penalty implemented as +1
    }

    #[test]
    fn lda_abs_y_page_cross_affects_cycles() {
        let mut cpu = setup_cpu();
        cpu.set_y(0x01);
        cpu.bus.write(0x0100, 0x42);
        let cycles = run_one(&mut cpu, &[0xB9, 0xFF, 0x00]); // LDA $00FF,Y
        assert_eq!(cpu.get_a(), 0x42);
        assert!(cycles >= 5);
    }

    #[test]
    fn lda_zp_and_zpx() {
        let mut cpu = setup_cpu();
        cpu.bus.write(0x0002, 0x11);
        let _ = run_one(&mut cpu, &[0xA5, 0x02]);
        assert_eq!(cpu.get_a(), 0x11);
        cpu.set_x(1);
        cpu.bus.write(0x0004, 0x22);
        let _ = run_one(&mut cpu, &[0xB5, 0x03]);
        assert_eq!(cpu.get_a(), 0x22);
    }

    #[test]
    fn lda_x_indirect() {
        let mut cpu = setup_cpu();
        cpu.set_x(0x04);
        // operand at $8001: base 0x20; add X -> 0x24; pointer [0x24..0x25] -> 0x1234
        cpu.bus.write(START + 1, 0x20);
        cpu.bus.write(0x0024, 0x34);
        cpu.bus.write(0x0025, 0x12);
        cpu.bus.write(0x1234, 0xAB);
        let _ = run_one(&mut cpu, &[0xA1, 0x20]);
        assert_eq!(cpu.get_a(), 0xAB);
    }

    #[test]
    fn lda_indirect_y() {
        let mut cpu = setup_cpu();
        // operand at $8001 = 0x20; pointer [0x20..0x21] = 0x12FF; Y=1 => 0x1300
        cpu.bus.write(START + 1, 0x20);
        cpu.bus.write(0x0020, 0xFF);
        cpu.bus.write(0x0021, 0x12);
        cpu.set_y(1);
        cpu.bus.write(0x1300, 0xEE);
        let _ = run_one(&mut cpu, &[0xB1, 0x20]);
        assert_eq!(cpu.get_a(), 0xEE);
    }

    // LDX
    #[test]
    fn ldx_variants() {
        let mut cpu = setup_cpu();
        let _ = run_one(&mut cpu, &[0xA2, 0x7F]);
        assert_eq!(cpu.get_x(), 0x7F);
        cpu.bus.write(0x1234, 0x10);
        let _ = run_one(&mut cpu, &[0xAE, 0x34, 0x12]);
        assert_eq!(cpu.get_x(), 0x10);
        cpu.set_y(1);
        cpu.bus.write(0x0100, 0x44);
        let _ = run_one(&mut cpu, &[0xBE, 0xFF, 0x00]); // abs,Y
        assert_eq!(cpu.get_x(), 0x44);
        cpu.bus.write(0x0003, 0x55);
        let _ = run_one(&mut cpu, &[0xA6, 0x03]); // zp
        assert_eq!(cpu.get_x(), 0x55);
        cpu.set_y(1);
        cpu.bus.write(0x0005, 0x66);
        let _ = run_one(&mut cpu, &[0xB6, 0x04]); // zp,Y
        assert_eq!(cpu.get_x(), 0x66);
    }

    // LDY
    #[test]
    fn ldy_variants() {
        let mut cpu = setup_cpu();
        let _ = run_one(&mut cpu, &[0xA0, 0x01]); // imm
        assert_eq!(cpu.get_y(), 0x01);
        cpu.bus.write(0x1234, 0x22);
        let _ = run_one(&mut cpu, &[0xAC, 0x34, 0x12]); // abs
        assert_eq!(cpu.get_y(), 0x22);
        cpu.set_x(1);
        cpu.bus.write(0x0100, 0x33);
        let _ = run_one(&mut cpu, &[0xBC, 0xFF, 0x00]); // abs,X
        assert_eq!(cpu.get_y(), 0x33);
        cpu.bus.write(0x0002, 0x44);
        let _ = run_one(&mut cpu, &[0xA4, 0x02]); // zp
        assert_eq!(cpu.get_y(), 0x44);
        cpu.set_x(1);
        cpu.bus.write(0x0004, 0x55);
        let _ = run_one(&mut cpu, &[0xB4, 0x03]); // zp,X
        assert_eq!(cpu.get_y(), 0x55);
    }

    // Stores (STA/STX/STY)
    #[test]
    fn sta_variants_write_memory() {
        let mut cpu = setup_cpu();
        cpu.set_a(0xAA);
        let _ = run_one(&mut cpu, &[0x8D, 0x34, 0x12]); // abs
        assert_eq!(cpu.bus.read(0x1234), 0xAA);
        cpu.set_x(1);
        cpu.set_a(0xBB);
        let _ = run_one(&mut cpu, &[0x9D, 0xFF, 0x00]); // abs,X
        assert_eq!(cpu.bus.read(0x0100), 0xBB);
        cpu.set_y(1);
        cpu.set_a(0xCC);
        let _ = run_one(&mut cpu, &[0x99, 0xFF, 0x00]); // abs,Y
        assert_eq!(cpu.bus.read(0x0100), 0xCC);
        cpu.set_a(0x11);
        let _ = run_one(&mut cpu, &[0x85, 0x02]); // zp
        assert_eq!(cpu.bus.read(0x0002), 0x11);
        cpu.set_x(1);
        cpu.set_a(0x22);
        let _ = run_one(&mut cpu, &[0x95, 0x03]); // zp,X
        assert_eq!(cpu.bus.read(0x0004), 0x22);
    }

    #[test]
    fn sta_x_indirect_and_indirect_y() {
        let mut cpu = setup_cpu();
        // (zp,X)
        cpu.set_a(0x33);
        cpu.set_x(2);
        cpu.bus.write(START + 1, 0x20);
        cpu.bus.write(0x0022, 0x34);
        cpu.bus.write(0x0023, 0x12);
        let _ = run_one(&mut cpu, &[0x81, 0x20]);
        assert_eq!(cpu.bus.read(0x1234), 0x33);
        // (zp),Y
        cpu.set_a(0x44);
        cpu.set_y(1);
        cpu.bus.write(START + 1, 0x30);
        cpu.bus.write(0x0030, 0xFF);
        cpu.bus.write(0x0031, 0x12);
        let _ = run_one(&mut cpu, &[0x91, 0x30]);
        assert_eq!(cpu.bus.read(0x1300), 0x44);
    }

    // Transfers
    #[test]
    fn transfer_ops_update_flags() {
        let mut cpu = setup_cpu();
        cpu.set_a(0x80);
        let _ = run_one(&mut cpu, &[0xAA]); // TAX
        assert_eq!(cpu.get_x(), 0x80);
        cpu.set_a(0x7F);
        let _ = run_one(&mut cpu, &[0xA8]); // TAY
        assert_eq!(cpu.get_y(), 0x7F);
        cpu.set_x(0x01);
        let _ = run_one(&mut cpu, &[0x8A]); // TXA
        assert_eq!(cpu.get_a(), 0x01);
        cpu.set_y(0x00);
        let _ = run_one(&mut cpu, &[0x98]); // TYA
        assert_eq!(cpu.get_a(), 0x00);
        let _ = run_one(&mut cpu, &[0xBA]); // TSX
        assert_eq!(cpu.get_x(), cpu.get_sp());
        cpu.set_x(0xFD);
        let _ = run_one(&mut cpu, &[0x9A]); // TXS
        assert_eq!(cpu.get_sp(), 0xFD);
    }

    // Stack ops
    #[test]
    fn stack_push_pull() {
        let mut cpu = setup_cpu();
        cpu.set_a(0x12);
        let _ = run_one(&mut cpu, &[0x48]); // PHA
        assert_eq!(cpu.peek_stack(), 0x12);
        // PHP/PLP roundtrip
        let p0 = cpu.get_p();
        let _ = run_one(&mut cpu, &[0x08]); // PHP
                                            // overwrite P intentionally, then pull it back
        cpu.set_p(0);
        let _ = run_one(&mut cpu, &[0x28]); // PLP
        assert_eq!(cpu.get_p(), p0);
        // PLA restores A and flags
        cpu.set_a(0);
        let _ = run_one(&mut cpu, &[0x68]);
        assert_eq!(cpu.get_a(), 0x12);
    }

    #[test]
    fn asl_accumulator() {
        let mut cpu = setup_cpu();
        cpu.set_a(0x80);
        let _ = run_one(&mut cpu, &[0x0A]);
        assert_eq!(cpu.get_a(), 0x00);
        let (n, _v, _b, _d, _i, z, c) = flags(cpu.get_p());
        // 1000 0000 -> 0000 0000, C=1
        assert_eq!(n, false);
        assert_eq!(z, true);
        assert_eq!(c, true);
    }

    #[test]
    fn asl_abs() {
        let mut cpu = setup_cpu();
        cpu.bus.write(0x1234, 0x80);
        let _ = run_one(&mut cpu, &[0x0E, 0x34, 0x12]);
        assert_eq!(cpu.bus.read(0x1234), 0x00);
        let (n, _v, _b, _d, _i, z, c) = flags(cpu.get_p());
        assert_eq!(n, false);
        assert_eq!(z, true);
        assert_eq!(c, true);
    }

    #[test]
    fn asl_abs_x() {
        let mut cpu = setup_cpu();
        cpu.set_x(0x01);
        cpu.bus.write(0x1235, 0x80);
        let _ = run_one(&mut cpu, &[0x1E, 0x34, 0x12]);
        assert_eq!(cpu.bus.read(0x1235), 0x00);
        let (n, _v, _b, _d, _i, z, c) = flags(cpu.get_p());
        assert_eq!(n, false);
        assert_eq!(z, true);
        assert_eq!(c, true);
    }

    #[test]
    fn asl_zp() {
        let mut cpu = setup_cpu();
        cpu.bus.write(0x0002, 0x80);
        let _ = run_one(&mut cpu, &[0x06, 0x02]);
        assert_eq!(cpu.bus.read(0x0002), 0x00);
        let (n, _v, _b, _d, _i, z, c) = flags(cpu.get_p());
        assert_eq!(n, false);
        assert_eq!(z, true);
        assert_eq!(c, true);
    }

    #[test]
    fn asl_zp_x() {
        let mut cpu = setup_cpu();
        cpu.set_x(0x01);
        cpu.bus.write(0x0003, 0x80);
        let _ = run_one(&mut cpu, &[0x16, 0x02]);
        assert_eq!(cpu.bus.read(0x0003), 0x00);
        let (n, _v, _b, _d, _i, z, c) = flags(cpu.get_p());
        assert_eq!(n, false);
        assert_eq!(z, true);
        assert_eq!(c, true);
    }
    // LSR TESTS
    #[test]
    fn lsr_accumulator() {
        let mut cpu = setup_cpu();
        cpu.set_a(0x80);
        // 1000 0000 -> 0100 0000, C=0
        let _ = run_one(&mut cpu, &[0x4A]);
        assert_eq!(cpu.get_a(), 0x40);
        let (n, _v, _b, _d, _i, z, c) = flags(cpu.get_p());
        assert_eq!(n, false);
        assert_eq!(z, false);
        assert_eq!(c, false);
    }

    #[test]
    fn lsr_abs() {
        let mut cpu = setup_cpu();
        cpu.bus.write(0x1234, 0x80);
        let _ = run_one(&mut cpu, &[0x4E, 0x34, 0x12]);
        assert_eq!(cpu.bus.read(0x1234), 0x40);
        let (n, _v, _b, _d, _i, z, c) = flags(cpu.get_p());
        assert_eq!(n, false);
        assert_eq!(z, false);
        assert_eq!(c, false);
    }
    #[test]
    fn lsr_abs_x() {
        let mut cpu = setup_cpu();
        cpu.set_x(0x01);
        cpu.bus.write(0x1235, 0x80);
        let _ = run_one(&mut cpu, &[0x5E, 0x34, 0x12]);
        assert_eq!(cpu.bus.read(0x1235), 0x40);
        let (n, _v, _b, _d, _i, z, c) = flags(cpu.get_p());
        assert_eq!(n, false);
        assert_eq!(z, false);
        assert_eq!(c, false);
    }

    #[test]
    fn lsr_zp() {
        let mut cpu = setup_cpu();
        cpu.bus.write(0x0001, 0x80);
        let _ = run_one(&mut cpu, &[0x46, 0x01]);
        assert_eq!(cpu.bus.read(0x0001), 0x40);
        let (n, _v, _b, _d, _i, z, c) = flags(cpu.get_p());
        assert_eq!(n, false);
        assert_eq!(z, false);
        assert_eq!(c, false);
    }

    #[test]
    fn lsr_zp_x() {
        let mut cpu = setup_cpu();
        cpu.set_x(0x01);
        cpu.bus.write(0x0000, 0x80);
        let _ = run_one(&mut cpu, &[0x56, 0x0FF]);
        assert_eq!(cpu.bus.read(0x0000), 0x40);
        let (n, _v, _b, _d, _i, z, c) = flags(cpu.get_p());
        assert_eq!(n, false);
        assert_eq!(z, false);
        assert_eq!(c, false);
    }
}
