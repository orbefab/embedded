//! The ATmega328P table: sizes, interrupt names, and the devices at each address.
//!
//! A device is a [`DeviceSpec`] row. Its registers carry the `SBI`/`CBI` strobe
//! mask, so another peripheral is another row
//! instead of a bus edit. `SPL`/`SPH` reset from RAMEND in the core. `UCSRnC`
//! resets inside the USART.
//!
//! Vector word addresses are `2 × (number − 1)`. `MCUCR.IVSEL` does not move them.

use crate::Port;
use crate::periph::acmp::{REG_ACSR, REG_DIDR1};
use crate::periph::adc::{REG_ADCH, REG_ADCL, REG_ADCSRA, REG_ADCSRB, REG_ADMUX, REG_DIDR0};
use crate::periph::eeprom::{REG_EEARH, REG_EEARL, REG_EECR, REG_EEDR};
use crate::periph::system::{
    PRADC, PRSPI, PRTIM0, PRTIM1, PRTIM2, PRTWI, PRUSART0, REG_CLKPR, REG_GPIOR0, REG_GPIOR1,
    REG_GPIOR2, REG_MCUSR, REG_OSCCAL, REG_PRR, REG_SMCR, REG_WDTCSR,
};
use crate::periph::timer::Timer;
use crate::periph::{
    AcmpCfg, AdcCfg, EepromCfg, ExtIntCfg, IntLine, PcGroup, REG_EICRA, REG_EICRB, REG_EIFR,
    REG_EIMSK, REG_MCUCR, REG_PCICR, REG_PCIFR, REG_PCMSK0, REG_PCMSK1, REG_PCMSK2, SystemCfg,
};
use crate::regs::{
    ASSR, GPIO_DDR, GPIO_PIN, GPIO_PORT, GTCCR, ICR_H, ICR_L, OCR_A, OCR_AH, OCR_B, OCR_BH,
    OwnedReg, RegBlock, SPCR, SPDR, SPSR, TCCR_A, TCCR_B, TCCR_C, TCNT_H, TCNT_L, TIFR, TIMSK,
    TWAMR, TWAR, TWBR, TWCR, TWDR, TWSR, UBRRH, UBRRL, UCSRA, UCSRA_TXC_STROBE, UCSRB, UCSRC, UDR,
    owned, push_reg, reg_block,
};

/// Where a pin function is wired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OcPin {
    pub port: Port,
    pub bit: u8,
}

/// Which prescaler a timer uses. Sync is Timer0/1. Async is Timer2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimerClock {
    Sync,
    Async,
}

/// Clock a device row is on. Sleep stops the domain Table 10-1 names.
///
/// `Asy` is Timer2's asynchronous clock. No TOSC crystal is modelled and
/// `AS2` does not switch the counter onto it, so a power-save sleep does not
/// keep that row counting. The column is still `Asy`, and the wake mask is
/// what lets a pending Timer2 request leave power-save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClockDomain {
    /// `clkI/O`. Runs in Idle only.
    Io,
    /// `clkADC`. Runs in Idle and ADC noise reduction.
    Adc,
    /// `clkASY`. Not modeled as a running oscillator. See the enum note.
    Asy,
    /// `clkCPU`. Sleep halts the core; the row has no separate gate.
    Cpu,
}

/// Table 10-1 column for one vector. `Int` is level or edge from `ISC` at the
/// check, because the same `INT0`/`INT1` line is either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WakeClass {
    None,
    Int,
    PinChange,
    Twi,
    Timer2,
    Eeprom,
    Adc,
    Wdt,
    OtherIo,
}

/// Bits of a sleep mode's wake mask. One bit per [`WakeClass`] column.
pub(crate) const WAKE_INT_LEVEL: u16 = 1 << 0;
pub(crate) const WAKE_INT_EDGE: u16 = 1 << 1;
pub(crate) const WAKE_PIN_CHANGE: u16 = 1 << 2;
pub(crate) const WAKE_TWI: u16 = 1 << 3;
pub(crate) const WAKE_TIMER2: u16 = 1 << 4;
pub(crate) const WAKE_EEPROM: u16 = 1 << 5;
pub(crate) const WAKE_ADC: u16 = 1 << 6;
pub(crate) const WAKE_WDT: u16 = 1 << 7;
pub(crate) const WAKE_OTHER_IO: u16 = 1 << 8;

