//! The bootloader a board burns into the boot section, as what it does on the way to the
//! sketch: the cycles it takes and the registers it leaves. The model does not run its code.
//!
//! Optiboot is the one Arduino's `boards.txt` (avr 1.8.6) burns for the Uno and for the Nano's
//! `cpu=atmega328`, `bootloaders/optiboot/optiboot_atmega328.hex`, linked at byte `0x7E00`
//! (`bootloaders/optiboot/Makefile`, target `atmega328`). Its `main` in `optiboot.c` reads
//! `MCUSR`, clears it, and jumps to the sketch unless `EXTRF` is set ("Adaboot no-wait mod"). On
//! the way it turns the watchdog off (`appStart`, `watchdogConfig(WATCHDOG_OFF)`). The cycle counts
//! come from `optiboot_atmega328.lst` and the AVRe column of DS40002198B.
//!
//! A reset with `EXTRF` set, from the `RESET` pin, waits for an upload. With nothing on RXD,
//! Optiboot sets up USART0 and a 1 s watchdog, flashes the L LED on `PB5` three times, and polls
//! `RXC0` until the watchdog resets the chip, which then goes straight to the sketch. The model
//! makes each of its bus accesses on its own cycle, as [`Wait`] lists them. A byte that arrives
//! is the start of an upload, which the model does not run.

/// Which bootloader the boot section holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Bootloader {
    /// None. With `BOOTRST` unprogrammed the reset vector is word 0, and the netlist refuses a
    /// programmed one.
    #[default]
    None,
    /// Optiboot, Arduino's build for the ATmega328P at 16 MHz.
    Optiboot,
}

/// Optiboot's first word, byte `0x7E00` (`--section-start=.text=0x7e00`).
pub const OPTIBOOT_WORD: u16 = 0x3F00;

/// The cycles from Optiboot's first instruction to the sketch's first fetch at word 0, after a
/// reset without `EXTRF`, from `optiboot_atmega328.lst`:
///
/// | Address | Instruction | Cycles |
/// | --- | --- | --- |
/// | `7e00` | `eor r1, r1` | 1 |
/// | `7e02` | `in r24, MCUSR` | 1 |
/// | `7e04` | `out MCUSR, r1` | 1 |
/// | `7e06` | `sbrs r24, 1`, no skip with `EXTRF` clear | 1 |
/// | `7e08` | `rcall appStart` | 3 |
/// | `7fea` | `ldi r24, 0` | 1 |
/// | `7fec` | `rcall watchdogConfig` | 3 |
/// | `7fbe` | `ldi r30`, `ldi r31`, `ldi r25` | 3 |
/// | `7fc4` | `st Z, r25`, `WDTCSR = WDCE \| WDE` | 2 |
/// | `7fc6` | `st Z, r24`, `WDTCSR = 0` | 2 |
/// | `7fc8` | `ret` | 4 |
/// | `7fee` | `eor r30, r30`, `eor r31, r31` | 2 |
/// | `7ff2` | `ijmp` to word 0 | 2 |
pub const OPTIBOOT_TO_APP: u64 = 26;

/// The cycle, counted from Optiboot's first instruction, on which each of its stores retires:
/// `out MCUSR` at 3, the two `st` to `WDTCSR` at 16 and 18.
pub const OPTIBOOT_MCUSR_AT: u64 = 3;
pub const OPTIBOOT_WDCE_AT: u64 = 16;
pub const OPTIBOOT_WDT_OFF_AT: u64 = 18;

impl Bootloader {
    /// The cycles from the reset vector at `boot_word` to the sketch's first fetch, when the
    /// bootloader goes straight to the sketch. The boot section can start below Optiboot: the
    /// Nano's high fuse `0xDA` puts it at word `0x3C00`, and the 768 words up to Optiboot are
    /// erased flash. DS40002198B lists no instruction for the erased word `0xFFFF`. The model
    /// counts one cycle for each, which is 48 us on 16 MHz, under a thousandth of the 65 ms the
    /// reset already waits.
    pub fn to_app(self, boot_word: u16) -> Option<u64> {
        match self {
            Bootloader::None => None,
            Bootloader::Optiboot => {
                let erased = u64::from(OPTIBOOT_WORD.saturating_sub(boot_word));
                Some(erased + OPTIBOOT_TO_APP)
            }
        }
    }
}

