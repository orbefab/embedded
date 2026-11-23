//! A card for every catalog part, and how search ranks them.

use std::fs;
use std::path::PathBuf;

use sha2::{Digest, Sha256};

use orbe_scene::{
    builtin_blocks, card, catalog_cards, folder_hash, ids_with_segment, missing_fields, near_names,
    param_sentence, search, search_blocks,
};

fn parts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../parts")
}

fn blocks_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../blocks")
}

fn title_word(title: &str, word: &str) -> bool {
    title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|have| have.eq_ignore_ascii_case(word))
}

#[test]
fn every_catalog_part_is_a_card() {
    let mut ids: Vec<String> = fs::read_dir(parts_dir())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|id| parts_dir().join(id).join("part.jsonc").is_file())
        .collect();
    ids.sort();
    let cards = catalog_cards(&parts_dir());
    let names: Vec<String> = cards.iter().map(|card| card.name.clone()).collect();
    assert_eq!(names, ids);
    for card in &cards {
        let json = serde_json::to_value(card).unwrap();
        assert_eq!(
            json.get("buy").is_some(),
            card.buy.is_some(),
            "{}",
            card.name
        );
        assert!(json.get("name").is_some());
        assert!(json.get("uses").is_some());
        assert!(json.get("used_by").is_some());
        assert!(json.get("not_simulated").is_some());
        assert!(!card.files.is_empty(), "{}", card.name);
        assert!(!card.license.is_empty(), "{}", card.name);
        assert!(card.hash.starts_with("sha256:"), "{}", card.name);
        assert_eq!(card.hash.len(), 71, "{}", card.name);
        for file in &card.files {
            assert_eq!(file.sha256.len(), 64, "{} {}", card.name, file.path);
            assert!(file.sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(!file.license.is_empty());
        }
    }
    let missing = missing_fields(&cards);
    println!("{missing}");
    assert_eq!(
        missing.buy,
        cards.iter().filter(|card| card.buy.is_none()).count()
    );
    assert!(missing.buy < missing.parts);
}

#[test]
fn a_passive_a_chip_and_a_board_keep_only_what_their_files_say() {
    let passive = card(&parts_dir(), "resistor-axial").unwrap();
    assert!(passive.description.is_some());
    assert!(passive.categories.is_some());
    assert!(passive.real.generic);
    assert_eq!(passive.real.maker.as_deref(), Some("Yageo"));
    assert_eq!(passive.real.number.as_deref(), Some("CFR-25"));
    let docs = passive.docs.unwrap();
    assert_eq!(docs.len(), 1);
    assert!(docs[0].url.is_none());
    assert!(passive.ratings.iter().any(|rating| rating.name == "power"));
    assert_eq!(
        passive.buy.as_ref().map(|links| links[0].url.as_str()),
        Some("https://www.digikey.com/en/products/detail/yageo/CFR-25JB-52-220R/1295")
    );
    assert!(passive.uses.is_empty());
    assert_eq!(passive.license, "MIT OR Apache-2.0");
    assert!(passive.hash.starts_with("sha256:"));
    assert!(
        passive
            .files
            .iter()
            .all(|file| file.license == "MIT OR Apache-2.0")
    );

    let chip = card(&parts_dir(), "atmega328p-dip28").unwrap();
    assert!(
        chip.description
            .as_deref()
            .unwrap()
            .contains("ATmega328P-PU")
    );
    assert!(
        chip.categories
            .as_ref()
            .unwrap()
            .iter()
            .any(|category| category == "microcontroller")
    );
    assert_eq!(chip.real.maker.as_deref(), Some("Microchip"));
    assert_eq!(chip.real.number.as_deref(), Some("ATmega328P-PU"));
    let docs = chip.docs.unwrap();
    assert!(docs.iter().any(|doc| {
        doc.name == "DS40002061B"
            && doc.url.as_deref()
                == Some("https://ww1.microchip.com/downloads/en/DeviceDoc/ATmega48A-PA-88A-PA-168A-PA-328-P-DS-DS40002061B.pdf")
    }));
    assert!(
        docs.iter()
            .any(|doc| doc.name == "avr-1.8.6 boards.txt" && doc.url.is_none())
    );
    assert!(
        chip.params
            .iter()
            .any(|param| param.name == "firmware" && param.required)
    );
    assert!(chip.buy.is_none());

    let board = card(&parts_dir(), "nano-board").unwrap();
    assert!(board.description.is_some());
    assert!(board.categories.is_some());
    assert_eq!(
        board.uses.get("led-0603").map(String::as_str),
        Some("0.2.1")
    );
    assert!(board.used_by.is_empty());
    let docs = board.docs.unwrap();
    assert!(docs.iter().any(|doc| doc.url.is_some()));
    assert!(board.pins.iter().any(|pin| pin.refs.is_some()));
    assert!(board.buy.is_none());
}