const WAKE_ALL: u16 = WAKE_INT_LEVEL
    | WAKE_INT_EDGE
    | WAKE_PIN_CHANGE
    | WAKE_TWI
    | WAKE_TIMER2
    | WAKE_EEPROM
    | WAKE_ADC
    | WAKE_WDT
    | WAKE_OTHER_IO;
/// Table 10-1 ADC noise reduction. Timer2's mark is note 2 (async only).
/// Async is unmodeled, so the mask does not include it. Other I/O and edges
/// are blank in the table.
const WAKE_ADC_NR: u16 =
    WAKE_INT_LEVEL | WAKE_PIN_CHANGE | WAKE_TWI | WAKE_EEPROM | WAKE_ADC | WAKE_WDT;
/// Power-down and standby. Level `INT`, pin change, TWI match, and the watchdog.
const WAKE_DOWN: u16 = WAKE_INT_LEVEL | WAKE_PIN_CHANGE | WAKE_TWI | WAKE_WDT;
/// Power-save and extended standby. Power-down, plus a pending Timer2 request.
/// The counter itself does not run: `clkASY` is unmodeled.
const WAKE_SAVE: u16 = WAKE_DOWN | WAKE_TIMER2;

/// Indexed by [`crate::periph::system::SleepMode::mask_index`].
pub(crate) const WAKE_MASK: [u16; 7] = [
    WAKE_ALL,
    WAKE_ADC_NR,
    WAKE_DOWN,
    WAKE_SAVE,
    WAKE_DOWN,
    WAKE_SAVE,
    0,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimerWidth {
    Eight,
    Sixteen,
}

/// One timer row. A later 16-bit timer is another [`TimerWidth::Sixteen`] row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TimerCfg {
    pub width: TimerWidth,
    pub clock: TimerClock,
    /// `TCNTn` low byte, so a test can tell two rows apart.
    pub tcnt: u16,
    pub timsk: u16,
    pub tifr: u16,
    pub oc: [OcPin; 2],
    /// Compare vector numbers. Entry 0 is compare A.
    pub comp_vec: [u8; 2],
    pub ovf_vec: u8,
    pub capt_vec: u8,
    /// `T0` or `T1` pin id. `None` when this timer has no external clock pin.
    pub t_pin: Option<u8>,
    /// `ICP1` pin id. `None` when this timer has no input capture pin.
    pub icp_pin: Option<u8>,
    /// Table 24-6 sources. Each byte is `(ADTS << 4) | TIFR bit number`.
    /// `None` is an empty slot. The ADC reads this row, not the timer's
    /// position in the device list.
    pub adc_trigger: [Option<u8>; 3],
    /// §23.3.2. `ACIC` drives this timer's capture input. Only Timer1.
    pub acic: bool,
    /// `PRR` bit that freezes this row. `None` means the row has no bit.
    pub prr_bit: Option<u8>,
    /// Which sleep clock stops the row. Not the timer's place in the list.
    pub domain: ClockDomain,
    pub regs: RegBlock,
}