/// Where Optiboot's wait for an upload is, from `optiboot_atmega328.lst`, `main` from `7e00` with
/// `EXTRF` set. Each state is one bus access, made at the start of its instruction as
/// `Core::step` makes it, and [`Wait::next`] gives the cycles to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wait {
    /// The `n`th access before the first flash: `MCUSR`, Timer1, USART0, the watchdog and
    /// `DDRB`.
    Setup(u8),
    /// A pass of the inlined `flash_led(LED_START_FLASHES * 2)`, `left` passes to go with this
    /// one.
    Flash { left: u8, at: Flash },
    /// `getch` polling `RXC0`.
    Getch,
}

/// The accesses of one `flash_led` pass, from `7e36`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flash {
    /// `sts TCNT1H`, `0xFC`, through the temporary byte.
    CountHigh,
    /// `sts TCNT1L`, `0x30`, so Timer1 overflows 976 counts on.
    CountLow,
    /// `out TIFR1`, clearing `TOV1`.
    ClearOverflow,
    /// `sbis TIFR1, TOV1` and `rjmp` back.
    PollOverflow,
    /// `sbi PINB, 5`, which toggles `PB5`.
    Toggle,
    /// `wdr`.
    Wdr,
}

/// One bus access of [`Wait`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    /// `OUT`, `STS` or `ST` of `value`, an instruction of `cycles`.
    Store { addr: u16, value: u8, cycles: u64 },
    /// `SBI`.
    SetBit { addr: u16, bit: u8 },
    /// `WDR`.
    Wdr,
    /// `SBIS`, which reads with a peek.
    SkipIfSet { addr: u16, bit: u8 },
    /// `LDS`, then `SBRS` on `bit`.
    Load { addr: u16, bit: u8 },
}

/// `LED_START_FLASHES` is 3 for `atmega328` in `bootloaders/optiboot/Makefile`, and `main` passes
/// twice that, one toggle a pass.
const FLASH_PASSES: u8 = 6;

/// `UCSR0A`, data address `0xC0`.
const UCSR0A: u16 = 0x00C0;
/// `RXC0`, bit 7 of `UCSR0A`.
const RXC0: u8 = 7;
/// `TIFR1`, I/O `0x16`.
const TIFR1: u16 = 0x0036;

/// The cycle, from Optiboot's first instruction, of the first access: `eor r1, r1`,
/// `in r24, MCUSR`, then `out MCUSR, r1`.
pub(crate) const WAIT_FIRST: u64 = 2;

/// The word of `getch`, `7fa6`, where a byte on RXD starts an upload.
pub(crate) const GETCH_WORD: u16 = 0x3FD3;

/// The accesses before the first flash, each with the cycles to the next:
///
/// | Cycle | Instruction | Store |
/// | --- | --- | --- |
/// | 2 | `out MCUSR, r1` | `MCUSR` 0 |
/// | 6 | `sts TCCR1B` | 5, `clkI/O` / 1024 |
/// | 9 | `sts UCSR0A` | 2, `U2X0` |
/// | 12 | `sts UCSR0B` | `0x18`, `RXEN0` and `TXEN0` |
/// | 15 | `sts UCSR0C` | 6, 8 data bits |
/// | 18 | `sts UBRR0L` | 16, 115200 baud nominal |
/// | 27 | `st Z`, in `watchdogConfig` | `WDTCSR` `WDCE` and `WDE` |
/// | 29 | `st Z` | `WDTCSR` `0x0E`, system reset after 1 s |
/// | 35 | `sbi DDRB, 5` | |
///
/// Between them run `ldi`s, `sbrs r24, 1` skipping `rcall appStart` (2), `rcall watchdogConfig`
/// (3) with its three `ldi`s, its `ret` (4), and the four `ldi`s of the flash loop, whose first
/// `sts` is at 41.
const SETUP: [(Access, u64); 9] = [
    (store(0x0054, 0, 1), 4),
    (store(0x0081, 0x05, 2), 3),
    (store(UCSR0A, 0x02, 2), 3),
    (store(0x00C1, 0x18, 2), 3),
    (store(0x00C2, 0x06, 2), 3),
    (store(0x00C4, 0x10, 2), 9),
    (store(0x0060, 0x18, 2), 2),
    (store(0x0060, 0x0E, 2), 6),
    (
        Access::SetBit {
            addr: 0x0024,
            bit: 5,
        },
        6,
    ),
];

