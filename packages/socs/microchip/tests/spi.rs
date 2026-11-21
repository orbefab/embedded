//! SPI on the ATmega328P, from DS40002061B chapter 19.
//!
//! Cycle numbers are the instruction-set counts plus Table 19-5. The generator
//! starts when the `SPDR` store retires (§19.2). It is not free-running.
//! `LDI` is 1 cycle and `OUT` is 1, so a leading `LDI`/`OUT` pair retires at
//! cycle 2. `STS` is 2 cycles, so `LDI`/`STS` retires at cycle 3.

use crate::common;
use common::Instructions;
use common::{DDRB, PORTB, clock_spi, in_io, ldi, out, sbrs, spin, sts};

use orbe_microchip::{Channel, Cpu, Level, PinDrive, PinOrigin, Port, SpiClock, Timeline};

const SPCR: u16 = 0x004C;
const SPSR: u16 = 0x004D;
const SPDR: u16 = 0x004E;
const PRR: u16 = 0x0064;
const SMCR: u16 = 0x0053;

const SPIE: u8 = 1 << 7;
const SPE: u8 = 1 << 6;
const DORD: u8 = 1 << 5;
const MSTR: u8 = 1 << 4;
const CPHA: u8 = 1 << 2;
const SPIF: u8 = 1 << 7;
const WCOL: u8 = 1 << 6;
const SPI2X: u8 = 1 << 0;
const PRSPI: u8 = 1 << 2;

const SS: u32 = 2;
const MOSI: u32 = 3;
const MISO: u32 = 4;
const SCK: u32 = 5;

const SPDR_IO: u16 = 0x2E;
const SPSR_IO: u16 = 0x2D;

/// Table 19-5. `(SPI2X, SPR1:0, SCK period in CPU cycles)`.
const RATES: &[(bool, u8, u64)] = &[
    (false, 0, 4),
    (false, 1, 16),
    (false, 2, 64),
    (false, 3, 128),
    (true, 0, 2),
    (true, 1, 8),
    (true, 2, 32),
    (true, 3, 64),
];

fn with_vector(prefix: &[u16]) -> Vec<u16> {
    let mut words = prefix.to_vec();
    words.resize(0x22, 0x0000);
    words.push(spin());
    words
}

fn arm(cpu: &mut Cpu, mode_bits: u8, spi2x: bool) {
    // MOSI and SCK are outputs. SS stays an input so it is not a GPIO chip-select.
    cpu.poke(DDRB, (1 << 3) | (1 << 5));
    cpu.poke(SPCR, SPE | MSTR | mode_bits);
    if spi2x {
        cpu.poke(SPSR, SPI2X);
    }
    while cpu.take_pin().is_some() {}
}

fn xfer_out(byte: u8) -> Vec<u16> {
    vec![ldi(16, u16::from(byte)), out(16, SPDR_IO), spin()]
}

fn xfer_sts(byte: u8) -> Vec<u16> {
    let mut words = vec![ldi(16, u16::from(byte))];
    words.extend(sts(16, SPDR));
    words.push(spin());
    words
}

fn edges(cpu: &mut Cpu) -> Vec<(u32, bool, u64)> {
    let mut out = Vec::new();
    while let Some(report) = cpu.take_pin() {
        if report.origin != PinOrigin::Spi {
            continue;
        }
        out.push((report.pin, report.level == Level::High, report.cycle));
    }
    out
}

type PinEdge = (u32, bool, u64);

/// Independent of the device. Figure 19-3 / 19-4 and a square wave of period `N`.
fn frame_edges(
    start: u64,
    byte: u8,
    period: u64,
    cpol: bool,
    cpha: bool,
    lsb: bool,
) -> Vec<PinEdge> {
    let half = period / 2;
    let mut mosi = false;
    let mut sck = cpol;
    let mut out = Vec::new();
    let bit = |index: u8| {
        let shift = if lsb { index } else { 7 - index };
        (byte >> shift) & 1 == 1
    };
    let mut change = |pin: u32, level: bool, at: u64, cur: &mut bool| {
        if *cur != level {
            out.push((pin, level, at));
            *cur = level;
        }
    };
    if !cpha {
        change(MOSI, bit(0), start, &mut mosi);
    }
    for index in 0u8..8 {
        let lead = start + half * (2 * u64::from(index) + 1);
        let trail = start + half * (2 * u64::from(index) + 2);
        if cpha {
            change(MOSI, bit(index), lead, &mut mosi);
        }
        change(SCK, !cpol, lead, &mut sck);
        if !cpha && index + 1 < 8 {
            change(MOSI, bit(index + 1), trail, &mut mosi);
        }
        change(SCK, cpol, trail, &mut sck);
    }
    out
}

fn run_master(mode_bits: u8, spi2x: bool, byte: u8, stepped: bool) -> (Vec<PinEdge>, (u8, u64)) {
    let mut cpu = Cpu::new();
    arm(&mut cpu, mode_bits, spi2x);
    cpu.load_words(&xfer_out(byte));
    let period = period_of(mode_bits, spi2x);
    let spif = 2 + 8 * period;
    if stepped {
        while cpu.cycles() < spif + 2 {
            cpu.run_until(cpu.cycles() + 1);
        }
    } else {
        cpu.run_until(spif + 2);
    }
    let taken = cpu.take(Channel::Spi).unwrap();
    (edges(&mut cpu), taken)
}