/// Timer0: `OCF0A` is `ADTS = 3` (`TIFR` bit 1), `TOV0` is `ADTS = 4` (bit 0,
/// so the packed byte is just `ADTS << 4`).
const ADC_TRIG_TIMER0: [Option<u8>; 3] = [Some((3 << 4) | 1), Some(4 << 4), None];
/// Timer1: `OCF1B` is `ADTS = 5` (bit 2), `TOV1` is `ADTS = 6` (bit 0),
/// `ICF1` is `ADTS = 7` (bit 5).
const ADC_TRIG_TIMER1: [Option<u8>; 3] = [Some((5 << 4) | 2), Some(6 << 4), Some((7 << 4) | 5)];
const ADC_TRIG_NONE: [Option<u8>; 3] = [None, None, None];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PortCfg {
    pub port: Port,
    pub regs: [OwnedReg; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PrescalerCfg {
    pub reg: OwnedReg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UsartCfg {
    pub regs: [OwnedReg; 6],
    pub prr_bit: u8,
    pub tx: OcPin,
    pub rx: OcPin,
    pub xck: OcPin,
    /// Table 12-6. Vector numbers, 1-based.
    pub rx_vec: u8,
    pub udre_vec: u8,
    pub tx_vec: u8,
}

/// SPI pins and vector. §19 and Table 14-3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SpiCfg {
    pub regs: [OwnedReg; 3],
    pub ss: OcPin,
    pub mosi: OcPin,
    pub miso: OcPin,
    pub sck: OcPin,
    /// Table 12-6 vector number. 18 is `SPI STC`, word address `0x22`.
    pub vec: u8,
    pub prr_bit: u8,
}

/// TWI pins and vector. §22 and Table 14-7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TwiCfg {
    pub regs: [OwnedReg; 6],
    pub sda: OcPin,
    pub scl: OcPin,
    /// Table 12-6 vector number. 25 is `TWI`, word address `0x30`.
    pub vec: u8,
    pub prr_bit: u8,
}

/// Built into [`crate::periph::Device`] from this row alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeviceSpec {
    Port(PortCfg),
    ExtInt(ExtIntCfg),
    Timer(TimerCfg),
    Prescaler(PrescalerCfg),
    Usart(UsartCfg),
    Spi(SpiCfg),
    Twi(TwiCfg),
    Adc(crate::periph::AdcCfg),
    Acmp(crate::periph::AcmpCfg),
    Eeprom(EepromCfg),
    System(SystemCfg),
}

/// `PRR` bit and sleep clock copied off the device row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ModuleGate {
    pub prr_bit: Option<u8>,
    pub domain: ClockDomain,
}

const fn present(bit: u8) -> Option<u8> {
    if bit == 0 { None } else { Some(bit) }
}

impl DeviceSpec {
    /// The row's power-reduction bit and clock. A zero bit is not in `PRR`.
    pub(crate) fn gate(self) -> ModuleGate {
        match self {
            DeviceSpec::Timer(cfg) => ModuleGate {
                prr_bit: cfg.prr_bit,
                domain: cfg.domain,
            },
            DeviceSpec::Usart(cfg) => ModuleGate {
                prr_bit: present(cfg.prr_bit),
                domain: ClockDomain::Io,
            },
            DeviceSpec::Spi(cfg) => ModuleGate {
                prr_bit: present(cfg.prr_bit),
                domain: ClockDomain::Io,
            },
            DeviceSpec::Twi(cfg) => ModuleGate {
                prr_bit: present(cfg.prr_bit),
                domain: ClockDomain::Io,
            },
            DeviceSpec::Adc(cfg) => ModuleGate {
                prr_bit: present(cfg.prr_bit),
                domain: ClockDomain::Adc,
            },
            DeviceSpec::ExtInt(_) | DeviceSpec::Prescaler(_) | DeviceSpec::Port(_) => ModuleGate {
                prr_bit: None,
                domain: ClockDomain::Io,
            },
            DeviceSpec::Acmp(_) => ModuleGate {
                prr_bit: None,
                domain: ClockDomain::Io,
            },
            DeviceSpec::Eeprom(_) | DeviceSpec::System(_) => ModuleGate {
                prr_bit: None,
                domain: ClockDomain::Cpu,
            },
        }
    }
}

const fn port_cfg(port: Port, pin: u16) -> PortCfg {
    PortCfg {
        port,
        regs: [
            owned(pin, GPIO_PIN, 0xFF),
            owned(pin + 1, GPIO_DDR, 0),
            owned(pin + 2, GPIO_PORT, 0),
        ],
    }
}

/// `TSM`, `PSRASY`, and `PSRSYNC` are modeled. Reserved bits read as zero (§17.4.1).
const fn prescaler(addr: u16) -> PrescalerCfg {
    PrescalerCfg {
        reg: owned(addr, GTCCR, 0),
    }
}

