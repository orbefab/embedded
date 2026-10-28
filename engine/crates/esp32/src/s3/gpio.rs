//! GPIO, IO_MUX and the GPIO matrix's output side: which level each pad drives, and what
//! `GPIO_IN` reads.
//!
//! A pad drives from the matrix when its IO_MUX function is the GPIO one: function 1 on every
//! pad, or function 0 on a pad whose function 0 is its GPIO. The matrix's signal 256 is the
//! pad's own `GPIO_OUT` bit, with `GPIO_ENABLE` as its output enable. No peripheral output is
//! modeled yet, so a pad routed to any other signal, or to another IO_MUX function, drives
//! nothing. A pad that does not drive pulls up when `FUN_WPU` is set. `FUN_WPD` is kept and
//! not stamped, since the seam has no pull-down drive. `GPIO_PINn`, which holds the open-drain
//! bit and the interrupt setup, is not modeled.

/// Pads GPIO0 to GPIO48. GPIO22 to GPIO25 have no pad, and their bits do nothing.
pub(crate) const PADS: usize = 49;

/// The bits of the 54 the registers hold.
const BITS: u64 = (1 << 54) - 1;

/// `FUNCn_OUT_SEL_CFG`'s value for the pad's own `GPIO_OUT` bit.
const SIG_GPIO_OUT: u32 = 0x100;
const OUT_SEL: u32 = 0x1ff;
const OUT_INV_SEL: u32 = 1 << 9;
const OEN_SEL: u32 = 1 << 10;
const OEN_INV_SEL: u32 = 1 << 11;

const FUN_WPU: u32 = 1 << 8;
const FUN_IE: u32 = 1 << 9;
const MCU_SEL_SHIFT: u32 = 12;
const MCU_SEL: u32 = 0x7 << MCU_SEL_SHIFT;
/// `FUN_DRV` 2, no pull, input off and function 0, as every pad reads at power-on here. The
/// datasheet's power-on pulls and input enables are not modeled.
const IO_MUX_RESET: u32 = 0x800;

/// The pads whose IO_MUX function 0 is their GPIO, from IDF's `io_mux_reg.h`: 0 to 21, 33 to
/// 38, 45 and 46. Function 0 of the others is flash SPI, JTAG, UART0 or the SPI clock pair.
const GPIO_FUNCTION_0: u64 = 0x3fffff | (0x3f << 33) | (0x3 << 45);

/// What a pad does to its net.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadDrive {
    High,
    Low,
    PullUp,
    Off,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Gpio {
    out: u64,
    enable: u64,
    out_sel: [u32; 54],
    in_sel: [u32; 256],
    io_mux: [u32; PADS],
    /// The level each pad with a net reads, `None` for one with no net.
    pads: [Option<bool>; PADS],
}