fn period_of(mode_bits: u8, spi2x: bool) -> u64 {
    let spr = mode_bits & 0x03;
    RATES
        .iter()
        .find(|row| row.0 == spi2x && row.1 == spr)
        .map(|row| row.2)
        .unwrap()
}

const CPOL: u8 = 1 << 3;
const SPCR_IO: u16 = 0x2C;

/// §19.5.1. Writing `CPOL` drives SCK to that idle level at the store's retire.
#[test]
fn cpol_idle_level_is_the_spcr_store() {
    // SCK starts high, so CPOL 0 has a falling edge to show. CPOL 1 starts low.
    // `LDI` is 1 cycle and `OUT` is 1. The `OUT` starts at cycle 1 and retires
    // at cycle 2, which is the reported edge.
    let mut low = Cpu::new();
    low.poke(DDRB, 1 << 5);
    low.poke(PORTB, 1 << 5);
    while low.take_pin().is_some() {}
    low.load_words(&[ldi(16, u16::from(SPE | MSTR)), out(16, SPCR_IO), spin()]);
    low.insns(2);
    assert_eq!(
        edges(&mut low),
        vec![(SCK, false, 2)],
        "CPOL 0 drives SCK low at the SPCR retire"
    );
    assert_eq!(low.pin_levels().get(SCK), Some(Level::Low));

    let mut high = Cpu::new();
    high.poke(DDRB, 1 << 5);
    while high.take_pin().is_some() {}
    high.load_words(&[
        ldi(16, u16::from(SPE | MSTR | CPOL)),
        out(16, SPCR_IO),
        spin(),
    ]);
    high.insns(2);
    assert_eq!(
        edges(&mut high),
        vec![(SCK, true, 2)],
        "CPOL 1 drives SCK high at the SPCR retire"
    );
    assert_eq!(high.pin_levels().get(SCK), Some(Level::High));
    assert!(high.take(Channel::Spi).is_none(), "no SPDR write yet");

    let mut cpu = Cpu::new();
    cpu.poke(DDRB, 1 << 5);
    while cpu.take_pin().is_some() {}
    cpu.load_words(&[
        ldi(16, u16::from(SPE | MSTR)),
        out(16, SPCR_IO),
        ldi(16, u16::from(SPE | MSTR | CPOL)),
        out(16, SPCR_IO),
        spin(),
    ]);
    cpu.insns(4);
    // Two `LDI`/`OUT` pairs. The second `OUT` starts at 3 and retires at 4.
    assert_eq!(
        edges(&mut cpu),
        vec![(SCK, true, 4)],
        "a later CPOL write moves idle SCK at that retire"
    );
    assert_eq!(cpu.pin_levels().get(SCK), Some(Level::High));
}

fn all_edges(cpu: &mut Cpu) -> Vec<(u32, Level, u64, PinOrigin)> {
    let mut out = Vec::new();
    while let Some(report) = cpu.take_pin() {
        out.push((report.pin, report.level, report.cycle, report.origin));
    }
    out
}

/// Clearing `SPE` releases SCK and MOSI at that store's retire (§19.5.1).
///
/// The master MISO force (Table 19-1) has no visible edge here.
/// `DDRB4` is already an input, so forcing MISO to an input does not move it.
#[test]
fn clearing_spe_releases_sck_and_mosi_at_the_second_out() {
    // DDRB5 and DDRB3 are outputs held low, so GPIO wants both pins low.
    // A poke plants that level and does not report it, so the first pin
    // stamp emits both lows. CPOL 1 drives SCK high on that same stamp.
    // Two `LDI`/`OUT` pairs. `OUT` is 1 cycle. The first retires at 2.
    // The second starts at 3 and retires at 4, and that refresh clears the
    // SCK and MOSI overrides. SCK returns to the GPIO level there. MOSI was
    // never given an SPI level, so the clear leaves it low and adds no edge.
    let mut cpu = Cpu::new();
    cpu.poke(DDRB, (1 << 3) | (1 << 5));
    while cpu.take_pin().is_some() {}
    cpu.load_words(&[
        ldi(16, u16::from(SPE | MSTR | CPOL)),
        out(16, SPCR_IO),
        ldi(16, 0),
        out(16, SPCR_IO),
        spin(),
    ]);
    cpu.insns(4);
    assert_eq!(cpu.peek(SPCR) & SPE, 0);
    assert_eq!(
        all_edges(&mut cpu),
        vec![
            (MOSI, Level::Low, 2, PinOrigin::Gpio),
            (SCK, Level::Low, 2, PinOrigin::Gpio),
            (SCK, Level::High, 2, PinOrigin::Spi),
            (SCK, Level::Low, 4, PinOrigin::Gpio),
        ],
        "SPE clear releases SCK at the second OUT's retire"
    );
    assert_eq!(cpu.pin_levels().get(SCK), Some(Level::Low));
    assert_eq!(cpu.pin_levels().get(MOSI), Some(Level::Low));
}