const fn usart_cfg(
    base: u16,
    tx: OcPin,
    rx: OcPin,
    xck: OcPin,
    vectors: (u8, u8, u8),
    prr_bit: u8,
) -> UsartCfg {
    let (rx_vec, udre_vec, tx_vec) = vectors;
    UsartCfg {
        prr_bit,
        tx,
        rx,
        xck,
        rx_vec,
        udre_vec,
        tx_vec,
        regs: [
            owned(base, UCSRA, UCSRA_TXC_STROBE),
            owned(base + 1, UCSRB, 0),
            owned(base + 2, UCSRC, 0),
            owned(base + 4, UBRRL, 0),
            owned(base + 5, UBRRH, 0),
            owned(base + 6, UDR, 0),
        ],
    }
}

/// §19. PB2 `SS`, PB3 `MOSI`, PB4 `MISO`, PB5 `SCK`. Vector 18 (`SPI_STC`). `PRSPI`.
const fn spi_328() -> SpiCfg {
    SpiCfg {
        regs: [
            owned(0x004C, SPCR, 0),
            owned(0x004D, SPSR, 0),
            owned(0x004E, SPDR, 0),
        ],
        ss: OcPin {
            port: Port::B,
            bit: 2,
        },
        mosi: OcPin {
            port: Port::B,
            bit: 3,
        },
        miso: OcPin {
            port: Port::B,
            bit: 4,
        },
        sck: OcPin {
            port: Port::B,
            bit: 5,
        },
        vec: 18,
        prr_bit: PRSPI,
    }
}

/// §22.9. PC4 `SDA`, PC5 `SCL`. Vector 25 (`TWI`). `PRTWI`.
const fn twi_328() -> TwiCfg {
    TwiCfg {
        regs: [
            owned(0x00B8, TWBR, 0),
            owned(0x00B9, TWSR, 0),
            owned(0x00BA, TWAR, 0),
            owned(0x00BB, TWDR, 0),
            owned(0x00BC, TWCR, 0),
            owned(0x00BD, TWAMR, 0),
        ],
        sda: OcPin {
            port: Port::C,
            bit: 4,
        },
        scl: OcPin {
            port: Port::C,
            bit: 5,
        },
        vec: 25,
        prr_bit: PRTWI,
    }
}

/// 328P ADC pins. ADC0–ADC5 are PC0–PC5 (§14.3.2). ADC6 and ADC7 have no
/// digital pad (TQFP / VQFN, §1.1.9).
const fn adc_328() -> AdcCfg {
    let mut regs = reg_block();
    // Writes to the data registers are ignored.
    push_reg(&mut regs, owned(0x0078, REG_ADCL, 0));
    push_reg(&mut regs, owned(0x0079, REG_ADCH, 0));
    // `ADIF` is write-one-to-clear (§24.9.2).
    push_reg(&mut regs, owned(0x007A, REG_ADCSRA, 0x10));
    push_reg(&mut regs, owned(0x007B, REG_ADCSRB, 0));
    push_reg(&mut regs, owned(0x007C, REG_ADMUX, 0));
    push_reg(&mut regs, owned(0x007E, REG_DIDR0, 0));
    AdcCfg {
        vec: 22,
        pin: [8, 9, 10, 11, 12, 13, 0xFF, 0xFF],
        prr_bit: PRADC,
        regs,
    }
}

/// 1 KiB. `EEAR9:8` are the only high bits (§8.6.1). Vector 23 is Table 12-6.
const fn eeprom_328() -> EepromCfg {
    let mut regs = reg_block();
    push_reg(&mut regs, owned(0x0042, REG_EEARH, 0));
    push_reg(&mut regs, owned(0x0041, REG_EEARL, 0));
    push_reg(&mut regs, owned(0x0040, REG_EEDR, 0));
    push_reg(&mut regs, owned(0x003F, REG_EECR, 0));
    EepromCfg {
        bytes: 1024,
        vec: 23,
        regs,
    }
}

