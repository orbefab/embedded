//! Cycle count for every mnemonic in the instruction catalog.
//!
//! Counts are the AVRe column of DS40002198C. That column is also AVRe+
//! (manual §5: the tables do not distinguish AVRe and AVRe+). Internal RAM is
//! note 1 on page 23.

use crate::common;
use common::{brbc, brbs};

use orbe_avr::Cpu;

#[derive(Clone, Copy)]
enum Words {
    One(u16),
    Two(u16, u16),
    Three(u16, u16, u16),
}

struct Row {
    name: &'static str,
    words: Words,
    sreg: u8,
    r0: u8,
    r1: u8,
    /// Data address `0x20`, the I/O register `SBIC`/`SBIS` address 0.
    io0: u8,
    cycles: u64,
}

impl Row {
    const fn new(name: &'static str, words: Words, cycles: u64) -> Self {
        Self {
            name,
            words,
            sreg: 0,
            r0: 0,
            r1: 0,
            io0: 0,
            cycles,
        }
    }

    const fn r0(mut self, r0: u8) -> Self {
        self.r0 = r0;
        self
    }

    const fn r1(mut self, r1: u8) -> Self {
        self.r1 = r1;
        self
    }

    const fn io0(mut self, io0: u8) -> Self {
        self.io0 = io0;
        self
    }
}

struct Branch {
    name: &'static str,
    word: u16,
    taken: u8,
    idle: u8,
}

// DS40002198C Table 5-3, pages 19–20. Taken is 2, not taken is 1.
const BRANCHES: &[Branch] = &[
    Branch {
        name: "BRBS",
        word: brbs(1, 1),
        taken: 0x02,
        idle: 0,
    },
    Branch {
        name: "BRBC",
        word: brbc(1, 1),
        taken: 0,
        idle: 0x02,
    },
    Branch {
        name: "BREQ",
        word: brbs(1, 1),
        taken: 0x02,
        idle: 0,
    },
    Branch {
        name: "BRNE",
        word: brbc(1, 1),
        taken: 0,
        idle: 0x02,
    },
    Branch {
        name: "BRCS",
        word: brbs(0, 1),
        taken: 0x01,
        idle: 0,
    },
    Branch {
        name: "BRCC",
        word: brbc(0, 1),
        taken: 0,
        idle: 0x01,
    },
    Branch {
        name: "BRSH",
        word: brbc(0, 1),
        taken: 0,
        idle: 0x01,
    },
    Branch {
        name: "BRLO",
        word: brbs(0, 1),
        taken: 0x01,
        idle: 0,
    },
    Branch {
        name: "BRMI",
        word: brbs(2, 1),
        taken: 0x04,
        idle: 0,
    },
    Branch {
        name: "BRPL",
        word: brbc(2, 1),
        taken: 0,
        idle: 0x04,
    },
    Branch {
        name: "BRGE",
        word: brbc(4, 1),
        taken: 0,
        idle: 0x10,
    },
    Branch {
        name: "BRLT",
        word: brbs(4, 1),
        taken: 0x10,
        idle: 0,
    },
    Branch {
        name: "BRHS",
        word: brbs(5, 1),
        taken: 0x20,
        idle: 0,
    },
    Branch {
        name: "BRHC",
        word: brbc(5, 1),
        taken: 0,
        idle: 0x20,
    },
    Branch {
        name: "BRTS",
        word: brbs(6, 1),
        taken: 0x40,
        idle: 0,
    },
    Branch {
        name: "BRTC",
        word: brbc(6, 1),
        taken: 0,
        idle: 0x40,
    },
    Branch {
        name: "BRVS",
        word: brbs(3, 1),
        taken: 0x08,
        idle: 0,
    },
    Branch {
        name: "BRVC",
        word: brbc(3, 1),
        taken: 0,
        idle: 0x08,
    },
    Branch {
        name: "BRIE",
        word: brbs(7, 1),
        taken: 0x80,
        idle: 0,
    },
    Branch {
        name: "BRID",
        word: brbc(7, 1),
        taken: 0,
        idle: 0x80,
    },
];