/// §19.3.2. `SS` held low while `MSTR` is written clears `MSTR`, sets `SPIF`,
/// and moves the pins at that store's retire.
#[test]
fn spcr_with_ss_low_faults_at_the_retire() {
    // `LDI` is 1 cycle and `OUT` is 1. The `OUT` starts at cycle 1 and retires
    // at cycle 2. DDRB3 and DDRB5 are outputs held low, so forcing them to
    // inputs releases them at that retire.
    let mut cpu = Cpu::new();
    cpu.poke(DDRB, (1 << 3) | (1 << 5));
    cpu.drive_pin(SS, Level::Low);
    while cpu.take_pin().is_some() {}
    cpu.load_words(&[ldi(16, u16::from(SPE | MSTR)), out(16, SPCR_IO), spin()]);
    cpu.insns(2);
    assert_eq!(cpu.peek(SPCR) & MSTR, 0);
    assert_eq!(cpu.peek(SPSR) & SPIF, SPIF);
    let moved: Vec<_> = all_edges(&mut cpu)
        .into_iter()
        .filter(|edge| edge.0 == MOSI || edge.0 == SCK)
        .collect();
    assert_eq!(
        moved,
        vec![
            (MOSI, Level::Released, 2, PinOrigin::Gpio),
            (SCK, Level::Released, 2, PinOrigin::Gpio),
        ],
        "the mode fault moves MOSI and SCK at the SPCR retire"
    );
}

#[test]
fn table_19_5_rates_and_the_first_edge_follow_the_write() {
    // §19.2 starts the generator at the write. §19.5 / Table 19-5 is the period.
    // The first edge is one half-period later. /16 from a retire at cycle 2
    // edges at 10, which is not a multiple of 16 from reset.
    for &(spi2x, spr, period) in RATES {
        let (pins, taken) = run_master(spr, spi2x, 0xA5, false);
        let expect = frame_edges(2, 0xA5, period, false, false, false);
        assert_eq!(pins, expect, "SPI2X={spi2x} SPR={spr}");
        assert_eq!(taken, (0xA5, 2 + 8 * period));
        let first_sck = pins.iter().find(|edge| edge.0 == SCK).unwrap();
        assert!(first_sck.1);
        assert_eq!(first_sck.2, 2 + period / 2, "half of {period}");
    }
}

#[test]
fn sts_retires_one_cycle_later_than_out() {
    // `STS` is 2 cycles. The generator uses that retire, cycle 3, not the
    // start of the store.
    let mut cpu = Cpu::new();
    arm(&mut cpu, 1, false);
    cpu.load_words(&xfer_sts(0xA5));
    cpu.run_until(3 + 8 * 16 + 2);
    let pins = edges(&mut cpu);
    assert_eq!(pins, frame_edges(3, 0xA5, 16, false, false, false));
    assert_eq!(cpu.take(Channel::Spi), Some((0xA5, 3 + 128)));
}

#[test]
fn mode_0_frame_of_0xa5_is_these_edges() {
    // Figure 19-3, `CPHA` = 0, `CPOL` = 0, MSB first, fosc/4. 0xA5 is
    // 1,0,1,0,0,1,0,1. Setup of bit 0 is the retire. SCK rises one half later.
    let (pins, taken) = run_master(0, false, 0xA5, false);
    assert_eq!(
        pins,
        vec![
            (MOSI, true, 2),
            (SCK, true, 4),
            (MOSI, false, 6),
            (SCK, false, 6),
            (SCK, true, 8),
            (MOSI, true, 10),
            (SCK, false, 10),
            (SCK, true, 12),
            (MOSI, false, 14),
            (SCK, false, 14),
            (SCK, true, 16),
            (SCK, false, 18),
            (SCK, true, 20),
            (MOSI, true, 22),
            (SCK, false, 22),
            (SCK, true, 24),
            (MOSI, false, 26),
            (SCK, false, 26),
            (SCK, true, 28),
            (MOSI, true, 30),
            (SCK, false, 30),
            (SCK, true, 32),
            (SCK, false, 34),
        ]
    );
    assert_eq!(taken, (0xA5, 34));
}

#[test]
fn four_modes_and_both_bit_orders_match_the_figures() {
    // Table 19-2 / 19-3 / 19-4 and Figures 19-3 and 19-4. §19.5.1: `DORD` = 1
    // sends LSB first. 0xA6 is not a bit palindrome, so the two orders differ.
    for mode in 0u8..4 {
        for lsb in [false, true] {
            let mut bits = mode << 2;
            if lsb {
                bits |= DORD;
            }
            let (jumped, jump_take) = run_master(bits, false, 0xA6, false);
            let (stepped, step_take) = run_master(bits, false, 0xA6, true);
            let expect = frame_edges(2, 0xA6, 4, mode & 2 != 0, mode & 1 != 0, lsb);
            assert_eq!(jumped, expect, "mode {mode} lsb {lsb}");
            assert_eq!(stepped, jumped, "stepped mode {mode} lsb {lsb}");
            assert_eq!(jump_take, (0xA6, 34));
            assert_eq!(step_take, jump_take);
        }
    }
}

/// Reads `SPSR` then `SPDR` into r17 once the transfer that `xfer_out` started is done.
fn received(cpu: &mut Cpu) -> u8 {
    cpu.run_until(36);
    cpu.load_words(&[in_io(16, SPSR_IO), in_io(17, SPDR_IO), spin()]);
    let at = cpu.cycles();
    cpu.run_until(at + 2);
    cpu.reg(17)
}