/// `SMCR`, `MCUSR`, `WDTCSR`, `CLKPR`, `PRR`, `OSCCAL`, `GPIOR0`–`GPIOR2`. `WDT` is vector 7.
const fn system_328() -> SystemCfg {
    let mut regs = reg_block();
    push_reg(&mut regs, owned(0x0053, REG_SMCR, 0));
    push_reg(&mut regs, owned(0x0054, REG_MCUSR, 0));
    // `WDIF` is write-one-to-clear (§11.9.2). The address is above `OUT`.
    push_reg(&mut regs, owned(0x0060, REG_WDTCSR, 0x80));
    push_reg(&mut regs, owned(0x0061, REG_CLKPR, 0));
    push_reg(&mut regs, owned(0x0064, REG_PRR, 0));
    push_reg(&mut regs, owned(0x0066, REG_OSCCAL, 0));
    // §7.5.1–§7.5.3. General-purpose I/O registers.
    push_reg(&mut regs, owned(0x003E, REG_GPIOR0, 0));
    push_reg(&mut regs, owned(0x004A, REG_GPIOR1, 0));
    push_reg(&mut regs, owned(0x004B, REG_GPIOR2, 0));
    SystemCfg { wdt_vec: 7, regs }
}

/// AIN0 is PD6, AIN1 is PD7 (§14.3.3, Table 14-9). Vector 24 is Table 12-6.
const fn acmp_328() -> AcmpCfg {
    let mut regs = reg_block();
    // `ACI` is write-one-to-clear (§23.3.2).
    push_reg(&mut regs, owned(0x0050, REG_ACSR, 0x10));
    push_reg(&mut regs, owned(0x007F, REG_DIDR1, 0));
    AcmpCfg {
        vec: 24,
        ain0: 22,
        ain1: 23,
        regs,
    }
}

const fn oc(port: Port, bit: u8) -> OcPin {
    OcPin { port, bit }
}

struct EightPins {
    oc_a: OcPin,
    oc_b: OcPin,
    comp_a: u8,
    comp_b: u8,
    ovf: u8,
}

#[allow(clippy::too_many_arguments)]
const fn timer8(
    clock: TimerClock,
    tccra: u16,
    timsk: u16,
    tifr: u16,
    assr: u16,
    t_pin: Option<u8>,
    pins: EightPins,
    adc_trigger: [Option<u8>; 3],
    prr_bit: Option<u8>,
    domain: ClockDomain,
) -> TimerCfg {
    let flags = Timer::flag_mask_for(false);
    let mut regs = reg_block();
    push_reg(&mut regs, owned(tccra, TCCR_A, 0));
    push_reg(&mut regs, owned(tccra + 1, TCCR_B, 0));
    push_reg(&mut regs, owned(tccra + 2, TCNT_L, 0));
    push_reg(&mut regs, owned(tccra + 3, OCR_A, 0));
    push_reg(&mut regs, owned(tccra + 4, OCR_B, 0));
    push_reg(&mut regs, owned(timsk, TIMSK, 0));
    push_reg(&mut regs, owned(tifr, TIFR, flags));
    if assr != 0 {
        push_reg(&mut regs, owned(assr, ASSR, 0));
    }
    TimerCfg {
        width: TimerWidth::Eight,
        clock,
        tcnt: tccra + 2,
        timsk,
        tifr,
        oc: [pins.oc_a, pins.oc_b],
        comp_vec: [pins.comp_a, pins.comp_b],
        ovf_vec: pins.ovf,
        capt_vec: 0,
        t_pin,
        icp_pin: None,
        adc_trigger,
        acic: false,
        prr_bit,
        domain,
        regs,
    }
}

struct SixteenRegs {
    tccra: u16,
    tcnt: u16,
    icr: u16,
    ocr_a: u16,
    timsk: u16,
    tifr: u16,
}

struct SixteenPins {
    oc: [OcPin; 2],
    comp: [u8; 2],
    ovf: u8,
    capt: u8,
    t_pin: Option<u8>,
    icp_pin: Option<u8>,
}