#[test]
fn search_ranks_an_exact_id_then_whole_words_then_the_rest() {
    let led = search(&parts_dir(), "led");
    let mut saw_rest = false;
    let mut title_hits = Vec::new();
    for card in &led {
        if title_word(&card.title, "led") {
            assert!(!saw_rest, "{}", card.name);
            title_hits.push(card.name.as_str());
        } else {
            saw_rest = true;
        }
    }
    assert_eq!(
        title_hits,
        [
            "led-0603",
            "led-0805",
            "led-3mm",
            "led-3mm-blue",
            "led-3mm-green",
            "led-3mm-white",
            "led-3mm-yellow",
            "led-5mm",
            "led-5mm-blue",
            "led-5mm-green",
            "led-5mm-white",
            "led-5mm-yellow",
            "neopixel-stick-8",
            "sk68xxmini-hs",
        ]
    );
    let rest: Vec<&str> = led
        .iter()
        .skip(title_hits.len())
        .map(|card| card.name.as_str())
        .collect();
    assert!(rest.contains(&"nano-board"));
    assert!(rest.contains(&"uno-board"));
    assert!(rest.windows(2).all(|pair| pair[0] < pair[1]));

    assert_eq!(search(&parts_dir(), "led-5mm")[0].name, "led-5mm");
    assert_eq!(search(&parts_dir(), "LED-5MM")[0].name, "led-5mm");

    let resistors = search(&parts_dir(), "resistor");
    let names: Vec<&str> = resistors.iter().map(|card| card.name.as_str()).collect();
    assert_eq!(
        &names[..4],
        [
            "potentiometer-wh148",
            "resistor-0603",
            "resistor-array-4x0603",
            "resistor-axial",
        ]
    );

    let switches = search(&parts_dir(), "switch");
    let names: Vec<&str> = switches.iter().map(|card| card.name.as_str()).collect();
    assert_eq!(
        &names[..9],
        [
            "nmos-irf520-to220",
            "npn-2n2222-to92",
            "npn-l8050-sot23",
            "pmos-irf9540n-to220",
            "pnp-2n3906-to92",
            "tactile-switch-6mm",
            "tactile-switch-ntc013",
            "tactile-switch-smd",
            "tactile-switch-ts42",
        ]
    );
    let diode = names
        .iter()
        .position(|name| *name == "diode-cd1206")
        .unwrap();
    assert!(diode > 8, "{names:?}");

    let boards = search(&parts_dir(), "board");
    let names: Vec<&str> = boards.iter().map(|card| card.name.as_str()).collect();
    assert_eq!(&names[..2], ["nano-board", "uno-board"]);
    let diode = names
        .iter()
        .position(|name| *name == "diode-1n4007")
        .unwrap();
    let resistor = names
        .iter()
        .position(|name| *name == "resistor-axial")
        .unwrap();
    assert!(diode > 1 && resistor > 1, "{names:?}");
}

#[test]
fn file_hashes_and_the_folder_hash_match_the_catalog() {
    for card in catalog_cards(&parts_dir()) {
        let dir = parts_dir().join(&card.name);
        assert_eq!(card.hash, folder_hash(&dir).unwrap(), "{}", card.name);
        for file in &card.files {
            let bytes = fs::read(dir.join(&file.path)).unwrap();
            assert_eq!(
                file.bytes,
                bytes.len() as u64,
                "{} {}",
                card.name,
                file.path
            );
            let digest = Sha256::digest(&bytes);
            let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
            assert_eq!(file.sha256, hex, "{} {}", card.name, file.path);
        }
    }
}