#[test]
fn a_master_reads_miso_from_its_pad() {
    // Table 19-1: in master mode MISO is an input, so the byte is what its pad holds.
    for (level, byte) in [(Level::High, 0xFF), (Level::Low, 0x00)] {
        let mut cpu = Cpu::new();
        arm(&mut cpu, 0, false);
        cpu.drive_pin(MISO, level);
        cpu.load_words(&xfer_out(0xA5));
        assert_eq!(received(&mut cpu), byte, "MISO {level:?}");
        assert_eq!(cpu.peek(SPSR) & SPIF, 0, "SPSR then SPDR clears SPIF");
    }
}

#[test]
fn a_master_never_drives_miso_whatever_ddrb_says() {
    // Table 19-1 overrides MISO's direction to input in master mode.
    let mut cpu = Cpu::new();
    cpu.poke(DDRB, (1 << 3) | (1 << 4) | (1 << 5));
    cpu.poke(SPCR, SPE | MSTR);
    cpu.load_words(&xfer_out(0xFF));
    cpu.run_until(36);
    assert_eq!(cpu.pin_levels().drive(MISO), Some(PinDrive::Off));
    assert!(
        edges(&mut cpu).iter().all(|&(pin, _, _)| pin != MISO),
        "no SPI edge on MISO"
    );
}

/// Three transfers of 0x55 into r17, r18 and r19, each waiting on `SPIF`.
fn three_bytes() -> Vec<u16> {
    let rjmp = |from: usize, to: usize| 0xC000 | ((to as i32 - from as i32 - 1) as u16 & 0x0FFF);
    let mut words = vec![ldi(16, 0x55)];
    for reg in [17, 18, 19] {
        let wait = words.len() + 1;
        words.extend([
            out(16, SPDR_IO),
            in_io(20, SPSR_IO),
            sbrs(20, 7),
            rjmp(wait + 2, wait),
            in_io(reg, SPDR_IO),
        ]);
    }
    words.push(spin());
    words
}

#[test]
fn a_mosi_to_miso_loopback_reads_back_every_byte_in_every_mode() {
    // §19.4 and Table 19-3: the master samples MISO on the edge opposite the one MOSI changes
    // on, so a wire from MOSI to MISO reads back each byte sent, the first one too. The host
    // here is a timeline that copies MOSI onto MISO's pad at each halt, as a net does.
    for mode_bits in [0, CPHA, CPOL, CPOL | CPHA, DORD] {
        for spi2x in [false, true] {
            let mut cpu = Cpu::new();
            arm(&mut cpu, mode_bits, spi2x);
            cpu.load_words(&three_bytes());
            let mut timeline = Timeline::new(cpu);
            let mut mosi = timeline.drives()[MOSI as usize];
            timeline.set_pad(MISO, mosi == PinDrive::High);
            while timeline.run_until(2_000).is_some() {
                let drive = timeline.drives()[MOSI as usize];
                if drive != mosi {
                    mosi = drive;
                    timeline.set_pad(MISO, drive == PinDrive::High);
                }
            }
            let cpu = timeline.cpu();
            assert_eq!(
                [cpu.reg(17), cpu.reg(18), cpu.reg(19)],
                [0x55; 3],
                "mode bits {mode_bits:#04x} SPI2X {spi2x}"
            );
        }
    }
}

#[test]
fn a_miso_edge_mid_byte_reaches_only_the_bits_sampled_after_it() {
    // MISO rises halfway through the byte, landed by the timeline at the cycle it reached
    // while the core may sit past it. The bits sampled before read 0 and the rest read 1.
    let mut cpu = Cpu::new();
    arm(&mut cpu, 0, false);
    cpu.drive_pin(MISO, Level::Low);
    cpu.load_words(&xfer_out(0x00));
    let mut timeline = Timeline::new(cpu);
    while let Some(halt) = timeline.run_until(60) {
        if halt.at >= 18 {
            break;
        }
    }
    timeline.set_pad(MISO, true);
    while timeline.run_until(60).is_some() {}
    let mut cpu = timeline.cpu().clone();
    cpu.load_words(&[in_io(16, SPSR_IO), in_io(17, SPDR_IO), spin()]);
    let at = cpu.cycles();
    cpu.run_until(at + 2);
    let byte = cpu.reg(17);
    assert!(
        byte != 0 && byte != 0xFF && byte & byte.wrapping_add(1) == 0,
        "{byte:#04x}: the low bits read 1"
    );
}