/// One 16-bit timer, from the addresses and vectors passed in.
const fn timer16(
    regs: SixteenRegs,
    pins: SixteenPins,
    adc_trigger: [Option<u8>; 3],
    acic: bool,
    prr_bit: Option<u8>,
) -> TimerCfg {
    let capture = pins.capt != 0;
    let flags = Timer::flag_mask_for(capture);
    let mut rows = reg_block();
    push_reg(&mut rows, owned(regs.tccra, TCCR_A, 0));
    push_reg(&mut rows, owned(regs.tccra + 1, TCCR_B, 0));
    push_reg(&mut rows, owned(regs.tccra + 2, TCCR_C, 0));
    push_reg(&mut rows, owned(regs.tcnt, TCNT_L, 0));
    push_reg(&mut rows, owned(regs.tcnt + 1, TCNT_H, 0));
    push_reg(&mut rows, owned(regs.icr, ICR_L, 0));
    push_reg(&mut rows, owned(regs.icr + 1, ICR_H, 0));
    push_reg(&mut rows, owned(regs.ocr_a, OCR_A, 0));
    push_reg(&mut rows, owned(regs.ocr_a + 1, OCR_AH, 0));
    push_reg(&mut rows, owned(regs.ocr_a + 2, OCR_B, 0));
    push_reg(&mut rows, owned(regs.ocr_a + 3, OCR_BH, 0));
    push_reg(&mut rows, owned(regs.timsk, TIMSK, 0));
    push_reg(&mut rows, owned(regs.tifr, TIFR, flags));
    TimerCfg {
        width: TimerWidth::Sixteen,
        clock: TimerClock::Sync,
        tcnt: regs.tcnt,
        timsk: regs.timsk,
        tifr: regs.tifr,
        oc: pins.oc,
        comp_vec: pins.comp,
        ovf_vec: pins.ovf,
        capt_vec: pins.capt,
        t_pin: pins.t_pin,
        icp_pin: pins.icp_pin,
        adc_trigger,
        acic,
        prr_bit,
        domain: ClockDomain::Io,
        regs: rows,
    }
}

// OC pins, 328P: OC0A PD6, OC0B PD5, OC1A PB1, OC1B PB2, OC2A PB3, OC2B PD3.
const OC0A_328: OcPin = oc(Port::D, 6);
const OC0B_328: OcPin = oc(Port::D, 5);
const OC1A_328: OcPin = oc(Port::B, 1);
const OC1B_328: OcPin = oc(Port::B, 2);
const OC2A_328: OcPin = oc(Port::B, 3);
const OC2B_328: OcPin = oc(Port::D, 3);

/// DS40002061B §15, §16, §18. Vector numbers are Table 12-6.
const TIMER0_328: DeviceSpec = DeviceSpec::Timer(timer8(
    TimerClock::Sync,
    0x0044,
    0x006E,
    0x0035,
    0,
    Some(20),
    EightPins {
        oc_a: OC0A_328,
        oc_b: OC0B_328,
        comp_a: 15,
        comp_b: 16,
        ovf: 17,
    },
    ADC_TRIG_TIMER0,
    Some(PRTIM0),
    ClockDomain::Io,
));

const TIMER1_328: DeviceSpec = DeviceSpec::Timer(timer16(
    SixteenRegs {
        tccra: 0x0080,
        tcnt: 0x0084,
        icr: 0x0086,
        ocr_a: 0x0088,
        timsk: 0x006F,
        tifr: 0x0036,
    },
    SixteenPins {
        oc: [OC1A_328, OC1B_328],
        comp: [12, 13],
        ovf: 14,
        capt: 11,
        t_pin: Some(21),
        icp_pin: Some(0),
    },
    ADC_TRIG_TIMER1,
    true,
    Some(PRTIM1),
));

const TIMER2_328: DeviceSpec = DeviceSpec::Timer(timer8(
    TimerClock::Async,
    0x00B0,
    0x0070,
    0x0037,
    0x00B6,
    None,
    EightPins {
        oc_a: OC2A_328,
        oc_b: OC2B_328,
        comp_a: 8,
        comp_b: 9,
        ovf: 10,
    },
    ADC_TRIG_NONE,
    Some(PRTIM2),
    ClockDomain::Asy,
));