/// A GPIO block register the model keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reg {
    Out(Word),
    Enable(Word),
    In(usize),
    OutSel(usize),
    InSel(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Word {
    Set(usize),
    W1ts(usize),
    W1tc(usize),
}

impl Gpio {
    /// The pads listed in `netted` have nets, and read low until the first input.
    pub(crate) fn new(netted: &[u8]) -> Gpio {
        let mut pads = [None; PADS];
        for &gpio in netted {
            pads[usize::from(gpio)] = Some(false);
        }
        Gpio {
            out: 0,
            enable: 0,
            out_sel: [SIG_GPIO_OUT; 54],
            in_sel: [0; 256],
            io_mux: [IO_MUX_RESET; PADS],
            pads,
        }
    }

    /// A register at `offset` in the GPIO block, or `None` for one the model does not keep.
    fn reg(offset: u32) -> Option<Reg> {
        let word = |base: u32, half: usize| match offset - base {
            0 => Word::Set(half),
            4 => Word::W1ts(half),
            8 => Word::W1tc(half),
            _ => unreachable!(),
        };
        Some(match offset {
            0x04..=0x0c => Reg::Out(word(0x04, 0)),
            0x10..=0x18 => Reg::Out(word(0x10, 1)),
            0x20..=0x28 => Reg::Enable(word(0x20, 0)),
            0x2c..=0x34 => Reg::Enable(word(0x2c, 1)),
            0x3c => Reg::In(0),
            0x40 => Reg::In(1),
            0x154..=0x550 => Reg::InSel(((offset - 0x154) / 4) as usize),
            0x554..=0x628 => Reg::OutSel(((offset - 0x554) / 4) as usize),
            _ => return None,
        })
    }

    /// The GPIO block register at `offset`, a multiple of 4, or `None` for one not kept.
    pub(crate) fn read(&self, offset: u32) -> Option<u32> {
        let half = |bits: u64, half: usize| (bits >> (32 * half)) as u32;
        Some(match Self::reg(offset)? {
            Reg::Out(Word::Set(k)) => half(self.out, k),
            Reg::Enable(Word::Set(k)) => half(self.enable, k),
            Reg::Out(_) | Reg::Enable(_) => 0,
            Reg::In(k) => half(self.levels(), k),
            Reg::OutSel(n) => self.out_sel[n],
            Reg::InSel(n) => self.in_sel[n],
        })
    }

    /// Writes the register at `offset`. `false` for one not kept, which drops the write.
    pub(crate) fn write(&mut self, offset: u32, value: u32) -> bool {
        let Some(reg) = Self::reg(offset) else {
            return false;
        };
        let apply = |bits: &mut u64, word: Word| {
            let (k, op) = match word {
                Word::Set(k) => (k, 0),
                Word::W1ts(k) => (k, 1),
                Word::W1tc(k) => (k, 2),
            };
            let shift = 32 * k;
            let mask = (u64::from(u32::MAX) << shift) & BITS;
            let value = (u64::from(value) << shift) & mask;
            *bits = match op {
                0 => (*bits & !mask) | value,
                1 => *bits | value,
                _ => *bits & !value,
            };
        };
        match reg {
            Reg::Out(word) => apply(&mut self.out, word),
            Reg::Enable(word) => apply(&mut self.enable, word),
            Reg::In(_) => {}
            Reg::OutSel(n) => self.out_sel[n] = value & 0xfff,
            Reg::InSel(n) => self.in_sel[n] = value & 0xff,
        }
        true
    }

    /// The IO_MUX register at `offset` in its block, a pad's for 0x04 to 0xc4.
    pub(crate) fn read_io_mux(&self, offset: u32) -> Option<u32> {
        Some(self.io_mux[Self::io_mux_pad(offset)?])
    }

    pub(crate) fn write_io_mux(&mut self, offset: u32, value: u32) -> bool {
        let Some(pad) = Self::io_mux_pad(offset) else {
            return false;
        };
        self.io_mux[pad] = value & 0xffff;
        true
    }

    fn io_mux_pad(offset: u32) -> Option<usize> {
        let pad = (offset.checked_sub(4)? / 4) as usize;
        (pad < PADS).then_some(pad)
    }

    /// The level a pad with a net reads from `at` on. A pad with no net ignores it.
    pub(crate) fn set_pad(&mut self, gpio: u8, level: bool) {
        let pad = &mut self.pads[usize::from(gpio)];
        debug_assert!(pad.is_some(), "GPIO{gpio} has a net");
        *pad = Some(level);
    }

    /// Whether the pad takes its output from the GPIO matrix.
    fn matrix_drives(&self, pad: usize) -> bool {
        match (self.io_mux[pad] & MCU_SEL) >> MCU_SEL_SHIFT {
            1 => true,
            0 => GPIO_FUNCTION_0 >> pad & 1 != 0,
            _ => false,
        }
    }

    /// The output enable and the level the pad drives when enabled.
    fn output(&self, pad: usize) -> (bool, bool) {
        let sel = self.out_sel[pad];
        if !self.matrix_drives(pad) || sel & OUT_SEL != SIG_GPIO_OUT {
            return (false, false);
        }
        let enable = (self.enable >> pad & 1 != 0) ^ (sel & OEN_SEL != 0 && sel & OEN_INV_SEL != 0);
        let level = (self.out >> pad & 1 != 0) ^ (sel & OUT_INV_SEL != 0);
        (enable, level)
    }

    pub(crate) fn drive(&self, pad: usize) -> PadDrive {
        match self.output(pad) {
            (true, true) => PadDrive::High,
            (true, false) => PadDrive::Low,
            (false, _) if self.io_mux[pad] & FUN_WPU != 0 => PadDrive::PullUp,
            (false, _) => PadDrive::Off,
        }
    }

    /// `GPIO_IN` and `GPIO_IN1` as one word. A pad with its input disabled reads 0. One with a
    /// net reads its net. One with no net reads what it drives, else its pull-up.
    fn levels(&self) -> u64 {
        (0..PADS)
            .filter(|&pad| self.io_mux[pad] & FUN_IE != 0)
            .filter(|&pad| match self.pads[pad] {
                Some(level) => level,
                None => match self.output(pad) {
                    (true, level) => level,
                    (false, _) => self.io_mux[pad] & FUN_WPU != 0,
                },
            })
            .fold(0, |bits, pad| bits | 1 << pad)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GPIO_FUNC: u32 = 1 << MCU_SEL_SHIFT;

    fn gpio21() -> Gpio {
        let mut gpio = Gpio::new(&[21]);
        gpio.write_io_mux(0x4 + 4 * 21, GPIO_FUNC | FUN_IE);
        gpio
    }

    #[test]
    fn a_pad_drives_its_out_bit_once_enabled() {
        let mut gpio = gpio21();
        assert_eq!(gpio.drive(21), PadDrive::Off);
        gpio.write(0x08, 1 << 21);
        assert_eq!(gpio.drive(21), PadDrive::Off);
        gpio.write(0x24, 1 << 21);
        assert_eq!(gpio.drive(21), PadDrive::High);
        gpio.write(0x0c, 1 << 21);
        assert_eq!(gpio.drive(21), PadDrive::Low);
        assert_eq!(gpio.read(0x04), Some(0));
        assert_eq!(gpio.read(0x20), Some(1 << 21));
    }

    #[test]
    fn the_high_word_reaches_gpio32_and_up() {
        let mut gpio = Gpio::new(&[]);
        gpio.write_io_mux(0x4 + 4 * 40, GPIO_FUNC);
        gpio.write(0x30, 1 << 8);
        gpio.write(0x14, 1 << 8);
        assert_eq!(gpio.drive(40), PadDrive::High);
        assert_eq!(gpio.read(0x10), Some(1 << 8));
    }

    #[test]
    fn a_pad_reads_its_net_and_one_with_no_net_reads_itself() {
        let mut gpio = gpio21();
        gpio.set_pad(21, true);
        assert_eq!(gpio.read(0x3c), Some(1 << 21));
        gpio.write_io_mux(0x4 + 4 * 21, GPIO_FUNC);
        assert_eq!(gpio.read(0x3c), Some(0), "input disabled");

        gpio.write_io_mux(0x4 + 4 * 5, GPIO_FUNC | FUN_IE | FUN_WPU);
        assert_eq!(gpio.read(0x3c), Some(1 << 5), "pulled up");
        assert_eq!(gpio.drive(5), PadDrive::PullUp);
        gpio.write(0x24, 1 << 5);
        assert_eq!(gpio.read(0x3c), Some(0), "drives its own low");
    }

    #[test]
    fn another_function_or_signal_drives_nothing() {
        let mut gpio = Gpio::new(&[]);
        gpio.write(0x30, 1 << (43 - 32));
        gpio.write(0x14, 1 << (43 - 32));
        assert_eq!(
            gpio.drive(43),
            PadDrive::Off,
            "function 0 of GPIO43 is U0TXD"
        );
        gpio.write_io_mux(0x4 + 4 * 43, GPIO_FUNC);
        assert_eq!(gpio.drive(43), PadDrive::High);
        gpio.write(0x554 + 4 * 43, 12);
        assert_eq!(gpio.drive(43), PadDrive::Off, "a peripheral signal");
        gpio.write(0x554 + 4 * 43, 12 | OEN_SEL);
        assert_eq!(
            gpio.drive(43),
            PadDrive::Off,
            "a peripheral signal, enabled from GPIO_ENABLE"
        );
        gpio.write(0x554 + 4 * 43, SIG_GPIO_OUT | OUT_INV_SEL);
        assert_eq!(gpio.drive(43), PadDrive::Low, "inverted");
    }

    #[test]
    fn oen_inv_sel_inverts_gpio_enable_only_under_oen_sel() {
        let mut gpio = Gpio::new(&[]);
        gpio.write_io_mux(0x4 + 4 * 43, GPIO_FUNC);
        gpio.write(0x14, 1 << (43 - 32));
        gpio.write(0x554 + 4 * 43, SIG_GPIO_OUT | OEN_INV_SEL);
        assert_eq!(gpio.drive(43), PadDrive::Off, "GPIO_ENABLE clear");
        gpio.write(0x554 + 4 * 43, SIG_GPIO_OUT | OEN_SEL | OEN_INV_SEL);
        assert_eq!(
            gpio.drive(43),
            PadDrive::High,
            "GPIO_ENABLE clear, inverted"
        );
        gpio.write(0x30, 1 << (43 - 32));
        assert_eq!(gpio.drive(43), PadDrive::Off, "GPIO_ENABLE set, inverted");
    }
}
