//! The ATmega328P's SPI master, its ADC6 and ADC7, and what it says it does not model, each on
//! the documented ATmega328P netlist with a firmware written here word by word. Each firmware
//! shows what it read on PORTD, and PB0 high once it finished.

use std::fs;

use crate::runs::common::repo;
use orbe_engine::name::Ref;
use orbe_engine::timestamp::Timestamp;

const SPCR: u16 = 0x2C;
const SPSR: u16 = 0x2D;
const SPDR: u16 = 0x2E;
const DDRB: u16 = 0x04;
const PORTB: u16 = 0x05;
const DDRD: u16 = 0x0A;
const PORTD: u16 = 0x0B;
const PORTC: u16 = 0x08;
const DDRC: u16 = 0x07;
const ACSR: u16 = 0x30;
const ADCSRA: u16 = 0x007A;
const ADMUX: u16 = 0x007C;
const ADCL: u16 = 0x0078;
const ADCH: u16 = 0x0079;
const TWAR: u16 = 0x00BA;
const TWCR: u16 = 0x00BC;
const TWAMR: u16 = 0x00BD;
const TCCR0A: u16 = 0x0044;
const GPIOR0: u16 = 0x003E;
const GPIOR1: u16 = 0x004A;
const GPIOR2: u16 = 0x004B;
const DWDR: u16 = 0x0051;
const SPMCSR: u16 = 0x0057;
const RESERVED_IO: u16 = 0x0058;

const BREAK: u16 = 0x9598;
const SPM: u16 = 0x95E8;

fn jmp(word: u16) -> [u16; 2] {
    [0x940C, word]
}

fn call(word: u16) -> [u16; 2] {
    [0x940E, word]
}

fn ldi(d: u16, k: u16) -> u16 {
    0xE000 | ((k & 0xF0) << 4) | ((d - 16) << 4) | (k & 0x0F)
}

fn cpi(d: u16, k: u16) -> u16 {
    0x3000 | ((k & 0xF0) << 4) | ((d - 16) << 4) | (k & 0x0F)
}

fn out(io: u16, r: u16) -> u16 {
    0xB800 | (r << 4) | ((io & 0x30) << 5) | (io & 0x0F)
}

fn in_(r: u16, io: u16) -> u16 {
    0xB000 | (r << 4) | ((io & 0x30) << 5) | (io & 0x0F)
}

fn sts(addr: u16, r: u16) -> [u16; 2] {
    [0x9200 | (r << 4), addr]
}

fn lds(r: u16, addr: u16) -> [u16; 2] {
    [0x9000 | (r << 4), addr]
}

fn sbrs(r: u16, bit: u16) -> u16 {
    0xFE00 | (r << 4) | bit
}

fn sbrc(r: u16, bit: u16) -> u16 {
    0xFC00 | (r << 4) | bit
}

fn sbi(io: u16, bit: u16) -> u16 {
    0x9A00 | (io << 3) | bit
}

/// `rjmp` from word `from` to word `to`.
fn rjmp(from: usize, to: usize) -> u16 {
    0xC000 | ((to as i32 - from as i32 - 1) as u16 & 0x0FFF)
}

/// `brne` from word `from` to word `to`.
fn brne(from: usize, to: usize) -> u16 {
    0xF401 | ((((to as i32 - from as i32 - 1) as u16) & 0x7F) << 3)
}

const SPIN: u16 = 0xCFFF;

/// Intel HEX of `(word_addr, words)` sections.
fn hex_sections(sections: &[(u16, &[u16])]) -> String {
    let mut text = String::new();
    for &(word_addr, words) in sections {
        let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        for (k, chunk) in bytes.chunks(16).enumerate() {
            let address = word_addr * 2 + (k * 16) as u16;
            let mut record = vec![chunk.len() as u8];
            record.extend(address.to_be_bytes());
            record.push(0);
            record.extend(chunk);
            let sum = record.iter().fold(0u8, |sum, &b| sum.wrapping_add(b));
            record.push(sum.wrapping_neg());
            text.push(':');
            for byte in record {
                text.push_str(&format!("{byte:02X}"));
            }
            text.push('\n');
        }
    }
    text + ":00000001FF\n"
}