/// `PUD` is bit 4. `IVSEL` and `IVCE` are stored and do not move the vectors.
#[allow(clippy::too_many_arguments)]
const fn extint_cfg(
    lines: &'static [IntLine],
    groups: &'static [PcGroup],
    mcucr_mask: u8,
    eicra_mask: u8,
    eicrb: u16,
    eicrb_mask: u8,
    eimsk_mask: u8,
    eifr_mask: u8,
    pcicr_mask: u8,
    pcifr_mask: u8,
    pcmsk1: u16,
    pcmsk2: u16,
) -> ExtIntCfg {
    let mut regs = reg_block();
    push_reg(&mut regs, owned(0x0055, REG_MCUCR, 0));
    push_reg(&mut regs, owned(0x0069, REG_EICRA, 0));
    if eicrb != 0 {
        push_reg(&mut regs, owned(eicrb, REG_EICRB, 0));
    }
    push_reg(&mut regs, owned(0x003D, REG_EIMSK, 0));
    push_reg(&mut regs, owned(0x003C, REG_EIFR, eifr_mask));
    push_reg(&mut regs, owned(0x0068, REG_PCICR, 0));
    push_reg(&mut regs, owned(0x003B, REG_PCIFR, pcifr_mask));
    push_reg(&mut regs, owned(0x006B, REG_PCMSK0, 0));
    if pcmsk1 != 0 {
        push_reg(&mut regs, owned(pcmsk1, REG_PCMSK1, 0));
    }
    if pcmsk2 != 0 {
        push_reg(&mut regs, owned(pcmsk2, REG_PCMSK2, 0));
    }
    ExtIntCfg {
        lines,
        groups,
        mcucr_mask,
        eifr_mask,
        pcifr_mask,
        eimsk_mask,
        pcicr_mask,
        eicra_mask,
        eicrb_mask,
        regs,
    }
}

/// DS40002061B §13. `INT0` is `PD2`, `INT1` is `PD3`.
const INTS_328: &[IntLine] = &[
    IntLine {
        pin: 18,
        bit: 0,
        isc_shift: 0,
        in_b: false,
        vector: 2,
    },
    IntLine {
        pin: 19,
        bit: 1,
        isc_shift: 2,
        in_b: false,
        vector: 3,
    },
];

/// `PCINT0`–`7` port B, `PCINT8`–`14` port C, `PCINT16`–`23` port D.
const GROUPS_328: &[PcGroup] = &[
    PcGroup {
        port: Port::B,
        index: 0,
        bit: 0,
        vector: 4,
        valid: 0xFF,
    },
    PcGroup {
        port: Port::C,
        index: 1,
        bit: 1,
        vector: 5,
        valid: 0x7F,
    },
    PcGroup {
        port: Port::D,
        index: 2,
        bit: 2,
        vector: 6,
        valid: 0xFF,
    },
];

/// 328P `MCUCR` read mask is `BODS|BODSE|PUD|IVSEL|IVCE`. Reserved bits read as zero.
const EXTINT_328: DeviceSpec = DeviceSpec::ExtInt(extint_cfg(
    INTS_328, GROUPS_328, 0x73, 0x0F, 0, 0, 0x03, 0x03, 0x07, 0x07, 0x006C, 0x006D,
));

pub(crate) const DEVICES: &[DeviceSpec] = &[
    DeviceSpec::Port(port_cfg(Port::B, 0x0023)),
    DeviceSpec::Port(port_cfg(Port::C, 0x0026)),
    DeviceSpec::Port(port_cfg(Port::D, 0x0029)),
    EXTINT_328,
    DeviceSpec::Prescaler(prescaler(0x0043)),
    TIMER0_328,
    TIMER1_328,
    TIMER2_328,
    // PD1 TXD, PD0 RXD. Vectors 19–21 are Table 12-6.
    DeviceSpec::Usart(usart_cfg(
        0x00C0,
        OcPin {
            port: Port::D,
            bit: 1,
        },
        OcPin {
            port: Port::D,
            bit: 0,
        },
        OcPin {
            port: Port::D,
            bit: 4,
        },
        (19, 20, 21),
        PRUSART0,
    )),
    DeviceSpec::Spi(spi_328()),
    DeviceSpec::Twi(twi_328()),
    // §24 and §23.
    DeviceSpec::Adc(adc_328()),
    DeviceSpec::Acmp(acmp_328()),
    DeviceSpec::Eeprom(eeprom_328()),
    DeviceSpec::System(system_328()),
];

