//! The timing set: programs written for this test, each marking time on a pin,
//! scored against the value the documents give.
//!
//! ISM is the instruction set manual DS40002198B, AVRe column. DS is the
//! datasheet DS40002061B. An `out` drives its pin on the clock edge that ends
//! it (DS Fig. 14-4), and cycle 0 is the first instruction. A row's source
//! names the model parameter of `docs/engine/avr.md` it relies on.
//!
//! `timing/hex` holds the builds of `timing/asm` for the ATmega328P, with no
//! start files or libraries, made by toolchain 7.3.0-atmel3.6.1:
//!
//! ```text
//! avr-gcc -mmcu=atmega328p -nostartfiles -nostdlib -x assembler-with-cpp asm/NAME.S -o NAME.elf
//! avr-objcopy -O ihex NAME.elf hex/NAME.hex
//! ```

use std::collections::BTreeMap;
use std::path::Path;

use orbe_microchip::{Cpu, Level, PIN_QUEUE_CAP, Port, parse_hex};

use Read::*;

/// Edge indices count from the pin's first rise.
#[derive(Debug, Clone, Copy)]
enum Read {
    /// `(pin, n)`: how long the `n`th rise stays high.
    Pulse(u32, usize),
    /// `(pin, n)`: how long the `n`th fall stays low.
    Gap(u32, usize),
    /// `(pin, n)`: the cycle of edge `n`.
    Edge(u32, usize),
    /// `(pin, n)`: edge `n` to edge `n + 1`.
    Period(u32, usize),
    /// `(pin, n)`: edge `n` to edge `n + 2`, one full wave.
    Wave(u32, usize),
    /// `(pin, from, n)`: edge `n` on `pin` after edge `n` on `from`.
    Lag(u32, u32, usize),
    /// `pin`: the number of edges.
    Count(u32),
    /// `PB5:0` at the end of the run.
    PortB,
}

struct Check {
    name: &'static str,
    read: Read,
    expected: u64,
    source: &'static str,
}

struct Program {
    name: &'static str,
    cycles: u64,
    checks: &'static [Check],
}

/// `program!(name, cycles; check: read => expected, source; ...)`, one check
/// per line.
macro_rules! program {
    ($name:literal, $cycles:expr; $($check:literal: $read:expr => $expected:expr, $source:expr;)*) => {
        Program {
            name: $name,
            cycles: $cycles,
            checks: &[$(Check { name: $check, read: $read, expected: $expected, source: $source }),*],
        }
    };
}

/// A TOV interrupt program. The store that starts the timer ends on edge
/// `start`, TOV comes every `clocks` timer clocks, and from `start` the main
/// loop is NOPs and an RJMP, `loop_len` cycles long.
struct IsrLoop {
    start: u64,
    clocks: u64,
    loop_len: u64,
}

impl IsrLoop {
    /// The edge of the `k`th TOV, from 1, at /`n`. The store's own edge counts
    /// with the stopped select (parameter 1) and the divider is 0 at cycle 0
    /// (parameter 3), so the first count is the first multiple of `n` past
    /// `start`. TOV rises on the MAX to 0 clock (DS 15.7.1, 16.9.1).
    const fn tov(&self, n: u64, k: u64) -> u64 {
        (self.start / n + 1) * n + (self.clocks - 1) * n + self.clocks * n * (k - 1)
    }

    /// The edge the `k`th ISR toggles PB5 on. A flag on the inner edge of the
    /// RJMP waits for it to end (parameter 2). Entry 4, JMP 3, and IN, LDI,
    /// EOR, OUT put the toggle 11 cycles after entry, and with RETI 4 the ISR
    /// puts the loop 15 cycles later (DS 7.7.1).
    const fn isr_out(&self, n: u64, k: u64) -> u64 {
        let tov = self.tov(n, k);
        let rjmp_inner = self.start + self.loop_len - 1 + 15 * (k - 1);
        let wait = if (tov - rjmp_inner).is_multiple_of(self.loop_len) {
            1
        } else {
            0
        };
        tov + wait + 11
    }