#[test]
fn spif_stays_set_until_spsr_then_spdr_and_wcol_uses_the_same_sequence() {
    // §19.5.2. Reading SPDR alone leaves SPIF. A write during the shift sets
    // WCOL and does not change the shifter. Reading SPSR with WCOL set and then
    // writing SPDR during the shift clears WCOL and sets it again.
    let mut cpu = Cpu::new();
    arm(&mut cpu, 0, false);
    cpu.load_words(&xfer_out(0x11));
    cpu.run_until(36);
    assert_eq!(cpu.peek(SPSR) & SPIF, SPIF);
    cpu.load_words(&[in_io(16, SPDR_IO), spin()]);
    let at = cpu.cycles();
    cpu.run_until(at + 1);
    assert_eq!(
        cpu.peek(SPSR) & SPIF,
        SPIF,
        "SPDR alone does not clear SPIF"
    );

    cpu.load_words(&[in_io(16, SPSR_IO), in_io(17, SPDR_IO), spin()]);
    let at = cpu.cycles();
    cpu.run_until(at + 2);
    assert_eq!(cpu.peek(SPSR) & SPIF, 0);

    let mut cpu = Cpu::new();
    arm(&mut cpu, 0, false);
    let mut words = vec![
        ldi(16, 0x11),
        out(16, SPDR_IO),
        ldi(16, 0x22),
        out(16, SPDR_IO),
    ];
    words.push(spin());
    cpu.load_words(&words);
    cpu.run_until(8);
    assert_eq!(cpu.peek(SPSR) & WCOL, WCOL);
    cpu.run_until(36);
    assert_eq!(
        cpu.take(Channel::Spi),
        Some((0x11, 34)),
        "the collided write is dropped"
    );

    cpu.load_words(&[in_io(16, SPSR_IO), in_io(17, SPDR_IO), spin()]);
    let at = cpu.cycles();
    cpu.run_until(at + 2);
    assert_eq!(cpu.peek(SPSR) & (SPIF | WCOL), 0);

    let mut cpu = Cpu::new();
    arm(&mut cpu, 0, false);
    let mut words = vec![ldi(16, 0x11), out(16, SPDR_IO), in_io(17, SPSR_IO)];
    words.extend([
        ldi(16, 0x22),
        out(16, SPDR_IO),
        in_io(18, SPSR_IO),
        ldi(16, 0x33),
        out(16, SPDR_IO),
        spin(),
    ]);
    cpu.load_words(&words);
    cpu.run_until(12);
    assert_eq!(cpu.peek(SPSR) & WCOL, WCOL, "the clearing write collides");
}

#[test]
fn receive_keeps_the_last_completed_byte_until_spdr_is_read() {
    // §19.2. Transmit is single-buffered. A second completion replaces the
    // unread byte. A read during the next shift returns the previous byte.
    let mut cpu = Cpu::new();
    arm(&mut cpu, 0, false);
    cpu.drive_pin(MISO, Level::High);
    cpu.load_words(&xfer_out(0x01));
    cpu.run_until(36);
    cpu.drive_pin(MISO, Level::Low);
    cpu.load_words(&xfer_out(0x02));
    cpu.run_until(cpu.cycles() + 2 + 32 + 2);
    cpu.load_words(&[in_io(16, SPSR_IO), in_io(17, SPDR_IO), spin()]);
    let at = cpu.cycles();
    cpu.run_until(at + 2);
    assert_eq!(cpu.reg(17), 0x00, "the unread 0xFF was replaced");

    let mut cpu = Cpu::new();
    arm(&mut cpu, 0, false);
    cpu.drive_pin(MISO, Level::High);
    cpu.load_words(&xfer_out(0x01));
    cpu.run_until(36);
    cpu.drive_pin(MISO, Level::Low);
    cpu.load_words(&[
        ldi(16, 0x02),
        out(16, SPDR_IO),
        in_io(17, SPSR_IO),
        in_io(18, SPDR_IO),
        spin(),
    ]);
    let at = cpu.cycles();
    cpu.run_until(at + 4);
    assert_eq!(cpu.reg(18), 0xFF, "the shifter is not what SPDR reads");
    cpu.run_until(at + 4 + 32);
    cpu.load_words(&[in_io(16, SPSR_IO), in_io(17, SPDR_IO), spin()]);
    let at = cpu.cycles();
    cpu.run_until(at + 2);
    assert_eq!(cpu.reg(17), 0x00);
}

#[test]
fn master_ss_driven_low_clears_mstr_and_sets_spif() {
    // §19.3. SS as an input, driven low, clears MSTR and sets SPIF.
    // SS as an output is GPIO, even when the pin is low.
    let mut cpu = Cpu::new();
    cpu.poke(SPCR, SPE | MSTR);
    cpu.drive_pin(SS, Level::Low);
    assert_eq!(cpu.peek(SPCR) & MSTR, 0);
    assert_eq!(cpu.peek(SPSR) & SPIF, SPIF);

    let mut cpu = Cpu::new();
    cpu.poke(DDRB, 1 << 2);
    cpu.poke(PORTB, 0);
    cpu.poke(SPCR, SPE | MSTR);
    assert_eq!(cpu.peek(SPCR) & MSTR, MSTR);
    cpu.drive_pin(SS, Level::Low);
    assert_eq!(cpu.peek(SPCR) & MSTR, MSTR);
}