/// DS40002061B Table 12-6. 26 vectors. The word is `2 × (number − 1)`.
pub(crate) const VECTORS: &[&str] = &[
    "RESET",
    "INT0",
    "INT1",
    "PCINT0",
    "PCINT1",
    "PCINT2",
    "WDT",
    "TIMER2_COMPA",
    "TIMER2_COMPB",
    "TIMER2_OVF",
    "TIMER1_CAPT",
    "TIMER1_COMPA",
    "TIMER1_COMPB",
    "TIMER1_OVF",
    "TIMER0_COMPA",
    "TIMER0_COMPB",
    "TIMER0_OVF",
    "SPI_STC",
    "USART_RX",
    "USART_UDRE",
    "USART_TX",
    "ADC",
    "EE_READY",
    "ANALOG_COMP",
    "TWI",
    "SPM_Ready",
];

/// Parallel to [`VECTORS`]. `Int` is split into level and edge at the wake.
pub(crate) const WAKE_CLASS: &[WakeClass] = &[
    WakeClass::None,
    WakeClass::Int,
    WakeClass::Int,
    WakeClass::PinChange,
    WakeClass::PinChange,
    WakeClass::PinChange,
    WakeClass::Wdt,
    WakeClass::Timer2,
    WakeClass::Timer2,
    WakeClass::Timer2,
    WakeClass::OtherIo,
    WakeClass::OtherIo,
    WakeClass::OtherIo,
    WakeClass::OtherIo,
    WakeClass::OtherIo,
    WakeClass::OtherIo,
    WakeClass::OtherIo,
    WakeClass::OtherIo,
    WakeClass::OtherIo,
    WakeClass::OtherIo,
    WakeClass::OtherIo,
    WakeClass::Adc,
    WakeClass::Eeprom,
    WakeClass::OtherIo,
    WakeClass::Twi,
    WakeClass::Eeprom,
];

pub(crate) const FLASH_WORDS: u16 = 16 * 1024;
pub(crate) const RAMEND: u16 = 0x08FF;
/// Cycles from accepting an interrupt until the vector instruction runs (§7.7.1).
pub(crate) const IRQ_CYCLES: u64 = 4;
pub(crate) const SPL: u16 = 0x005D;
pub(crate) const SPH: u16 = 0x005E;
pub(crate) const SREG: u16 = 0x005F;
/// USART instance behind the host channel `Usart0`.
pub(crate) const USART0: u8 = 0;

/// Program address of vector `number` (1-based).
pub(crate) const fn vector_word(number: u8) -> u16 {
    (number.wrapping_sub(1) as u16) * 2
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Table 10-1 has one class per vector.
    #[test]
    fn wake_classes_cover_every_vector() {
        assert_eq!(WAKE_CLASS.len(), VECTORS.len());
    }

    fn spec<T>(pick: impl Fn(&DeviceSpec) -> Option<T>) -> T {
        DEVICES.iter().find_map(pick).expect("row")
    }

    /// DS40002061B §14.3: TXD PD1, RXD PD0, XCK PD4, SS PB2, MOSI PB3,
    /// MISO PB4, SCK PB5, SDA PC4, SCL PC5, INT0 PD2, INT1 PD3.
    #[test]
    fn alternate_pins_are_section_14_3() {
        let at = |pin: OcPin| (pin.port, pin.bit);
        let usart = spec(|s| match s {
            DeviceSpec::Usart(cfg) => Some(*cfg),
            _ => None,
        });
        assert_eq!(at(usart.tx), (Port::D, 1));
        assert_eq!(at(usart.rx), (Port::D, 0));
        assert_eq!(at(usart.xck), (Port::D, 4));
        let spi = spec(|s| match s {
            DeviceSpec::Spi(cfg) => Some(*cfg),
            _ => None,
        });
        assert_eq!(
            [at(spi.ss), at(spi.mosi), at(spi.miso), at(spi.sck)],
            [(Port::B, 2), (Port::B, 3), (Port::B, 4), (Port::B, 5)]
        );
        let twi = spec(|s| match s {
            DeviceSpec::Twi(cfg) => Some(*cfg),
            _ => None,
        });
        assert_eq!([at(twi.sda), at(twi.scl)], [(Port::C, 4), (Port::C, 5)]);
        let ext = spec(|s| match s {
            DeviceSpec::ExtInt(cfg) => Some(*cfg),
            _ => None,
        });
        let pins: Vec<u32> = ext.lines.iter().map(|line| u32::from(line.pin)).collect();
        assert_eq!(pins, [Port::D.pin_id(2), Port::D.pin_id(3)]);
    }
}