/// Intel HEX of `words` from address 0.
fn hex(words: &[u16]) -> String {
    hex_sections(&[(0, words)])
}

/// Shows r17 on PORTD, then sets PB0 and spins.
fn show_r17(words: &mut Vec<u16>) {
    words.extend([
        ldi(16, 0xFF),
        out(DDRD, 16),
        out(PORTD, 17),
        sbi(PORTB, 0),
        SPIN,
    ]);
}

/// The documented netlist, 200 us long, running `words`, with `parts` added on `nets` and the
/// chip's pins remapped by `pins`.
fn netlist(
    words: &[u16],
    nets: &[&str],
    parts: serde_json::Value,
    pins: &[(&str, &str)],
) -> serde_json::Value {
    let doc = fs::read_to_string(repo("docs/engine/netlist.md")).unwrap();
    let section = &doc[doc.find("### ATmega328P").unwrap()..];
    let start = section.find("```json\n").unwrap() + "```json\n".len();
    let end = start + section[start..].find("\n```").unwrap();
    let mut netlist: serde_json::Value = serde_json::from_str(&section[start..end]).unwrap();
    netlist["analysis"]["transient"] = serde_json::json!({
        "stop_ps": 200_000_000u64, "max_step_ps": 10_000_000u64, "grid_ps": 10_000_000u64
    });
    for net in nets {
        netlist["nets"].as_array_mut().unwrap().push((*net).into());
    }
    netlist["parts"]
        .as_array_mut()
        .unwrap()
        .extend(parts.as_array().unwrap().iter().cloned());
    let chip = netlist["parts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|part| part["kind"] == "atmega328p")
        .unwrap();
    chip["firmware"] = hex(words).into();
    for (pin, net) in pins {
        chip["pins"][*pin] = (*net).into();
    }
    netlist
}

fn resistor(name: &str, a: &str, b: &str, ohms: f64) -> serde_json::Value {
    serde_json::json!({
        "name": name, "kind": "resistor", "pins": { "a": a, "b": b },
        "resistance": { "value": ohms, "unit": "ohm" },
        "ratings": { "power": null }
    })
}

struct Ran {
    done: orbe_engine::Completed,
}

impl Ran {
    fn new(netlist: &serde_json::Value) -> Ran {
        Ran {
            done: orbe_engine::run(
                netlist.to_string().as_bytes(),
                Timestamp::from_unix_seconds(0),
            ),
        }
    }

    /// Each diagnostic as `code: message`.
    fn diagnostics(&self) -> Vec<String> {
        self.done
            .results
            .diagnostics()
            .iter()
            .map(|d| {
                let value = serde_json::to_value(d).unwrap();
                format!(
                    "{}: {}",
                    value["code"].as_str().unwrap(),
                    value["message"].as_str().unwrap()
                )
            })
            .collect()
    }

    /// Whether `net` is above 2.5 V at the last row.
    fn high(&self, net: &str) -> bool {
        let trace = self.done.results.trace().unwrap();
        let reference: Ref = format!("#net.{net}").parse().unwrap();
        let column = trace
            .columns()
            .iter()
            .position(|r| *r == reference)
            .unwrap();
        *trace.column(column).last().unwrap() > 2.5
    }

    /// PORTD's eight levels as a byte, once PB0 says the firmware finished.
    fn portd(&self) -> u8 {
        assert!(
            self.high("pb0"),
            "the firmware finished: {:?}",
            self.diagnostics()
        );
        (0..8).fold(0, |byte, bit| {
            byte | (u8::from(self.high(&format!("pd{bit}"))) << bit)
        })
    }
}

/// SS, MOSI and SCK out, SPI on as master at fosc/16, then one transfer of `byte` into r17.
fn spi_master(byte: u8) -> Vec<u16> {
    let mut words = vec![
        ldi(16, 0b0010_1101),
        out(DDRB, 16),
        ldi(16, 0x51),
        out(SPCR, 16),
        ldi(16, u16::from(byte)),
        out(SPDR, 16),
    ];
    let wait = words.len();
    words.extend([
        in_(18, SPSR),
        sbrs(18, 7),
        rjmp(wait + 2, wait),
        in_(17, SPDR),
    ]);
    words
}

#[test]
fn spi_with_miso_to_ground_through_100_ohm_reads_0x00() {
    let mut words = spi_master(0xA5);
    show_r17(&mut words);
    let ran = Ran::new(&netlist(
        &words,
        &[],
        serde_json::json!([resistor("rmiso", "pb4", "gnd", 100.0)]),
        &[],
    ));
    assert_eq!(ran.portd(), 0x00);
    assert_eq!(ran.diagnostics(), Vec::<String>::new());
}

#[test]
fn spi_with_miso_to_vcc_through_100_ohm_reads_0xff() {
    let mut words = spi_master(0xA5);
    show_r17(&mut words);
    let ran = Ran::new(&netlist(
        &words,
        &[],
        serde_json::json!([resistor("rmiso", "pb4", "vcc", 100.0)]),
        &[],
    ));
    assert_eq!(ran.portd(), 0xFF);
    assert_eq!(ran.diagnostics(), Vec::<String>::new());
}

#[test]
fn spi_with_mosi_wired_to_miso_reads_0x55_on_every_byte() {
    // Three transfers of 0x55. r17 shows the first, and PB0 rises only when all three read
    // 0x55.
    let mut words = spi_master(0x55);
    for reg in [19, 20] {
        words.push(out(SPDR, 16));
        let wait = words.len();
        words.extend([
            in_(18, SPSR),
            sbrs(18, 7),
            rjmp(wait + 2, wait),
            in_(reg, SPDR),
        ]);
    }
    words.extend([ldi(16, 0xFF), out(DDRD, 16), out(PORTD, 17)]);
    let fail = words.len() + 8;
    for reg in [17, 19, 20] {
        let at = words.len() + 1;
        words.extend([cpi(reg, 0x55), brne(at, fail)]);
    }
    words.extend([sbi(PORTB, 0), SPIN]);
    assert_eq!(words.len(), fail);
    words.push(SPIN);
    let mut netlist = netlist(&words, &[], serde_json::json!([]), &[("PB4", "pb3")]);
    netlist["nets"]
        .as_array_mut()
        .unwrap()
        .retain(|net| net != "pb4");
    let ran = Ran::new(&netlist);
    assert_eq!(ran.portd(), 0x55);
    assert_eq!(ran.diagnostics(), Vec::<String>::new());
}

#[test]
fn the_comparator_on_a_divider_is_refused_at_its_first_read_of_acsr() {
    // AIN0 (PD6) at 1.906 V and AIN1 (PD7) at 0.976 V, so ACO is 1 on the part (§23.1). The
    // model has their logic levels, not their voltages, so it refuses the read.
    let words = vec![
        ldi(16, 0x20),
        out(DDRB, 16),
        in_(17, ACSR),
        sbrs(17, 5),
        rjmp(4, 2),
        sbi(PORTB, 5),
        SPIN,
    ];
    let parts = serde_json::json!([
        resistor("ra1", "vcc", "pd6", 16_230.0),
        resistor("ra2", "pd6", "gnd", 10_000.0),
        resistor("rb1", "vcc", "pd7", 41_230.0),
        resistor("rb2", "pd7", "gnd", 10_000.0),
    ]);
    let ran = Ran::new(&netlist(&words, &[], parts, &[]));
    // The load is the instruction at cycle 2. PB5's drive changed on that cycle too, and that
    // stop came first, so the refusal's stop is the next cycle, 187500 ps. PB5 never rises.
    assert_eq!(
        ran.diagnostics(),
        [
            "chip.unsupported_setting: #u1 read the AC output while the comparator is on at 0x00000004, which the model works out from the logic level on each input pin, not its voltage, at 187500 ps"
        ]
    );
    assert!(!ran.high("pb5"));
}

#[test]
fn twi_slave_via_twar_or_twamr_with_twea_refuses_and_master_stays_quiet() {
    // `Wire.begin(0x42)`: TWAR = 0x84, then TWCR = TWEN | TWIE | TWEA. The refusal comes at the
    // TWCR store, word 4 (byte 8), cycle 4 (reported at cycle 5 after the first step at cycle 4).
    let mut twar = vec![ldi(16, 0x84)];
    twar.extend(sts(TWAR, 16));
    twar.push(ldi(16, 0x45));
    twar.extend(sts(TWCR, 16));
    twar.extend([ldi(17, 0x42), ldi(16, 0x01), out(DDRB, 16)]);
    show_r17(&mut twar);
    let ran = Ran::new(&netlist(&twar, &[], serde_json::json!([]), &[]));
    assert!(!ran.high("pb0"), "PB0 never rises past the refusal");
    assert_eq!(
        ran.diagnostics(),
        [
            "chip.unsupported_setting: #u1 set TWI slave at 0x00000008, which the model does not run, at 312500 ps"
        ]
    );

    // Nonzero `TWAMR` mask with `TWEN | TWEA` also needs slave mode, even with `TWAR` untouched.
    let mut twamr = vec![ldi(16, 0x0E)];
    twamr.extend(sts(TWAMR, 16));
    twamr.push(ldi(16, 0x45));
    twamr.extend(sts(TWCR, 16));
    twamr.extend([ldi(17, 0x42), ldi(16, 0x01), out(DDRB, 16)]);
    show_r17(&mut twamr);
    let ran = Ran::new(&netlist(&twamr, &[], serde_json::json!([]), &[]));
    assert!(!ran.high("pb0"), "PB0 never rises past the refusal");
    assert_eq!(
        ran.diagnostics(),
        [
            "chip.unsupported_setting: #u1 set TWI slave at 0x00000008, which the model does not run, at 312500 ps"
        ]
    );

    // Stock `Wire.begin()` (master mode): sets `TWCR = TWEN | TWIE | TWEA` without touching
    // `TWAR` or `TWAMR`, so it stays quiet.
    let mut master = vec![ldi(16, 0x45)];
    master.extend(sts(TWCR, 16));
    master.extend([ldi(17, 0x55), ldi(16, 0x01), out(DDRB, 16)]);
    show_r17(&mut master);
    let ran = Ran::new(&netlist(&master, &[], serde_json::json!([]), &[]));
    assert_eq!(ran.portd(), 0x55);
    assert_eq!(ran.diagnostics(), Vec::<String>::new());

    // Reserved bits in `TCCR0A` (bits 3:2, §15.9.1) are ignored on write and read as 0.
    let mut reserved = vec![ldi(16, 0x0C)];
    reserved.extend(sts(TCCR0A, 16));
    reserved.extend(lds(17, TCCR0A));
    reserved.extend([ldi(16, 0x01), out(DDRB, 16)]);
    show_r17(&mut reserved);
    let ran = Ran::new(&netlist(&reserved, &[], serde_json::json!([]), &[]));
    assert_eq!(ran.portd(), 0x00);
    assert_eq!(ran.diagnostics(), Vec::<String>::new());
}

#[test]
fn reserved_io_addresses_read_zero_and_gpior_stores() {
    // DS40002061B §36, §25.4.1, §27.9.1: reserved I/O addresses (0x0058), `DWDR` with `DWEN`
    // unprogrammed (0x0051), and read-only `SPMCSR.RWWSB` (0x0057) are not RAM and read as 0.
    for (addr, value) in [(RESERVED_IO, 0xA5), (DWDR, 0xA5), (SPMCSR, 0x40)] {
        let mut words = vec![ldi(16, value)];
        words.extend(sts(addr, 16));
        words.extend(lds(17, addr));
        words.extend([ldi(16, 0x01), out(DDRB, 16)]);
        show_r17(&mut words);
        let ran = Ran::new(&netlist(&words, &[], serde_json::json!([]), &[]));
        assert_eq!(ran.portd(), 0x00, "addr {addr:#06x}");
        assert_eq!(ran.diagnostics(), Vec::<String>::new(), "addr {addr:#06x}");
    }
    // DS40002061B §7.5: `GPIOR0`, `GPIOR1` and `GPIOR2` are general-purpose R/W registers.
    for (addr, value) in [(GPIOR0, 0xA5), (GPIOR1, 0x5A), (GPIOR2, 0x3C)] {
        let mut words = vec![ldi(16, value)];
        words.extend(sts(addr, 16));
        words.extend(lds(17, addr));
        words.extend([ldi(16, 0x01), out(DDRB, 16)]);
        show_r17(&mut words);
        let ran = Ran::new(&netlist(&words, &[], serde_json::json!([]), &[]));
        assert_eq!(ran.portd(), value as u8, "GPIOR {addr:#06x}");
        assert_eq!(ran.diagnostics(), Vec::<String>::new(), "GPIOR {addr:#06x}");
    }
}

#[test]
fn break_runs_as_a_nop_when_dwen_is_unprogrammed() {
    // DS40002198B `BREAK`, DS40002061B Table 28-8: with `DWEN` unprogrammed, `BREAK` acts as a
    // one-cycle `NOP` instead of halting the CPU.
    let mut words = vec![BREAK, ldi(17, 0x5A), ldi(16, 0x01), out(DDRB, 16)];
    show_r17(&mut words);
    let ran = Ran::new(&netlist(&words, &[], serde_json::json!([]), &[]));
    assert_eq!(ran.portd(), 0x5A);
    assert_eq!(ran.diagnostics(), Vec::<String>::new());
}

#[test]
fn spm_in_the_application_section_is_a_nop_and_in_the_boot_section_refuses() {
    // DS40002061B §27.2, §27.6: `SPM` in the Application Flash section (`pc < boot_word`) is
    // disabled by hardware and has no effect.
    let mut app = vec![SPM, ldi(17, 0x33), ldi(16, 0x01), out(DDRB, 16)];
    show_r17(&mut app);
    let ran = Ran::new(&netlist(&app, &[], serde_json::json!([]), &[]));
    assert_eq!(ran.portd(), 0x33);
    assert_eq!(ran.diagnostics(), Vec::<String>::new());

    // In the Boot Loader Flash section (`pc >= 0x3800` for high fuse 0xD9, Table 27-13), `SPM`
    // would program flash, so the chip refuses it.
    let boot_word = 0x3800u16;
    let boot = [SPM, sbi(DDRB, 0), sbi(PORTB, 0), SPIN];
    let mut nl = netlist(&[], &[], serde_json::json!([]), &[]);
    let chip = nl["parts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|part| part["kind"] == "atmega328p")
        .unwrap();
    chip["firmware"] = hex_sections(&[(0, &jmp(boot_word)), (boot_word, &boot)]).into();
    let ran = Ran::new(&nl);
    assert!(!ran.high("pb0"), "PB0 never rises past the refused SPM");
    assert_eq!(
        ran.diagnostics(),
        [
            "chip.unsupported_instruction: #u1 reached `spm` at 0x00007000, which the model does not run, at 187500 ps"
        ]
    );
}

#[test]
fn calling_optiboot_do_spm_refuses_with_optiboot_unsimulated() {
    // Optiboot's `do_spm` entry sits in the boot section (byte 0x7FFC, word 0x3FFE). The engine
    // does not load Optiboot's code into flash, so calling into `0x3F00..=0x3FFF` refuses loudly
    // rather than executing NOPs through the end of flash.
    let mut words = Vec::new();
    words.extend(call(0x3FFE));
    words.extend([sbi(DDRB, 0), sbi(PORTB, 0), SPIN]);
    let mut nl = netlist(&words, &[], serde_json::json!([]), &[]);
    let chip = nl["parts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|part| part["kind"] == "atmega328p")
        .unwrap();
    chip["bootloader"] = "optiboot".into();
    chip["fuses"]["high"] = 0xDE.into();
    let ran = Ran::new(&nl);
    assert!(!ran.high("pb0"), "PB0 never rises past the refused call");
    assert_eq!(
        ran.diagnostics(),
        [
            "chip.unsupported_setting: #u1 used Optiboot, its boot-section code (such as do_spm), at 0x00007ffc, which the model does not simulate, at 312500 ps"
        ]
    );
}

/// The documented netlist running one conversion of the channel and reference `admux` selects,
/// with `parts` added on `nets` and the chip's pins remapped by `pins`. ADCSRA: on, start,
/// clk/128. The code shows on PC1:0 and PORTD. The first conversion takes 25 ADC clocks of 128
/// cycles, 200 us (§24.4), so the run is 400 us.
fn conversion(
    admux: u16,
    nets: &[&str],
    parts: serde_json::Value,
    pins: &[(&str, &str)],
) -> serde_json::Value {
    let mut words = vec![ldi(16, admux)];
    words.extend(sts(ADMUX, 16));
    words.push(ldi(16, 0xC7));
    words.extend(sts(ADCSRA, 16));
    let wait = words.len();
    words.extend(lds(16, ADCSRA));
    words.extend([sbrc(16, 6), rjmp(wait + 3, wait)]);
    words.extend(lds(17, ADCL));
    words.extend(lds(18, ADCH));
    words.extend([
        ldi(16, 0x03),
        out(DDRC, 16),
        out(PORTC, 18),
        ldi(16, 0x01),
        out(DDRB, 16),
    ]);
    show_r17(&mut words);
    let mut netlist = netlist(&words, nets, parts, pins);
    netlist["analysis"]["transient"]["stop_ps"] = 400_000_000u64.into();
    netlist
}

/// The code a run of [`conversion`] showed, and the run's diagnostics.
fn converted(netlist: &serde_json::Value) -> (u16, Vec<String>) {
    let ran = Ran::new(netlist);
    let high = u16::from(ran.high("pc0")) | (u16::from(ran.high("pc1")) << 1);
    ((high << 8) | u16::from(ran.portd()), ran.diagnostics())
}

/// ADC6 and ADC7 read the `ADC6` and `ADC7` pins, which the TQFP has, and 0 V on a part that
/// leaves them out, as the DIP does. Neither is a note. A divider from VCC puts 2/3 of AVCC on
/// `a6` and 1/3 on `a7`, codes 682 and 341 (§24.7).
#[test]
fn adc6_and_adc7_read_their_pins_and_0_v_without_them() {
    let dividers = serde_json::json!([
        resistor("r6a", "vcc", "a6", 10e3),
        resistor("r6b", "a6", "gnd", 20e3),
        resistor("r7a", "vcc", "a7", 20e3),
        resistor("r7b", "a7", "gnd", 10e3),
    ]);
    let tqfp = |admux| {
        converted(&conversion(
            admux,
            &["a6", "a7"],
            dividers.clone(),
            &[("ADC6", "a6"), ("ADC7", "a7")],
        ))
    };
    let dip = |admux| converted(&conversion(admux, &[], serde_json::json!([]), &[]));
    // ADMUX: AVCC as the reference and ADC6 or ADC7.
    assert_eq!(tqfp(0x46), (682, Vec::new()));
    assert_eq!(tqfp(0x47), (341, Vec::new()));
    assert_eq!(dip(0x46), (0, Vec::new()));
    assert_eq!(dip(0x47), (0, Vec::new()));
}

#[test]
fn the_temperature_sensor_reads_the_world_at_60_degrees() {
    // ADMUX: the 1.1 V reference and ADC8. Table 24-2's typical values put 60 degC at about
    // 352 mV, ADC 328.
    let read = |celsius: Option<f64>| {
        let mut netlist = conversion(0xC8, &[], serde_json::json!([]), &[]);
        if let Some(celsius) = celsius {
            netlist["world"] = serde_json::json!({
                "temperature": { "value": celsius, "unit": "degC" }
            });
        }
        let (code, diagnostics) = converted(&netlist);
        assert_eq!(diagnostics, Vec::<String>::new());
        code
    };
    let hot = read(Some(60.0));
    assert!((326..=330).contains(&hot), "{hot}");
    let room = read(None);
    assert!((290..=294).contains(&room), "25 degC with no world: {room}");
}