    /// From the second ISR's toggle to the third.
    const fn period(&self, n: u64) -> u64 {
        self.isr_out(n, 3) - self.isr_out(n, 2)
    }

    /// From OC0A's second toggle to the second ISR's. OC0A toggles one timer
    /// clock before TOV, on the clock TCNT0 reaches OCR0A = 0xFF.
    const fn lag(&self, n: u64) -> u64 {
        self.isr_out(n, 2) - (self.tov(n, 2) - n)
    }
}

/// `t0ovf_*`: `OUT TCCR0B` ends at edge 17, and the loop is 200 NOPs.
const T0OVF: IsrLoop = IsrLoop {
    start: 17,
    clocks: 256,
    loop_len: 202,
};

/// `t1ovf_*`: `STS TCCR1B` ends at edge 12, and the loop is one NOP.
const T1OVF: IsrLoop = IsrLoop {
    start: 12,
    clocks: 65_536,
    loop_len: 3,
};

const T0_TOV_ISR: &str = "DS 15.7.1 / Fig. 15-8: TOV0 every 256 timer clocks; DS 7.7.1 and parameter 2: each ISR's entry follows the loop phase (IsrLoop)";
const T1_TOV_ISR: &str = "DS 16.9.1 / Fig. 16-12: TOV1 every 65536 timer clocks; DS 7.7.1 and parameter 2: each ISR's entry follows the loop phase (IsrLoop)";
const OC0A_TO_ISR: &str = "DS Table 15-2 and §15.5: OC0A toggles on the clock TCNT0 reaches OCR0A = 0xFF; Fig. 15-8: TOV0 sets one timer clock later, as 0xFF goes to 0; DS 7.7.1: entry 4 + JMP 3 + IN, LDI, EOR, OUT 4, with parameter 2 for the entry (IsrLoop)";

struct Run {
    edges: BTreeMap<u32, Vec<(u64, Level)>>,
    port_b: u64,
}

fn run(program: &Program) -> Run {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/timing/hex");
    let text = std::fs::read_to_string(dir.join(format!("{}.hex", program.name))).unwrap();
    let mut cpu = Cpu::new();
    cpu.load_image(&parse_hex(&text).unwrap()).unwrap();
    let mut edges: BTreeMap<u32, Vec<(u64, Level)>> = BTreeMap::new();
    while cpu.cycles() < program.cycles {
        let horizon = (cpu.cycles() + PIN_QUEUE_CAP as u64).min(program.cycles);
        cpu.run_until(horizon);
        while let Some(report) = cpu.take_pin() {
            edges
                .entry(report.pin)
                .or_default()
                .push((report.cycle, report.level));
        }
    }
    assert_eq!(cpu.pin_overflow(), 0, "{}: pin edges dropped", program.name);
    edges.values_mut().for_each(from_first_rise);
    let levels = cpu.pin_levels();
    let port_b = (0..6)
        .filter(|&bit| levels.get(Port::B.pin_id(bit)) == Some(Level::High))
        .map(|bit| 1 << bit)
        .sum();
    Run { edges, port_b }
}

fn from_first_rise(seq: &mut Vec<(u64, Level)>) {
    let first = seq.iter().position(|&(_, level)| level == Level::High);
    seq.drain(..first.unwrap_or(seq.len()));
}

