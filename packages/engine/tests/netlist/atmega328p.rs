use orbe_engine::mcu::Atmega328p;
use orbe_engine::netlist::{self, Chip, Model, Netlist};

use orbe_engine::diagnostic::Diagnostic;

const PART: &str = include_str!("fixtures/atmega328p.json");

fn chip(edit: impl FnOnce(&mut serde_json::Value)) -> Result<Netlist, Vec<Diagnostic>> {
    let mut part: serde_json::Value = serde_json::from_str(PART).unwrap();
    edit(&mut part);
    let mut nets: Vec<&str> = part["pins"]
        .as_object()
        .into_iter()
        .flat_map(|pins| pins.values())
        .filter_map(serde_json::Value::as_str)
        .chain(["gnd", "agnd"])
        .collect();
    nets.sort();
    nets.dedup();
    let document = serde_json::json!({
        "version": 1,
        "seed": 0,
        "analysis": { "op": {} },
        "ground": "gnd",
        "nets": nets,
        "parts": [part]
    });
    netlist::parse(&serde_json::to_vec(&document).unwrap())
}

fn values(netlist: &Netlist) -> Atmega328p {
    let (_, part) = netlist.parts().next().unwrap();
    match &part.model {
        Model::Chip(Chip::Atmega328p(chip)) => (**chip).clone(),
        other => panic!("{other:?}"),
    }
}

fn errors(edit: impl FnOnce(&mut serde_json::Value)) -> Vec<String> {
    chip(edit)
        .unwrap_err()
        .iter()
        .map(|d| serde_json::to_value(d).unwrap())
        .map(|d| format!("{} {} {}", d["code"], d["refs"], d["data"]))
        .collect()
}