#[test]
fn used_by_matches_the_uses_graph() {
    let cards = catalog_cards(&parts_dir());
    for card in &cards {
        let mut expect: Vec<&str> = cards
            .iter()
            .filter(|other| other.uses.contains_key(&card.name))
            .map(|other| other.name.as_str())
            .collect();
        expect.sort();
        let have: Vec<&str> = card.used_by.iter().map(String::as_str).collect();
        assert_eq!(have, expect, "{}", card.name);
    }
}

#[test]
fn a_null_rating_stays_on_the_card_and_a_number_default_keeps_its_unit() {
    let led = card(&parts_dir(), "led-3mm").unwrap();
    let names: Vec<&str> = led
        .ratings
        .iter()
        .map(|rating| rating.name.as_str())
        .collect();
    assert_eq!(names, ["current", "power"]);
    let led_current = led
        .ratings
        .iter()
        .find(|rating| rating.name == "current")
        .unwrap();
    let led_power = led
        .ratings
        .iter()
        .find(|rating| rating.name == "power")
        .unwrap();
    assert_eq!(led_current.value, Some(0.03));
    assert_eq!(led_power.value, Some(0.075));

    let diode = card(&parts_dir(), "diode-1n4007").unwrap();
    let current = diode
        .ratings
        .iter()
        .find(|rating| rating.name == "current")
        .unwrap();
    let power = diode
        .ratings
        .iter()
        .find(|rating| rating.name == "power")
        .unwrap();
    assert_eq!(current.value, Some(1.0));
    assert!(power.value.is_none());

    let pot = card(&parts_dir(), "potentiometer-wh148").unwrap();
    let position = pot
        .params
        .iter()
        .find(|param| param.name == "position")
        .unwrap();
    let json = serde_json::to_value(position).unwrap();
    assert_eq!(json["default"], serde_json::json!(0.5));
    assert_eq!(json["above"], serde_json::json!(0));
    assert_eq!(json["below"], serde_json::json!(1));
    assert_eq!(json["unit"], "1");
    assert_eq!(json["live"], true);
    assert_eq!(
        param_sentence(position),
        "a number above 0 and below 1, 0.5 by default, and `set` changes it while a session runs"
    );
    let resistance = pot
        .params
        .iter()
        .find(|param| param.name == "resistance")
        .unwrap();
    assert!(
        serde_json::to_value(resistance)
            .unwrap()
            .get("live")
            .is_none()
    );

    let switch = card(&parts_dir(), "tactile-switch-6mm").unwrap();
    let bounce = switch
        .params
        .iter()
        .find(|param| param.name == "bounce")
        .unwrap();
    let max_bounces = switch
        .params
        .iter()
        .find(|param| param.name == "max_bounces")
        .unwrap();
    assert!(serde_json::to_value(bounce).unwrap()["default"].is_null());
    assert_eq!(
        serde_json::to_value(max_bounces).unwrap()["default"],
        serde_json::json!(2)
    );
    assert_eq!(param_sentence(bounce), "a value in s, not set by default");
    assert_eq!(param_sentence(max_bounces), "a number, 2 by default");
}

#[test]
fn one_id_segment_names_that_part_and_several_stay_a_list() {
    let owned = |ids: &[&str]| -> Vec<String> { ids.iter().map(|id| (*id).to_string()).collect() };
    assert_eq!(
        ids_with_segment("1n4007", &parts_dir(), &blocks_dir()),
        owned(&["diode-1n4007"])
    );
    assert_eq!(
        ids_with_segment("1N4007", &parts_dir(), &blocks_dir()),
        owned(&["diode-1n4007"])
    );
    assert_eq!(
        ids_with_segment("switch", &parts_dir(), &blocks_dir()),
        owned(&[
            "tactile-switch-6mm",
            "tactile-switch-ntc013",
            "tactile-switch-smd",
            "tactile-switch-ts42",
        ])
    );
    assert!(ids_with_segment("not-a-part-zzzz", &parts_dir(), &blocks_dir()).is_empty());
    assert_eq!(
        ids_with_segment("fade", &parts_dir(), &blocks_dir()),
        owned(&["nano-led-fade"])
    );
}