fn measure(read: Read, run: &Run) -> Option<u64> {
    let seq = |pin: u32| run.edges.get(&pin).map_or(&[][..], Vec::as_slice);
    let after = |pin: u32, k: usize| Some(seq(pin).get(k + 1)?.0 - seq(pin).get(k)?.0);
    let nth = |pin: u32, index: usize, level: Level| {
        seq(pin)
            .iter()
            .enumerate()
            .filter(|(_, (_, l))| *l == level)
            .nth(index)
            .map(|(k, _)| k)
    };
    match read {
        Pulse(pin, rise) => after(pin, nth(pin, rise, Level::High)?),
        Gap(pin, fall) => after(pin, nth(pin, fall, Level::Low)?),
        Edge(pin, edge) => Some(seq(pin).get(edge)?.0),
        Period(pin, edge) => after(pin, edge),
        Wave(pin, edge) => Some(seq(pin).get(edge + 2)?.0 - seq(pin).get(edge)?.0),
        Lag(pin, from, edge) => seq(pin).get(edge)?.0.checked_sub(seq(from).get(edge)?.0),
        Count(pin) => u64::try_from(seq(pin).len()).ok(),
        PortB => Some(run.port_b),
    }
}

#[test]
fn every_row_is_exact() {
    let runs: Vec<Run> = std::thread::scope(|scope| {
        let handles: Vec<_> = PROGRAMS
            .iter()
            .map(|program| scope.spawn(move || run(program)))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut rows = 0;
    let mut misses = Vec::new();
    for (program, run) in PROGRAMS.iter().zip(&runs) {
        for check in program.checks {
            rows += 1;
            let got = measure(check.read, run);
            if got != Some(check.expected) {
                misses.push(format!(
                    "{}: {}: got {got:?}, expected {} ({})",
                    program.name, check.name, check.expected, check.source
                ));
            }
        }
    }
    println!("timing set: {rows} rows");
    assert_eq!(rows, 107);
    assert!(misses.is_empty(), "{}", misses.join("\n"));
}

const PROGRAMS: &[Program] = &[
    program!("isa", 3000;
        "nop": Pulse(5, 0) => 2, "ISM Table 5-6 NOP";
        "add": Pulse(5, 1) => 2, "ISM Table 5-2 ADD";
        "adiw": Pulse(5, 2) => 3, "ISM Table 5-2 ADIW";
        "sbiw": Pulse(5, 3) => 3, "ISM Table 5-2 SBIW";
        "mul": Pulse(5, 4) => 3, "ISM Table 5-2 MUL";
        "movw": Pulse(5, 5) => 2, "ISM Table 5-4 MOVW";
        "rjmp": Pulse(5, 6) => 3, "ISM Table 5-3 RJMP";
        "jmp": Pulse(5, 7) => 4, "ISM Table 5-3 JMP";
        "ijmp": Pulse(5, 8) => 3, "ISM Table 5-3 IJMP";
        "breq taken": Pulse(5, 9) => 3, "ISM Table 5-3 BREQ 1/2";
        "breq not taken": Pulse(5, 10) => 2, "ISM Table 5-3 BREQ 1/2";
        "cpse no skip (+nop)": Pulse(5, 11) => 3, "ISM Table 5-3 CPSE 1/2/3";
        "cpse skip 1-word": Pulse(5, 12) => 3, "ISM Table 5-3 CPSE 1/2/3";
        "cpse skip 2-word": Pulse(5, 13) => 4, "ISM Table 5-3 CPSE 1/2/3";
        "sbrs skip 1-word": Pulse(5, 14) => 3, "ISM Table 5-3 SBRS 1/2/3";
        "sbrs skip 2-word": Pulse(5, 15) => 4, "ISM Table 5-3 SBRS 1/2/3";
        "sbrc no skip (+nop)": Pulse(5, 16) => 3, "ISM Table 5-3 SBRC 1/2/3";
        "sbis skip 1-word": Pulse(5, 17) => 3, "ISM Table 5-3 SBIS 1/2/3";
        "sbis skip 2-word": Pulse(5, 18) => 4, "ISM Table 5-3 SBIS 1/2/3";
        "sbic no skip (+nop)": Pulse(5, 19) => 3, "ISM Table 5-3 SBIC 1/2/3";
        "ld X": Pulse(5, 20) => 3, "ISM Table 5-4 LD";
        "ld X+": Pulse(5, 21) => 3, "ISM Table 5-4 LD";
        "ld -X": Pulse(5, 22) => 3, "ISM Table 5-4 LD";
        "ldd Y+q": Pulse(5, 23) => 3, "ISM Table 5-4 LDD";
        "lds": Pulse(5, 24) => 3, "ISM Table 5-4 LDS";
        "st X": Pulse(5, 25) => 3, "ISM Table 5-4 ST";
        "st -Y": Pulse(5, 26) => 3, "ISM Table 5-4 ST";
        "std Z+q": Pulse(5, 27) => 3, "ISM Table 5-4 STD";
        "sts": Pulse(5, 28) => 3, "ISM Table 5-4 STS";
        "push": Pulse(5, 29) => 3, "ISM Table 5-4 PUSH";
        "pop": Pulse(5, 30) => 3, "ISM Table 5-4 POP";
        "lpm": Pulse(5, 31) => 4, "ISM Table 5-4 LPM";
        "lpm Rd,Z+": Pulse(5, 32) => 4, "ISM Table 5-4 LPM";
        "rcall + ret": Pulse(5, 33) => 8, "ISM Table 5-3 RCALL 3, RET 4";
        "call + ret": Pulse(5, 34) => 9, "ISM Table 5-3 CALL 4, RET 4";
        "icall + ret": Pulse(5, 35) => 8, "ISM Table 5-3 ICALL 3, RET 4";
        "in": Pulse(5, 36) => 2, "ISM Table 5-4 IN";
        "out": Pulse(5, 37) => 2, "ISM Table 5-4 OUT";
        "sbi": Pulse(5, 38) => 3, "ISM Table 5-5 SBI";
        "cbi": Pulse(5, 39) => 3, "ISM Table 5-5 CBI";
        "dec/brne loop x10": Pulse(5, 40) => 31, "ISM Table 5-2 DEC, Table 5-3 BRNE 1/2";
    ),
    program!("pinwrite", 200;
        "out PORTB (cycle 2) -> high": Edge(5, 0) => 3, "DS Fig. 14-4: the pin follows at the clock edge ending the write; cycle 0 is the first instruction";
        "cbi PORTB (cycles 3-4) -> low": Edge(5, 1) => 5, "DS Fig. 14-4: the pin follows at the clock edge ending the write; cycle 0 is the first instruction; ISM Table 5-5 CBI 2";
        "st X (cycles 7-8) -> high": Edge(5, 2) => 9, "DS Fig. 14-4: the pin follows at the clock edge ending the write; cycle 0 is the first instruction; ISM Table 5-4 ST 2";
        "sts (cycles 10-11) -> low": Edge(5, 3) => 12, "DS Fig. 14-4: the pin follows at the clock edge ending the write; cycle 0 is the first instruction; ISM Table 5-4 STS 2";
    ),
    program!("int_entry", 600;
        "EE_READY: sei, nop, entry, jmp, out": Pulse(5, 0) => 10, "DS 7.7.1: entry 4, vector JMP 3; DS 7.7: one instruction after SEI; 1+1+4+3+1";
        "cbi, reti, out": Gap(5, 0) => 7, "DS 7.7.1: RETI 4; ISM Table 5-5 CBI 2; 2+4+1";
    ),
    program!("int_reti", 600;
        "pending at reti: one main instruction first (lo->hi)": Gap(5, 2) => 5, "DS 7.7: one more instruction after RETI; RETI 4 + OUT 1";
        "re-entry (hi->lo)": Pulse(5, 3) => 8, "DS 7.7.1: entry 4 + JMP 3 + OUT 1";
    ),
    program!("t0ctc_1_p0", 2500;
        "T0 CTC /1 OCR0A=99: toggle period": Period(22, 1) => 100, "DS 15.7.2 fOC0 = fclk/(2N(1+OCR0A)); Fig. 15-11";
        "T0 CTC /1: first toggle (CS write ends at 8)": Edge(22, 0) => 107, "DS Fig. 14-4 and §15.8: the TCCR0B store lands on its ending edge 8, which still counts on the old select; §17.2, prescaler 0 at the first instruction: the first /1 clock is cycle 9; §15.5 and Fig. 15-11: OC0A toggles on the clock TCNT0 reaches OCR0A = 99, 9 + 98 x 1";
    ),
    program!("t0ctc_1_p3", 2500;
        "T0 CTC /1 OCR0A=99: toggle period": Period(22, 1) => 100, "DS 15.7.2 fOC0 = fclk/(2N(1+OCR0A)); Fig. 15-11";
        "T0 CTC /1: first toggle (CS write ends at 11)": Edge(22, 0) => 110, "DS Fig. 14-4 and §15.8: the TCCR0B store lands on its ending edge 11, which still counts on the old select; §17.2, prescaler 0 at the first instruction: the first /1 clock is cycle 12; §15.5 and Fig. 15-11: OC0A toggles on the clock TCNT0 reaches OCR0A = 99, 12 + 98 x 1";
    ),
    program!("t0ovf_1", 3280;
        "T0 normal /1: TOV ISR period": Period(5, 1) => T0OVF.period(1), T0_TOV_ISR;
        "T0 normal /1: OC0A toggle (OCR0A=0xFF) to ISR edge": Lag(5, 22, 1) => T0OVF.lag(1), OC0A_TO_ISR;
    ),
    program!("t1ctc_1", 6000;
        "T1 CTC /1 OCR1A=999: toggle period": Period(1, 1) => 1000, "DS 16.9.2 fOC1A = fclk/(2N(1+OCR1A)); Fig. 16-11";
    ),
    program!("t0fpwm_1", 3280;
        "T0 fast PWM /1 OCR0A=99: high time": Pulse(22, 1) => 99, "DS Table 15-3: non-inverting OC0A sets at BOTTOM and clears on the compare match; §15.5: the pin acts on the clock TCNT0 reaches OCR0A and the flag one clock later, so OC0A is high while TCNT0 is 0 to 98, 99 timer clocks";
        "T0 fast PWM /1: period": Wave(22, 1) => 256, "DS 15.7.3 fOC0PWM = fclk/(N*256)";
    ),
    program!("t1ovf_1", 231_376;
        "T1 normal /1: TOV1 ISR period": Period(5, 1) => T1OVF.period(1), T1_TOV_ISR;
    ),
    program!("t0ctc_8_p0", 6000;
        "T0 CTC /8 OCR0A=99: toggle period": Period(22, 1) => 800, "DS 15.7.2 fOC0 = fclk/(2N(1+OCR0A)); Fig. 15-11";
        "T0 CTC /8: first toggle (CS write ends at 8)": Edge(22, 0) => 800, "DS Fig. 14-4 and §15.8: the TCCR0B store lands on its ending edge 8, which still counts on the old select; §17.2, prescaler 0 at the first instruction: the first /8 clock is cycle 16; §15.5 and Fig. 15-11: OC0A toggles on the clock TCNT0 reaches OCR0A = 99, 16 + 98 x 8";
    ),
    program!("t0ctc_8_p3", 6000;
        "T0 CTC /8 OCR0A=99: toggle period": Period(22, 1) => 800, "DS 15.7.2 fOC0 = fclk/(2N(1+OCR0A)); Fig. 15-11";
        "T0 CTC /8: first toggle (CS write ends at 11)": Edge(22, 0) => 800, "DS Fig. 14-4 and §15.8: the TCCR0B store lands on its ending edge 11, which still counts on the old select; §17.2, prescaler 0 at the first instruction: the first /8 clock is cycle 16; §15.5 and Fig. 15-11: OC0A toggles on the clock TCNT0 reaches OCR0A = 99, 16 + 98 x 8";
    ),
    program!("t0ovf_8", 12_240;
        "T0 normal /8: TOV ISR period": Period(5, 1) => T0OVF.period(8), T0_TOV_ISR;
        "T0 normal /8: OC0A toggle (OCR0A=0xFF) to ISR edge": Lag(5, 22, 1) => T0OVF.lag(8), OC0A_TO_ISR;
    ),
    program!("t1ctc_8", 34_000;
        "T1 CTC /8 OCR1A=999: toggle period": Period(1, 1) => 8000, "DS 16.9.2 fOC1A = fclk/(2N(1+OCR1A)); Fig. 16-11";
    ),
    program!("t0fpwm_8", 12_240;
        "T0 fast PWM /8 OCR0A=99: high time": Pulse(22, 1) => 792, "DS Table 15-3: non-inverting OC0A sets at BOTTOM and clears on the compare match; §15.5: the pin acts on the clock TCNT0 reaches OCR0A and the flag one clock later, so OC0A is high while TCNT0 is 0 to 98, 99 timer clocks";
        "T0 fast PWM /8: period": Wave(22, 1) => 2048, "DS 15.7.3 fOC0PWM = fclk/(N*256)";
    ),
    program!("t1ovf_8", 1_837_008;
        "T1 normal /8: TOV1 ISR period": Period(5, 1) => T1OVF.period(8), T1_TOV_ISR;
    ),
    program!("t0ctc_64_p0", 34_000;
        "T0 CTC /64 OCR0A=99: toggle period": Period(22, 1) => 6400, "DS 15.7.2 fOC0 = fclk/(2N(1+OCR0A)); Fig. 15-11";
        "T0 CTC /64: first toggle (CS write ends at 8)": Edge(22, 0) => 6336, "DS Fig. 14-4 and §15.8: the TCCR0B store lands on its ending edge 8, which still counts on the old select; §17.2, prescaler 0 at the first instruction: the first /64 clock is cycle 64; §15.5 and Fig. 15-11: OC0A toggles on the clock TCNT0 reaches OCR0A = 99, 64 + 98 x 64";
    ),
    program!("t0ctc_64_p3", 34_000;
        "T0 CTC /64 OCR0A=99: toggle period": Period(22, 1) => 6400, "DS 15.7.2 fOC0 = fclk/(2N(1+OCR0A)); Fig. 15-11";
        "T0 CTC /64: first toggle (CS write ends at 11)": Edge(22, 0) => 6336, "DS Fig. 14-4 and §15.8: the TCCR0B store lands on its ending edge 11, which still counts on the old select; §17.2, prescaler 0 at the first instruction: the first /64 clock is cycle 64; §15.5 and Fig. 15-11: OC0A toggles on the clock TCNT0 reaches OCR0A = 99, 64 + 98 x 64";
    ),
    program!("t0ovf_64", 83_920;
        "T0 normal /64: TOV ISR period": Period(5, 1) => T0OVF.period(64), T0_TOV_ISR;
        "T0 normal /64: OC0A toggle (OCR0A=0xFF) to ISR edge": Lag(5, 22, 1) => T0OVF.lag(64), OC0A_TO_ISR;
    ),
    program!("t1ctc_64", 258_000;
        "T1 CTC /64 OCR1A=999: toggle period": Period(1, 1) => 64_000, "DS 16.9.2 fOC1A = fclk/(2N(1+OCR1A)); Fig. 16-11";
    ),
    program!("t0fpwm_64", 83_920;
        "T0 fast PWM /64 OCR0A=99: high time": Pulse(22, 1) => 6336, "DS Table 15-3: non-inverting OC0A sets at BOTTOM and clears on the compare match; §15.5: the pin acts on the clock TCNT0 reaches OCR0A and the flag one clock later, so OC0A is high while TCNT0 is 0 to 98, 99 timer clocks";
        "T0 fast PWM /64: period": Wave(22, 1) => 16_384, "DS 15.7.3 fOC0PWM = fclk/(N*256)";
    ),
    program!("t1ovf_64", 14_682_064;
        "T1 normal /64: TOV1 ISR period": Period(5, 1) => T1OVF.period(64), T1_TOV_ISR;
    ),
    program!("t0ctc_256_p0", 130_000;
        "T0 CTC /256 OCR0A=99: toggle period": Period(22, 1) => 25_600, "DS 15.7.2 fOC0 = fclk/(2N(1+OCR0A)); Fig. 15-11";
        "T0 CTC /256: first toggle (CS write ends at 8)": Edge(22, 0) => 25_344, "DS Fig. 14-4 and §15.8: the TCCR0B store lands on its ending edge 8, which still counts on the old select; §17.2, prescaler 0 at the first instruction: the first /256 clock is cycle 256; §15.5 and Fig. 15-11: OC0A toggles on the clock TCNT0 reaches OCR0A = 99, 256 + 98 x 256";
    ),
    program!("t0ctc_256_p3", 130_000;
        "T0 CTC /256 OCR0A=99: toggle period": Period(22, 1) => 25_600, "DS 15.7.2 fOC0 = fclk/(2N(1+OCR0A)); Fig. 15-11";
        "T0 CTC /256: first toggle (CS write ends at 11)": Edge(22, 0) => 25_344, "DS Fig. 14-4 and §15.8: the TCCR0B store lands on its ending edge 11, which still counts on the old select; §17.2, prescaler 0 at the first instruction: the first /256 clock is cycle 256; §15.5 and Fig. 15-11: OC0A toggles on the clock TCNT0 reaches OCR0A = 99, 256 + 98 x 256";
    ),
    program!("t0ovf_256", 329_680;
        "T0 normal /256: TOV ISR period": Period(5, 1) => T0OVF.period(256), T0_TOV_ISR;
        "T0 normal /256: OC0A toggle (OCR0A=0xFF) to ISR edge": Lag(5, 22, 1) => T0OVF.lag(256), OC0A_TO_ISR;
    ),
    program!("t1ctc_256", 1_026_000;
        "T1 CTC /256 OCR1A=999: toggle period": Period(1, 1) => 256_000, "DS 16.9.2 fOC1A = fclk/(2N(1+OCR1A)); Fig. 16-11";
    ),
    program!("t0fpwm_256", 329_680;
        "T0 fast PWM /256 OCR0A=99: high time": Pulse(22, 1) => 25_344, "DS Table 15-3: non-inverting OC0A sets at BOTTOM and clears on the compare match; §15.5: the pin acts on the clock TCNT0 reaches OCR0A and the flag one clock later, so OC0A is high while TCNT0 is 0 to 98, 99 timer clocks";
        "T0 fast PWM /256: period": Wave(22, 1) => 65_536, "DS 15.7.3 fOC0PWM = fclk/(N*256)";
    ),
    program!("t1ovf_256", 58_722_256;
        "T1 normal /256: TOV1 ISR period": Period(5, 1) => T1OVF.period(256), T1_TOV_ISR;
    ),
    program!("t0ctc_1024_p0", 514_000;
        "T0 CTC /1024 OCR0A=99: toggle period": Period(22, 1) => 102_400, "DS 15.7.2 fOC0 = fclk/(2N(1+OCR0A)); Fig. 15-11";
        "T0 CTC /1024: first toggle (CS write ends at 8)": Edge(22, 0) => 101_376, "DS Fig. 14-4 and §15.8: the TCCR0B store lands on its ending edge 8, which still counts on the old select; §17.2, prescaler 0 at the first instruction: the first /1024 clock is cycle 1024; §15.5 and Fig. 15-11: OC0A toggles on the clock TCNT0 reaches OCR0A = 99, 1024 + 98 x 1024";
    ),
    program!("t0ctc_1024_p3", 514_000;
        "T0 CTC /1024 OCR0A=99: toggle period": Period(22, 1) => 102_400, "DS 15.7.2 fOC0 = fclk/(2N(1+OCR0A)); Fig. 15-11";
        "T0 CTC /1024: first toggle (CS write ends at 11)": Edge(22, 0) => 101_376, "DS Fig. 14-4 and §15.8: the TCCR0B store lands on its ending edge 11, which still counts on the old select; §17.2, prescaler 0 at the first instruction: the first /1024 clock is cycle 1024; §15.5 and Fig. 15-11: OC0A toggles on the clock TCNT0 reaches OCR0A = 99, 1024 + 98 x 1024";
    ),
    program!("t0ovf_1024", 1_312_720;
        "T0 normal /1024: TOV ISR period": Period(5, 1) => T0OVF.period(1024), T0_TOV_ISR;
        "T0 normal /1024: OC0A toggle (OCR0A=0xFF) to ISR edge": Lag(5, 22, 1) => T0OVF.lag(1024), OC0A_TO_ISR;
    ),
    program!("t1ctc_1024", 4_098_000;
        "T1 CTC /1024 OCR1A=999: toggle period": Period(1, 1) => 1_024_000, "DS 16.9.2 fOC1A = fclk/(2N(1+OCR1A)); Fig. 16-11";
    ),
    program!("t0fpwm_1024", 1_312_720;
        "T0 fast PWM /1024 OCR0A=99: high time": Pulse(22, 1) => 101_376, "DS Table 15-3: non-inverting OC0A sets at BOTTOM and clears on the compare match; §15.5: the pin acts on the clock TCNT0 reaches OCR0A and the flag one clock later, so OC0A is high while TCNT0 is 0 to 98, 99 timer clocks";
        "T0 fast PWM /1024: period": Wave(22, 1) => 262_144, "DS 15.7.3 fOC0PWM = fclk/(N*256)";
    ),
    program!("t0fpwm_ocr0", 3280;
        "T0 fast PWM /1 OCR0A=0: high time": Pulse(22, 1) => 1, "DS 15.7.3: OCR0A = BOTTOM is a narrow spike each MAX+1 timer clocks; parameter 4: one timer clock";
        "T0 fast PWM /1 OCR0A=0: period": Wave(22, 1) => 256, "DS 15.7.3 fOC0PWM = fclk/(N*256)";
    ),
    program!("t0fpwm_ocr1", 3280;
        "T0 fast PWM /1 OCR0A=1: high time": Pulse(22, 1) => 1, "DS Table 15-3: non-inverting OC0A sets at BOTTOM and clears on the compare match; parameter 4: high while TCNT0 is 0, one timer clock, the same waveform as OCR0A = 0";
        "T0 fast PWM /1 OCR0A=1: period": Wave(22, 1) => 256, "DS 15.7.3 fOC0PWM = fclk/(N*256)";
    ),
    program!("t0fpwm_ocr255", 3280;
        "T0 fast PWM /1 OCR0A=0xFF: first rise at BOTTOM": Edge(22, 0) => 264, "DS Table 15-3 note 1: with OCR0A = TOP the match is ignored and OC0A sets at BOTTOM; parameter 1: the TCCR0B store ends at edge 8 with the old select, so TCNT0 is 1 at edge 9 and 0 again at edge 264";
        "T0 fast PWM /1 OCR0A=0xFF: no other edge": Count(22) => 1, "DS 15.7.3 and Table 15-3 note 1: OCR0A = MAX holds non-inverting OC0A high";
    ),
    program!("t1ovf_1024", 234_883_024;
        "T1 normal /1024: TOV1 ISR period": Period(5, 1) => T1OVF.period(1024), T1_TOV_ISR;
    ),
    program!("t0_read", 200;
        "T0 /1 started by OUT ending at edge 8; IN in cycle 13 reads TCNT0": PortB => 5, "DS Fig. 14-4: an OUT lands at the edge that ends it, so the old clock select governs that edge; ticks at edges 9..13 (DS 17.1 /1)";
    ),
    program!("t0_double_cs", 2000;
        "T0 /8 then /64, /64 store ends on /8 edge 16; IN in cycle 70 reads TCNT0": PortB => 2, "DS Fig. 14-4: an OUT lands at the edge that ends it, so the old clock select governs that edge: /8 ticks at 16; DS 17.2 free-running: /64 ticks at 64";
    ),
];