#[test]
fn slave_shifts_a_host_master_byte_and_ss_high_drops_a_partial() {
    // §19.2 slave and §19.3. SS low enables the shift. SS high resets it,
    // so a later full frame is not a continuation of the partial one.
    // Period 8 gives each half four CPU cycles, longer than the two-cycle minimum.
    let mut cpu = Cpu::new();
    cpu.load_words(&[spin()]);
    cpu.poke(DDRB, 1 << 4);
    cpu.poke(SPCR, SPE);
    cpu.poke(SPDR, 0xF0);
    cpu.drive_pin(SS, Level::Low);
    cpu.run_cycles(4);
    cpu.drive_pin(SCK, Level::High);
    cpu.run_cycles(4);
    cpu.drive_pin(SCK, Level::Low);
    cpu.run_cycles(4);
    cpu.drive_pin(SS, Level::High);
    cpu.run_cycles(4);
    assert_eq!(cpu.peek(SPSR) & SPIF, 0, "one bit is not a completed byte");
    let miso = clock_spi(
        &mut cpu,
        &[0x3C],
        SpiClock {
            period: 8,
            mode: 0,
            lsb_first: false,
        },
    );
    assert_eq!(miso, vec![0xF0]);
    assert_eq!(cpu.peek(SPDR), 0x3C);
}

fn slave_ready(byte: u8) -> Cpu {
    let mut cpu = Cpu::new();
    cpu.load_words(&[spin()]);
    cpu.poke(DDRB, 1 << 4);
    cpu.poke(SPCR, SPE);
    cpu.poke(SPDR, byte);
    cpu
}

fn clock8() -> SpiClock {
    SpiClock {
        period: 8,
        mode: 0,
        lsb_first: false,
    }
}

#[test]
fn a_second_slave_byte_echoes_what_the_shifter_received() {
    // §19.2. Transmit is single-buffered. After SPDR = 0xF0 the host clocks
    // 0x3C then 0xA5 with no second write. MISO is the written byte, then
    // the byte the shifter just took in. SPDR reads the second byte.
    let mut cpu = slave_ready(0xF0);
    let miso = clock_spi(&mut cpu, &[0x3C, 0xA5], clock8());
    assert_eq!(miso, vec![0xF0, 0x3C]);
    assert_eq!(cpu.peek(SPDR), 0xA5);
}

#[test]
fn a_write_between_slave_bytes_replaces_the_echo() {
    // The same two clocks, with SPDR written after the first byte. That
    // write replaces the shifter, so the second MISO byte is the write.
    let mut cpu = slave_ready(0xF0);
    let first = clock_spi(&mut cpu, &[0x3C], clock8());
    assert_eq!(first, vec![0xF0]);
    cpu.poke(SPDR, 0x5A);
    let second = clock_spi(&mut cpu, &[0xA5], clock8());
    assert_eq!(second, vec![0x5A]);
    assert_eq!(cpu.peek(SPDR), 0xA5);
}

#[test]
fn a_slave_write_while_ss_stays_low_presents_bit_0() {
    // §19.2 / Figure 19-3. CPHA 0 keeps the next bit on MISO before the
    // sample. SS stays low across both bytes. Period 8: the first SPIF is
    // cycle 64 and the next leading edge, which samples, is 68. A write at
    // 66 replaces that bit. CPHA 1 would present it on the leading edge.
    let mut cpu = slave_ready(0xF0);
    assert!(cpu.queue_spi_master(0, &[0x3C, 0x11], clock8()));
    cpu.run_until(66);
    cpu.poke(SPDR, 0xA5);
    cpu.run_until(2 * 64 + 2);
    let got: Vec<_> = std::iter::from_fn(|| cpu.take(Channel::SpiHostMaster)).collect();
    assert_eq!(got, vec![(0xF0, 64), (0xA5, 128)]);
}

#[test]
fn a_slave_write_inside_a_bit_sets_wcol_and_keeps_the_byte() {
    // The leading edge of the second byte is cycle 68, so cycle 70 is inside
    // that bit. The write sets WCOL and the shifter keeps the echoed 0x3C.
    let mut cpu = slave_ready(0xF0);
    assert!(cpu.queue_spi_master(0, &[0x3C, 0x11], clock8()));
    cpu.run_until(70);
    cpu.poke(SPDR, 0xA5);
    assert_eq!(cpu.peek(SPSR) & WCOL, WCOL);
    cpu.run_until(2 * 64 + 2);
    let got: Vec<u8> =
        std::iter::from_fn(|| cpu.take(Channel::SpiHostMaster).map(|(byte, _)| byte)).collect();
    assert_eq!(got, vec![0xF0, 0x3C]);
}

#[test]
fn a_queued_host_master_matches_across_run_slices() {
    // queue_spi_master arms pad events and returns. Slices of run_until
    // deliver the same MISO bytes, last-SCK stamps, and pin edges as one run.
    // Period 8: eight SCK periods put the last edge of byte i at
    // start + (i + 1) × 64.
    //
    // §19.2 / Figure 19-3, mode 0. SS low at `start` shows the output low,
    // then bit 7 of 0xF0 (11110000) is driven high. The first 0 is the
    // trailing edge start + 4×8. The echo 0x3C is 00111100, presented from
    // the end of the first byte (start + 64): its first 1 is bit 2 and the
    // following 0 is bit 6. The second byte ends at start + 128 and presents
    // 0xA5's MSB. SS high one cycle later releases MISO (§19.3).
    let once = host_master_trace(false);
    let sliced = host_master_trace(true);
    assert_eq!(once.pins, sliced.pins, "pin edges");
    assert_eq!(once.bytes, sliced.bytes, "MISO stamps");
    assert_eq!(once.miso, sliced.miso, "MISO wire");
    let start = once.start;
    let period = 8;
    let echo = |bit: u64| start + 8 * period + bit * period;
    assert_eq!(once.bytes, vec![(0xF0, start + 64), (0x3C, start + 2 * 64)]);
    assert_eq!(
        once.miso,
        vec![
            (Level::Low, start),
            (Level::High, start),
            (Level::Low, start + 4 * period),
            (Level::High, echo(2)),
            (Level::Low, echo(6)),
            (Level::High, start + 16 * period),
            (Level::Released, start + 16 * period + 1),
        ]
    );
}