impl Access {
    /// The cycles of the access's own instruction, which the bus ticks through at once. `set`
    /// is whether the bit it read was set: `SBIS` skips the one-word `rjmp` in 2.
    pub(crate) fn cycles(self, set: bool) -> u64 {
        match self {
            Access::Store { cycles, .. } => cycles,
            Access::SetBit { .. } | Access::Load { .. } => 2,
            Access::Wdr => 1,
            Access::SkipIfSet { .. } => 1 + u64::from(set),
        }
    }
}

const fn store(addr: u16, value: u8, cycles: u64) -> Access {
    Access::Store {
        addr,
        value,
        cycles,
    }
}

impl Wait {
    pub(crate) const START: Self = Wait::Setup(0);

    pub(crate) fn access(self) -> Access {
        match self {
            Wait::Setup(n) => SETUP[usize::from(n)].0,
            Wait::Flash { at, .. } => match at {
                Flash::CountHigh => store(0x0085, 0xFC, 2),
                Flash::CountLow => store(0x0084, 0x30, 2),
                Flash::ClearOverflow => store(TIFR1, 0x01, 1),
                Flash::PollOverflow => Access::SkipIfSet {
                    addr: TIFR1,
                    bit: 0,
                },
                Flash::Toggle => Access::SetBit {
                    addr: 0x0023,
                    bit: 5,
                },
                Flash::Wdr => Access::Wdr,
            },
            Wait::Getch => Access::Load {
                addr: UCSR0A,
                bit: RXC0,
            },
        }
    }

    /// The next state and the cycles from this access's start to its start, given whether the
    /// bit this access read was set. `None` once `getch` has a byte.
    pub(crate) fn next(self, set: bool) -> Option<(Self, u64)> {
        let flash = |left, at| Wait::Flash { left, at };
        Some(match self {
            Wait::Setup(n) => {
                let cycles = SETUP[usize::from(n)].1;
                if usize::from(n) + 1 < SETUP.len() {
                    (Wait::Setup(n + 1), cycles)
                } else {
                    (flash(FLASH_PASSES, Flash::CountHigh), cycles)
                }
            }
            Wait::Flash { left, at } => match at {
                Flash::CountHigh => (flash(left, Flash::CountLow), 2),
                Flash::CountLow => (flash(left, Flash::ClearOverflow), 2),
                Flash::ClearOverflow => (flash(left, Flash::PollOverflow), 1),
                // `sbis` is 1 cycle and `rjmp` 2, or `sbis` skips the one-word `rjmp` in 2.
                Flash::PollOverflow if set => (flash(left, Flash::Toggle), 2),
                Flash::PollOverflow => (self, 3),
                Flash::Toggle => (flash(left, Flash::Wdr), 2),
                // `wdr`, `subi` and a taken `brne` back to `7e36`.
                Flash::Wdr if left > 1 => (flash(left - 1, Flash::CountHigh), 4),
                // `wdr`, `subi`, `brne` not taken, ten one-cycle instructions from `7e4c` and
                // `rcall getch`.
                Flash::Wdr => (Wait::Getch, 16),
            },
            // `lds` 2, `sbrs` 1 and `rjmp` 2.
            Wait::Getch if set => return None,
            Wait::Getch => (Wait::Getch, 5),
        })
    }
}