// DS40002198C Table 5-2, pages 18–19, unless a row says otherwise.
const ROWS: &[Row] = &[
    Row::new("ADD", Words::One(0x0C00), 1),
    Row::new("ADC", Words::One(0x1C00), 1),
    Row::new("ADIW", Words::One(0x9600), 2),
    Row::new("SUB", Words::One(0x1800), 1),
    Row::new("SUBI", Words::One(0x5000), 1),
    Row::new("SBC", Words::One(0x0800), 1),
    Row::new("SBCI", Words::One(0x4000), 1),
    Row::new("SBIW", Words::One(0x9700), 2),
    Row::new("AND", Words::One(0x2000), 1),
    Row::new("ANDI", Words::One(0x7000), 1),
    Row::new("OR", Words::One(0x2800), 1),
    Row::new("ORI", Words::One(0x6000), 1),
    Row::new("EOR", Words::One(0x2400), 1),
    Row::new("COM", Words::One(0x9400), 1),
    Row::new("NEG", Words::One(0x9401), 1),
    Row::new("SBR", Words::One(0x6000), 1),
    Row::new("CBR", Words::One(0x7000), 1),
    Row::new("INC", Words::One(0x9403), 1),
    Row::new("DEC", Words::One(0x940A), 1),
    Row::new("TST", Words::One(0x2000), 1),
    Row::new("CLR", Words::One(0x2400), 1),
    Row::new("SER", Words::One(0xEF0F), 1),
    Row::new("MUL", Words::One(0x9C00), 2),
    Row::new("MULS", Words::One(0x0200), 2),
    Row::new("MULSU", Words::One(0x0300), 2),
    Row::new("FMUL", Words::One(0x0308), 2),
    Row::new("FMULS", Words::One(0x0380), 2),
    Row::new("FMULSU", Words::One(0x0388), 2),
    // Table 5-3, pages 19–20. Call/return on a 16-bit PC, pages 50, 100–102.
    Row::new("RJMP", Words::One(0xC000), 2),
    Row::new("IJMP", Words::One(0x9409), 2),
    Row::new("JMP", Words::Two(0x940C, 0x0004), 3),
    Row::new("RCALL", Words::One(0xD000), 3),
    Row::new("ICALL", Words::One(0x9509), 3),
    Row::new("CALL", Words::Two(0x940E, 0x0000), 4),
    Row::new("RET", Words::One(0x9508), 4),
    Row::new("RETI", Words::One(0x9518), 4),
    // CPSE / SBRC / SBRS / SBIC / SBIS: Table 5-3 and Table 6-48, pages 19 and 62–63.
    // 1 if no skip, 2 over one word, 3 over two words.
    Row::new("CPSE no skip", Words::One(0x1001), 1).r1(1),
    Row::new("CPSE skip one word", Words::Two(0x1000, 0x0000), 2),
    Row::new("CPSE skip JMP", Words::Three(0x1000, 0x940C, 0x0000), 3),
    Row::new("CP", Words::One(0x1400), 1),
    Row::new("CPC", Words::One(0x0400), 1),
    Row::new("CPI", Words::One(0x3000), 1),
    Row::new("SBRC no skip", Words::One(0xFC00), 1).r0(1),
    Row::new("SBRC skip one word", Words::Two(0xFC00, 0x0000), 2),
    Row::new("SBRC skip STS", Words::Three(0xFC00, 0x9200, 0x0100), 3),
    Row::new("SBRS no skip", Words::One(0xFE00), 1),
    Row::new("SBRS skip one word", Words::Two(0xFE00, 0x0000), 2).r0(1),
    Row::new("SBRS skip LDS", Words::Three(0xFE00, 0x9000, 0x0100), 3).r0(1),
    Row::new("SBIC no skip", Words::One(0x9900), 1).io0(1),
    Row::new("SBIC skip one word", Words::Two(0x9900, 0x0000), 2),
    Row::new("SBIC skip CALL", Words::Three(0x9900, 0x940E, 0x0000), 3),
    Row::new("SBIS no skip", Words::One(0x9B00), 1),
    Row::new("SBIS skip one word", Words::Two(0x9B00, 0x0000), 2).io0(1),
    Row::new("SBIS skip JMP", Words::Three(0x9B00, 0x940C, 0x0000), 3).io0(1),
    // Table 5-5, page 22.
    Row::new("SBI", Words::One(0x9A00), 2),
    Row::new("CBI", Words::One(0x9800), 2),
    Row::new("LSL", Words::One(0x0C00), 1),
    Row::new("LSR", Words::One(0x9406), 1),
    Row::new("ROL", Words::One(0x1C00), 1),
    Row::new("ROR", Words::One(0x9407), 1),
    Row::new("ASR", Words::One(0x9405), 1),
    Row::new("SWAP", Words::One(0x9402), 1),
    Row::new("BSET", Words::One(0x9408), 1),
    Row::new("BCLR", Words::One(0x9488), 1),
    Row::new("BST", Words::One(0xFA00), 1),
    Row::new("BLD", Words::One(0xF800), 1),
    Row::new("SEC", Words::One(0x9408), 1),
    Row::new("CLC", Words::One(0x9488), 1),
    Row::new("SEN", Words::One(0x9428), 1),
    Row::new("CLN", Words::One(0x94A8), 1),
    Row::new("SEZ", Words::One(0x9418), 1),
    Row::new("CLZ", Words::One(0x9498), 1),
    Row::new("SEI", Words::One(0x9478), 1),
    Row::new("CLI", Words::One(0x94F8), 1),
    Row::new("SES", Words::One(0x9448), 1),
    Row::new("CLS", Words::One(0x94C8), 1),
    Row::new("SEV", Words::One(0x9438), 1),
    Row::new("CLV", Words::One(0x94B8), 1),
    Row::new("SET", Words::One(0x9468), 1),
    Row::new("CLT", Words::One(0x94E8), 1),
    Row::new("SEH", Words::One(0x9458), 1),
    Row::new("CLH", Words::One(0x94D8), 1),
    // Table 5-4, pages 20–21. LD/ST/LDS/STS are 2 on internal RAM. LPM is 3.
    // PUSH and POP are 2.
    Row::new("MOV", Words::One(0x2C00), 1),
    Row::new("MOVW", Words::One(0x0100), 1),
    Row::new("LDI", Words::One(0xE000), 1),
    Row::new("LD X", Words::One(0x900C), 2),
    Row::new("LD X+", Words::One(0x900D), 2),
    Row::new("LD -X", Words::One(0x900E), 2),
    Row::new("LD Y", Words::One(0x8008), 2),
    Row::new("LD Y+", Words::One(0x9009), 2),
    Row::new("LD -Y", Words::One(0x900A), 2),
    Row::new("LDD Y", Words::One(0x8009), 2),
    Row::new("LD Z", Words::One(0x8000), 2),
    Row::new("LD Z+", Words::One(0x9001), 2),
    Row::new("LD -Z", Words::One(0x9002), 2),
    Row::new("LDD Z", Words::One(0x8001), 2),
    Row::new("LDS", Words::Two(0x9000, 0x0100), 2),
    Row::new("ST X", Words::One(0x920C), 2),
    Row::new("ST X+", Words::One(0x920D), 2),
    Row::new("ST -X", Words::One(0x920E), 2),
    Row::new("ST Y", Words::One(0x8208), 2),
    Row::new("ST Y+", Words::One(0x9209), 2),
    Row::new("ST -Y", Words::One(0x920A), 2),
    Row::new("STD Y", Words::One(0x8209), 2),
    Row::new("ST Z", Words::One(0x8200), 2),
    Row::new("ST Z+", Words::One(0x9201), 2),
    Row::new("ST -Z", Words::One(0x9202), 2),
    Row::new("STD Z", Words::One(0x8201), 2),
    Row::new("STS", Words::Two(0x9200, 0x0100), 2),
    Row::new("LPM", Words::One(0x95C8), 3),
    Row::new("LPM Z", Words::One(0x9004), 3),
    Row::new("LPM Z+", Words::One(0x9005), 3),
    // Table 6-112, page 121: AVRe cycles vary with programming time. Flash is
    // not programmed, so the logged SPM retires in one cycle.
    Row::new("SPM", Words::One(0x95E8), 1),
    Row::new("IN", Words::One(0xB000), 1),
    Row::new("OUT", Words::One(0xB800), 1),
    Row::new("PUSH", Words::One(0x920F), 2),
    Row::new("POP", Words::One(0x900F), 2),
    // Table 5-6, pages 22–23. BREAK is 1 in the AVRe column.
    Row::new("NOP", Words::One(0x0000), 1),
    Row::new("SLEEP", Words::One(0x9588), 1),
    Row::new("WDR", Words::One(0x95A8), 1),
    Row::new("BREAK", Words::One(0x9598), 1),
];

fn run(mut cpu: Cpu, words: Words, sreg: u8, r0: u8, r1: u8, io0: u8) -> Cpu {
    match words {
        Words::One(a) => cpu.load_words(&[a]),
        Words::Two(a, b) => cpu.load_words(&[a, b]),
        Words::Three(a, b, c) => cpu.load_words(&[a, b, c]),
    }
    cpu.set_sreg(sreg);
    cpu.set_reg(0, r0);
    cpu.set_reg(1, r1);
    cpu.poke(0x0020, io0);
    cpu.step();
    cpu
}

#[test]
fn cycle_count_for_every_mnemonic() {
    for row in ROWS {
        let cpu = run(Cpu::new(), row.words, row.sreg, row.r0, row.r1, row.io0);
        assert_eq!(cpu.cycles(), row.cycles, "{}", row.name);
    }
    for branch in BRANCHES {
        let taken = run(Cpu::new(), Words::One(branch.word), branch.taken, 0, 0, 0);
        assert_eq!(taken.cycles(), 2, "{} taken", branch.name);
        let idle = run(Cpu::new(), Words::One(branch.word), branch.idle, 0, 0, 0);
        assert_eq!(idle.cycles(), 1, "{} not taken", branch.name);
    }
}