struct HostTrace {
    pins: Vec<PinEdge>,
    bytes: Vec<(u8, u64)>,
    start: u64,
    miso: Vec<(Level, u64)>,
}

fn host_master_trace(sliced: bool) -> HostTrace {
    let mut cpu = slave_ready(0xF0);
    let start = cpu.cycles();
    assert!(cpu.queue_spi_master(start, &[0x3C, 0xA5], clock8()));
    assert_eq!(cpu.cycles(), start, "queue_spi_master does not run the CPU");
    let end = start + 2 * 64 + 1;
    if sliced {
        while cpu.cycles() < end + 1 {
            let step = (cpu.cycles() + 3).min(end + 1);
            cpu.run_until(step);
        }
    } else {
        cpu.run_until(end + 1);
    }
    let mut pins = Vec::new();
    let mut miso = Vec::new();
    while let Some(report) = cpu.take_pin() {
        if report.pin == MISO {
            miso.push((report.level, report.cycle));
        }
        if report.origin == PinOrigin::Spi {
            pins.push((report.pin, report.level == Level::High, report.cycle));
        }
    }
    let mut bytes = Vec::new();
    while let Some(byte) = cpu.take(Channel::SpiHostMaster) {
        bytes.push(byte);
    }
    HostTrace {
        pins,
        bytes,
        start,
        miso,
    }
}

#[test]
fn a_drive_during_a_queued_transfer_keeps_its_cycle() {
    // A host drive inside the transfer is applied at that cycle. A call that
    // ran the CPU to the end of the clocks would stamp it at the end instead.
    let mut cpu = slave_ready(0xF0);
    let start = cpu.cycles();
    assert!(cpu.queue_spi_master(start, &[0x3C, 0xA5], clock8()));
    cpu.poke(0x002A, 1);
    cpu.poke(0x002B, 0);
    let mid = start + 20;
    cpu.run_until(mid);
    cpu.drive_pin(Port::D.pin_id(0), Level::High);
    assert_eq!(cpu.cycles(), mid);
    cpu.run_until(start + 2 * 64 + 2);
    let bytes: Vec<u8> =
        std::iter::from_fn(|| cpu.take(Channel::SpiHostMaster).map(|(byte, _)| byte)).collect();
    assert_eq!(bytes, vec![0xF0, 0x3C]);
}

#[test]
fn a_master_and_a_host_master_that_finish_together_use_two_channels() {
    // SS is an output, so the host's SS low does not clear MSTR (§19.3).
    // LDI plus OUT retires at cycle 2. Period 4 sets SPIF at 34. The host
    // master, queued at that same retire, samples its last SCK edge on
    // that cycle too. Each byte stays on its own channel. MISO is an input in master mode
    // (Table 19-1), so the host's own high on it is what the host master reads.
    let mut cpu = Cpu::new();
    cpu.poke(DDRB, 1 << 2);
    cpu.poke(SPCR, SPE | MSTR);
    cpu.drive_pin(MISO, Level::High);
    cpu.load_words(&[ldi(16, 0xA5), out(16, SPDR_IO), spin()]);
    assert!(cpu.queue_spi_master(
        2,
        &[0x00],
        SpiClock {
            period: 4,
            mode: 0,
            lsb_first: false,
        },
    ));
    cpu.run_until(40);
    assert_eq!(cpu.take(Channel::Spi), Some((0xA5, 34)));
    assert_eq!(cpu.take(Channel::Spi), None);
    assert_eq!(cpu.take(Channel::SpiHostMaster), Some((0xFF, 34)));
    assert_eq!(cpu.take(Channel::SpiHostMaster), None);
}

#[test]
fn a_queue_while_busy_or_with_no_bytes_changes_nothing() {
    // Empty bytes is rejected before anything is armed. A second queue
    // while the first transfer is active is rejected, and that transfer's
    // MISO bytes and pin edges stay the ones from the first queue.
    let mut idle = slave_ready(0xF0);
    assert!(!idle.queue_spi_master(0, &[], clock8()));
    assert_eq!(idle.cycles(), 0);
    idle.run_until(2 * 64 + 2);
    assert!(edges(&mut idle).is_empty());
    assert_eq!(idle.take(Channel::SpiHostMaster), None);

    let HostTrace { pins, bytes, .. } = host_master_trace(false);
    let mut cpu = slave_ready(0xF0);
    assert!(cpu.queue_spi_master(0, &[0x3C, 0xA5], clock8()));
    assert!(!cpu.queue_spi_master(0, &[], clock8()));
    cpu.run_until(70);
    assert!(!cpu.queue_spi_master(70, &[0x11], clock8()));
    cpu.run_until(2 * 64 + 2);
    let again = edges(&mut cpu);
    let mut got = Vec::new();
    while let Some(byte) = cpu.take(Channel::SpiHostMaster) {
        got.push(byte);
    }
    assert_eq!(again, pins);
    assert_eq!(got, bytes);
}

