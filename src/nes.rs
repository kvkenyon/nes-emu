use crate::bus::Bus;
use crate::cpu::CPU;

pub struct NES {
    pub cpu: CPU<Bus>,
}

#[cfg(test)]
mod tests {
    use crate::bus::Bus;

    use super::*;
    #[test]
    fn construct_nes() {
        let cpu = CPU::new(Bus::new());
        let _ = NES { cpu };
    }
}