#[test]
fn search_finds_a_block_by_a_word_of_its_id() {
    let fade = search_blocks(&parts_dir(), &blocks_dir(), "fade");
    assert_eq!(fade.len(), 1);
    assert_eq!(fade[0].name, "nano-led-fade");
    assert_eq!(
        near_names("nano-led-fad", &parts_dir(), &blocks_dir())
            .first()
            .map(String::as_str),
        Some("nano-led-fade")
    );
    let names: Vec<String> = builtin_blocks(&parts_dir(), &blocks_dir())
        .into_iter()
        .map(|card| card.name)
        .collect();
    assert_eq!(
        names,
        [
            "nano-button-led",
            "nano-led-fade",
            "nano-light-comparator",
            "nano-neopixel-stick",
            "uno-button-led",
        ]
    );
}

#[test]
fn an_unknown_id_names_the_closest_parts() {
    assert!(card(&parts_dir(), "LED-5MM").is_none());
    assert_eq!(
        near_names("LED-5MM", &parts_dir(), &blocks_dir())
            .first()
            .map(String::as_str),
        Some("led-5mm")
    );
    assert_eq!(
        near_names("resistor-axil", &parts_dir(), &blocks_dir())
            .first()
            .map(String::as_str),
        Some("resistor-axial")
    );
    assert!(
        near_names("resistor-axial", &parts_dir(), &blocks_dir())
            .iter()
            .all(|id| id != "resistor-axial")
    );
    assert!(near_names("not-a-part-zzzz", &parts_dir(), &blocks_dir()).len() <= 8);
}

#[test]
fn every_file_of_a_board_is_mit_or_apache() {
    let uno = card(&parts_dir(), "uno-board").unwrap();
    let licence = |path: &str| {
        uno.files
            .iter()
            .find(|file| file.path == path)
            .map(|file| file.license.as_str())
    };
    for path in ["part.jsonc", "uno-board.glb"] {
        assert_eq!(licence(path), Some("MIT OR Apache-2.0"), "{path}");
    }
    assert!(
        uno.files
            .iter()
            .all(|file| file.path != "model.py" && file.path != "asset.json")
    );
    assert_eq!(uno.license, "MIT OR Apache-2.0");
    let nano = card(&parts_dir(), "nano-board").unwrap();
    assert!(
        nano.files
            .iter()
            .all(|file| file.license == "MIT OR Apache-2.0")
    );
}

#[test]
fn the_esp32s3_rom_is_apache_and_the_part_names_it() {
    let chip = card(&parts_dir(), "esp32s3-qfn56").unwrap();
    assert_eq!(
        chip.license,
        "(MIT OR Apache-2.0) AND Apache-2.0 AND MIT AND BSD-3-Clause AND BSD-2-Clause AND LicenseRef-Cadence-Xtensa AND LicenseRef-newlib AND LicenseRef-GCC-Runtime-Library-Exception"
    );
    let licence = |path: &str| {
        chip.files
            .iter()
            .find(|file| file.path == path)
            .map(|file| file.license.as_str())
    };
    let image = "Apache-2.0 AND MIT AND BSD-3-Clause AND BSD-2-Clause AND LicenseRef-Cadence-Xtensa AND LicenseRef-newlib AND LicenseRef-GCC-Runtime-Library-Exception";
    assert_eq!(licence("esp32s3_rev0_rom.elf"), Some("Apache-2.0"));
    assert_eq!(licence("LICENSE-rom"), Some("Apache-2.0"));
    assert_eq!(licence("NOTICE"), Some("Apache-2.0"));
    assert_eq!(licence("part.jsonc"), Some("MIT OR Apache-2.0"));
    assert_eq!(licence("hello_world.bin"), Some(image));
    assert_eq!(licence("partition-table.bin"), Some(image));
    assert_eq!(licence("NOTICE-hello"), Some(image));
    assert_eq!(licence("LICENSE-freertos"), Some("MIT"));
    assert_eq!(licence("LICENSE-tlsf"), Some("BSD-3-Clause"));
    assert_eq!(licence("LICENSE-ubsan"), Some("BSD-2-Clause"));
    assert_eq!(licence("LICENSE-xtensa"), Some("LicenseRef-Cadence-Xtensa"));
    assert_eq!(licence("LICENSE-newlib"), Some("LicenseRef-newlib"));
}