#[test]
fn table_19_1_forces_the_idle_direction() {
    // Master: MISO is an input even when DDR is set. Slave: MOSI, SCK, and SS
    // are inputs. SS high tri-states slave MISO.
    let mut cpu = Cpu::new();
    cpu.poke(DDRB, 1 << 4);
    cpu.poke(PORTB, 1 << 4);
    cpu.poke(SPCR, SPE | MSTR);
    cpu.drive_pin(MISO, Level::Low);
    assert_eq!(cpu.pin_levels().get(MISO), Some(Level::Low));

    let mut cpu = Cpu::new();
    cpu.poke(DDRB, (1 << 3) | (1 << 5) | (1 << 2));
    cpu.poke(PORTB, (1 << 3) | (1 << 5) | (1 << 2));
    cpu.poke(SPCR, SPE);
    cpu.drive_pin(MOSI, Level::Low);
    cpu.drive_pin(SCK, Level::Low);
    assert_eq!(cpu.pin_levels().get(MOSI), Some(Level::Low));
    assert_eq!(cpu.pin_levels().get(SCK), Some(Level::Low));
    cpu.drive_pin(MISO, Level::Low);
    assert_eq!(
        cpu.pin_levels().get(MISO),
        Some(Level::Low),
        "SS high tri-states MISO"
    );
}

#[test]
fn prspi_discards_a_write_and_freezes_a_transfer() {
    // §10.9. PRSPI stops the SPI clock. A store while it is set does not start
    // a generator. A transfer already running resumes with the lost cycles added.
    let mut cpu = Cpu::new();
    arm(&mut cpu, 3, false);
    cpu.poke(PRR, PRSPI);
    cpu.load_words(&xfer_out(0xA5));
    cpu.run_until(40);
    assert_eq!(cpu.peek(SPSR) & SPIF, 0);
    assert_eq!(cpu.take(Channel::Spi), None);

    let mut cpu = Cpu::new();
    arm(&mut cpu, 3, false);
    cpu.load_words(&xfer_out(0xA5));
    cpu.run_until(20);
    let frozen = cpu.cycles();
    cpu.poke(PRR, PRSPI);
    cpu.run_until(frozen + 500);
    assert_eq!(cpu.peek(SPSR) & SPIF, 0);
    let resume = cpu.cycles();
    cpu.poke(PRR, 0);
    let spif = 2 + 8 * 128 + (resume - frozen);
    cpu.run_until(spif + 2);
    assert_eq!(cpu.take(Channel::Spi), Some((0xA5, spif)));
}

#[test]
fn the_spi_vector_is_0x22_and_clears_spif_but_not_wcol() {
    // Table 12-6 vector 18, word 0x22. Entry clears SPIF (§19.5.2) and leaves WCOL.
    let mut cpu = Cpu::new();
    cpu.poke(DDRB, (1 << 3) | (1 << 5));
    cpu.poke(SPCR, SPIE | SPE | MSTR);
    cpu.set_sreg(0x80);
    cpu.load_words(&with_vector(&[
        ldi(16, 0x11),
        out(16, SPDR_IO),
        ldi(16, 0x22),
        out(16, SPDR_IO),
        spin(),
    ]));
    cpu.run_until(34 + 16);
    assert_eq!(cpu.pc(), 0x0022);
    assert_eq!(cpu.peek(SPSR) & SPIF, 0);
    assert_eq!(cpu.peek(SPSR) & WCOL, WCOL);
}

#[test]
fn spi_interrupt_wakes_idle_and_not_power_down() {
    // Table 10-1. SPI is "other I/O": Idle only. Power-down stops clkI/O,
    // so the /128 frame never sets SPIF.
    let mut idle = Cpu::new();
    idle.poke(DDRB, (1 << 3) | (1 << 5));
    idle.poke(SPCR, SPIE | SPE | MSTR | 0x03);
    idle.poke(SMCR, 0x01);
    idle.set_sreg(0x80);
    idle.load_words(&with_vector(&[ldi(16, 0xA5), out(16, SPDR_IO), 0x9588]));
    idle.run_until(2 + 8 * 128 + 32);
    assert!(!idle.is_sleeping());
    assert_eq!(idle.pc(), 0x0022);

    let mut down = Cpu::new();
    down.poke(DDRB, (1 << 3) | (1 << 5));
    down.poke(SPCR, SPIE | SPE | MSTR | 0x03);
    down.poke(SMCR, 0x05);
    down.set_sreg(0x80);
    down.load_words(&with_vector(&[ldi(16, 0xA5), out(16, SPDR_IO), 0x9588]));
    down.run_until(4_000);
    assert!(down.is_sleeping());
    assert_eq!(down.peek(SPSR) & SPIF, 0);
}

#[test]
fn spi_pins_are_pb2_through_pb5() {
    // §14.3.1.
    assert_eq!(Port::B.pin_id(2), SS);
    assert_eq!(Port::B.pin_id(3), MOSI);
    assert_eq!(Port::B.pin_id(4), MISO);
    assert_eq!(Port::B.pin_id(5), SCK);
}