fn message(edit: impl FnOnce(&mut serde_json::Value)) -> String {
    let [diagnostic] = &chip(edit).unwrap_err()[..] else {
        panic!("one diagnostic");
    };
    serde_json::to_value(diagnostic).unwrap()["message"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn an_atmega328p_parses_with_its_pins_in_dip_order_and_its_flash_from_the_hex() {
    let netlist = chip(|_| {}).unwrap();
    let chip = values(&netlist);
    let nets: Vec<_> = chip
        .pins
        .iter()
        .map(|&id| netlist.net_name(id).as_str())
        .collect();
    assert_eq!(
        nets[..10],
        [
            "pc6", "pd0", "pd1", "pd2", "pd3", "pd4", "vcc", "gnd", "xtal1", "xtal2"
        ]
    );
    assert_eq!(nets[21], "gnd");
    assert_eq!(nets[27], "pc5");
    assert_eq!(
        chip.firmware.flash(),
        [
            0x25, 0x9A, 0x1D, 0x9A, 0x0F, 0xE1, 0x0A, 0x95, 0xF1, 0xF7, 0, 0, 0, 0, 0, 0, 0xF8,
            0xCF
        ]
    );
    assert_eq!(chip.clock_hz.value, 16_000_000);
    assert_eq!(
        (chip.fuses.low(), chip.fuses.high(), chip.fuses.extended()),
        (0xFF, 0xD9, 0xFD)
    );
    assert!(chip.fuses.has_reset());
    assert_eq!(chip.leakage_resistance.si.get(), 5.5e6);
    assert_eq!(chip.input_threshold.high.si.get(), 0.52);
    let (_, part) = netlist.parts().next().unwrap();
    assert!(part.model.ratings().is_empty());
}

#[test]
fn a_gap_in_the_hex_is_erased_flash() {
    let hex = ":020004000C945A\n:00000001FF";
    let netlist = chip(|part| part["firmware"] = hex.into()).unwrap();
    assert_eq!(
        values(&netlist).firmware.flash(),
        [0xFF, 0xFF, 0xFF, 0xFF, 0x0C, 0x94]
    );
}

#[test]
fn an_end_record_s_address_is_ignored() {
    let netlist = chip(|part| part["firmware"] = ":0100000000FF\n:00000101FE".into()).unwrap();
    assert_eq!(values(&netlist).firmware.flash(), [0x00]);
}

#[test]
fn each_firmware_fault_names_its_line_and_rule() {
    let cases = [
        (
            ":10000000259A1D9A0FE10A95F1F700000000000003\n:02001000F8CF28\n:00000001FF",
            "2",
            "checksum",
        ),
        ("259A\n:00000001FF", "1", "record"),
        (":0200000025XA00\n:00000001FF", "1", "record"),
        (":0300000025\n:00000001FF", "1", "record"),
        (":00000006FA\n:00000001FF", "1", "record_type"),
        (":00000001FF\n:00000001FF", "2", "after_end"),
        (":0100000100FE\n:00000001FF", "1", "record"),
        (
            ":0400000001020304F2\n:020003000506F0\n:00000001FF",
            "2",
            "overlap",
        ),
        (
            ":020000020800F4\n:0100000000FF\n:00000001FF",
            "2",
            "past_flash",
        ),
        (":027FFF00FFFF82\n:00000001FF", "1", "past_flash"),
        (":02001000F8CF27\n", "null", "no_end"),
        ("", "null", "no_end"),
        (":00000001FF\n", "null", "empty"),
        (":020000040000FA\n:00000001FF", "null", "empty"),
    ];
    for (hex, line, rule) in cases {
        assert_eq!(
            errors(|part| part["firmware"] = hex.into()),
            [format!(
                r##""netlist.firmware" ["#u1"] {{"field":"firmware","line":{line},"rule":"{rule}"}}"##
            )],
            "{hex:?}"
        );
    }
}

#[test]
fn the_last_byte_of_the_flash_is_in_it() {
    let netlist = chip(|part| part["firmware"] = ":017FFF00AAD7\n:00000001FF".into()).unwrap();
    let flash = values(&netlist).firmware.flash().to_vec();
    assert_eq!(flash.len(), Atmega328p::FLASH_BYTES);
    assert_eq!(flash[Atmega328p::FLASH_BYTES - 1], 0xAA);
}

#[test]
fn the_factory_internal_rc_fuses_are_unsupported() {
    assert_eq!(
        errors(|part| part["fuses"] =
            serde_json::json!({ "low": 0x62, "high": 0xD9, "extended": 0xFF })),
        [r##""netlist.value" ["#u1"] {"field":"fuses.low","rule":"unsupported","value":98.0}"##]
    );
    assert_eq!(
        message(|part| part["fuses"]["low"] = 0x62.into()),
        "#u1 fuses.low is 0x62; the model does not run that mode"
    );
}

#[test]
fn records_that_meet_without_overlapping_are_one_image() {
    let hex = ":0400000001020304F2\n:020004000506EF\n:00000001FF";
    let netlist = chip(|part| part["firmware"] = hex.into()).unwrap();
    assert_eq!(values(&netlist).firmware.flash(), [1, 2, 3, 4, 5, 6]);
}

#[test]
fn only_modes_the_model_runs_are_supported() {
    let fuses = |low: u8, high: u8, extended: u8| {
        move |part: &mut serde_json::Value| {
            part["fuses"] = serde_json::json!({ "low": low, "high": high, "extended": extended })
        }
    };
    for low in [0xF6, 0xF7, 0xFE, 0xFF] {
        assert!(chip(fuses(low, 0xD9, 0xFF)).is_ok(), "{low:#x}");
    }
    for extended in [0xFC, 0xFD, 0xFE, 0xFF, 0x07] {
        assert!(chip(fuses(0xFF, 0xD9, extended)).is_ok(), "{extended:#x}");
    }
    for (low, high, extended, field, byte) in [
        (0xF5, 0xD9, 0xFF, "fuses.low", 245),
        (0xF0, 0xD9, 0xFF, "fuses.low", 240),
        (0xBF, 0xD9, 0xFF, "fuses.low", 191),
        (0xFF, 0x99, 0xFF, "fuses.high", 153),
        (0xFF, 0xDE, 0xFF, "fuses.high", 222),
        (0xFF, 0xD9, 0xFB, "fuses.extended", 251),
        (0xFF, 0xD9, 0xF8, "fuses.extended", 248),
    ] {
        assert_eq!(
            errors(fuses(low, high, extended)),
            [format!(
                r##""netlist.value" ["#u1"] {{"field":"{field}","rule":"unsupported","value":{byte}.0}}"##
            )]
        );
    }
}

#[test]
fn rstdisbl_makes_pc6_a_port_pin() {
    let netlist = chip(|part| part["fuses"]["high"] = 0x59.into()).unwrap();
    assert!(!values(&netlist).fuses.has_reset());
}

#[test]
fn a_threshold_is_a_fraction_of_the_supply_with_low_at_most_high() {
    let value = |v: f64| serde_json::json!({ "value": v, "unit": "1" });
    assert_eq!(
        errors(|part| {
            part["input_threshold"]["high"] = value(1.0);
            part["reset_threshold"]["low"] = value(0.0);
        }),
        [
            r##""netlist.value" ["#u1"] {"field":"input_threshold.high","rule":"fraction","value":1.0}"##,
            r##""netlist.value" ["#u1"] {"field":"reset_threshold.low","rule":"fraction","value":0.0}"##,
        ]
    );
    assert_eq!(
        errors(|part| part["input_threshold"]["low"] = value(0.6)),
        [
            r##""netlist.value" ["#u1"] {"field":"input_threshold.low","rule":"above_high","value":0.6}"##
        ]
    );
    let equal = chip(|part| part["reset_threshold"]["high"] = value(0.42)).unwrap();
    assert_eq!(values(&equal).reset_threshold.high.si.get(), 0.42);
    assert_eq!(
        errors(|part| {
            part["reset_threshold"]["high"] = serde_json::json!({ "value": 0, "unit": "V" });
        }),
        [
            r##""netlist.unit" ["#u1"] {"expected":"1","field":"reset_threshold.high","found":"V"}"##,
            r##""netlist.value" ["#u1"] {"field":"reset_threshold.high","rule":"fraction","value":0.0}"##,
        ]
    );
}

#[test]
fn each_value_names_its_dotted_field() {
    assert_eq!(
        errors(|part| {
            part["clock_hz"]["value"] = 0.into();
            part["reset_threshold"]["high"]["unit"] = "V".into();
            part["output"]["low_drop"]["value"] = (-0.1).into();
            part["pull_up_resistance"]["value"] = 0.0.into();
            part["leakage_resistance"]["value"] = (-1.0).into();
        }),
        [
            r##""netlist.value" ["#u1"] {"field":"clock_hz","rule":"positive","value":0.0}"##,
            r##""netlist.value" ["#u1"] {"field":"output.low_drop","rule":"non_negative","value":-0.1}"##,
            r##""netlist.value" ["#u1"] {"field":"pull_up_resistance","rule":"positive","value":0.0}"##,
            r##""netlist.value" ["#u1"] {"field":"leakage_resistance","rule":"positive","value":-1.0}"##,
            r##""netlist.unit" ["#u1"] {"expected":"1","field":"reset_threshold.high","found":"V"}"##,
        ]
    );
}

#[test]
fn a_clock_is_a_whole_number_of_hertz_and_a_ratings_object_names_every_key() {
    let syntax = |edit: fn(&mut serde_json::Value)| {
        let errors = errors(edit);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with(r##""netlist.syntax""##), "{errors:?}");
    };
    syntax(|part| part["clock_hz"]["value"] = 16e6.into());
    syntax(|part| part["clock_hz"]["value"] = 5_000_000_000_u64.into());
    syntax(|part| part["ratings"] = serde_json::json!({}));
    syntax(|part| part["fuses"]["low"] = 256.into());
}

fn ratings() -> serde_json::Value {
    let value = |value: f64, unit: &str| serde_json::json!({ "value": value, "unit": unit });
    serde_json::json!({
        "pin_current": value(0.04, "A"),
        "supply_current": value(0.2, "A"),
        "rail_margin": value(0.5, "V"),
        "reset_voltage": value(13.0, "V"),
        "supply_voltage": value(6.0, "V"),
    })
}

#[test]
fn a_part_with_no_ratings_object_is_unrated() {
    assert_eq!(values(&chip(|_| {}).unwrap()).ratings, None);
}

#[test]
fn the_ratings_are_read_in_si_and_a_null_key_is_unrated() {
    let chip = values(
        &chip(|part| {
            part["ratings"] = ratings();
            part["ratings"]["reset_voltage"] = serde_json::Value::Null;
        })
        .unwrap(),
    );
    let ratings = chip.ratings.unwrap();
    let si = |value: Option<orbe_engine::netlist::Value>| value.map(|value| value.si.get());
    assert_eq!(si(ratings.pin_current), Some(0.04));
    assert_eq!(si(ratings.supply_current), Some(0.2));
    assert_eq!(si(ratings.rail_margin), Some(0.5));
    assert_eq!(si(ratings.reset_voltage), None);
    assert_eq!(si(ratings.supply_voltage), Some(6.0));
}

#[test]
fn each_rating_names_its_dotted_field() {
    assert_eq!(
        errors(|part| {
            part["ratings"] = ratings();
            part["ratings"]["pin_current"]["unit"] = "V".into();
            part["ratings"]["rail_margin"]["value"] = 0.0.into();
            part["ratings"]["supply_voltage"]["value"] = (-6.0).into();
        }),
        [
            r##""netlist.unit" ["#u1"] {"expected":"A","field":"ratings.pin_current","found":"V"}"##,
            r##""netlist.value" ["#u1"] {"field":"ratings.rail_margin","rule":"positive","value":0.0}"##,
            r##""netlist.value" ["#u1"] {"field":"ratings.supply_voltage","rule":"positive","value":-6.0}"##,
        ]
    );
    let missing = errors(|part| {
        part["ratings"] = ratings();
        part["ratings"]
            .as_object_mut()
            .unwrap()
            .remove("supply_current");
    });
    assert_eq!(missing.len(), 1);
    assert!(
        missing[0].starts_with(r##""netlist.syntax""##),
        "{missing:?}"
    );
}

#[test]
fn every_pin_is_required() {
    assert_eq!(
        errors(|part| {
            part["pins"].as_object_mut().unwrap().remove("GND_22");
        }),
        [r##""netlist.pins" ["#u1"] {"missing":["GND_22"],"unexpected":[]}"##]
    );
}

/// Figure 1-1 names pins 8 and 22 `GND`, one function.
#[test]
fn the_two_ground_pins_name_one_net() {
    assert_eq!(
        errors(|part| part["pins"]["GND_22"] = "agnd".into()),
        [
            r##""netlist.same_net" ["#u1.pin.GND_22","#u1.pin.GND_8"] {"expected":"gnd","found":"agnd"}"##
        ]
    );
    assert_eq!(
        message(|part| part["pins"]["GND_22"] = "agnd".into()),
        "pin #u1.pin.GND_22 is on `agnd`; it must be on `gnd`, as #u1.pin.GND_8 is"
    );
}
