//! A card for every catalog part, and how search ranks them.

use std::fs;
use std::path::PathBuf;

use orbe_scene::{card, catalog_cards, missing_fields, near_names, search};

fn parts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../parts")
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
    let cards = catalog_cards();
    let names: Vec<String> = cards.iter().map(|card| card.name.clone()).collect();
    assert_eq!(names, ids);
    for card in &cards {
        let json = serde_json::to_value(card).unwrap();
        assert!(json.get("buy").is_none(), "{}", card.name);
        assert!(json.get("name").is_some());
        assert!(json.get("uses").is_some());
        assert!(json.get("used_by").is_some());
        assert!(json.get("not_simulated").is_some());
        assert!(!card.files.is_empty(), "{}", card.name);
        for file in &card.files {
            assert_eq!(file.sha256.len(), 64, "{} {}", card.name, file.path);
            assert!(file.sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(!file.license.is_empty());
        }
        assert!(card.buy.is_none());
    }
    let missing = missing_fields(&cards);
    println!("{missing}");
    assert_eq!(missing.buy, missing.parts);
}

#[test]
fn a_passive_a_chip_and_a_board_keep_only_what_their_files_say() {
    let passive = card("resistor-axial").unwrap();
    assert!(passive.description.is_some());
    assert!(passive.categories.is_some());
    assert!(passive.real.generic);
    assert_eq!(passive.real.maker.as_deref(), Some("Yageo"));
    assert_eq!(passive.real.number.as_deref(), Some("CFR-25"));
    let docs = passive.docs.unwrap();
    assert_eq!(docs.len(), 1);
    assert!(docs[0].url.is_none());
    assert!(passive.ratings.iter().any(|rating| rating.name == "power"));
    assert!(passive.uses.is_empty());
    assert!(
        passive
            .files
            .iter()
            .all(|file| file.license == "MIT OR Apache-2.0")
    );

    let chip = card("atmega328p-dip28").unwrap();
    assert!(chip.description.is_none());
    assert!(chip.categories.is_none());
    assert_eq!(chip.real.maker.as_deref(), Some("Microchip"));
    assert_eq!(chip.real.number.as_deref(), Some("ATmega328P-PU"));
    let docs = chip.docs.unwrap();
    assert!(docs.iter().all(|doc| doc.url.is_none()));
    assert!(docs.iter().any(|doc| doc.name == "DS40002061B"));
    assert!(
        chip.params
            .iter()
            .any(|param| param.name == "firmware" && param.required)
    );
    assert!(chip.buy.is_none());

    let board = card("nano-board").unwrap();
    assert!(board.description.is_some());
    assert!(board.categories.is_some());
    assert_eq!(
        board.uses.get("led-0603").map(String::as_str),
        Some("0.1.0")
    );
    assert!(board.used_by.is_empty());
    let docs = board.docs.unwrap();
    assert!(docs.iter().any(|doc| doc.url.is_some()));
    assert!(board.pins.iter().any(|pin| pin.refs.is_some()));
}

#[test]
fn search_ranks_an_exact_id_then_title_words_then_the_rest() {
    let led = search("led");
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
    assert_eq!(title_hits, ["led-0603", "led-0805", "led-3mm", "led-5mm"]);
    let rest: Vec<&str> = led
        .iter()
        .skip(title_hits.len())
        .map(|card| card.name.as_str())
        .collect();
    assert!(rest.contains(&"nano-board"));
    assert!(rest.contains(&"uno-board"));
    assert!(rest.windows(2).all(|pair| pair[0] < pair[1]));

    assert_eq!(search("led-5mm")[0].name, "led-5mm");
    assert_eq!(search("LED-5MM")[0].name, "led-5mm");

    let resistors = search("resistor");
    let mut previous = String::new();
    let mut saw_rest = false;
    for card in &resistors {
        if title_word(&card.title, "resistor") {
            assert!(!saw_rest);
            assert!(previous < card.name);
            previous = card.name.clone();
        } else {
            saw_rest = true;
        }
    }
    assert!(resistors.iter().any(|card| card.name == "resistor-axial"));
    assert!(
        resistors
            .iter()
            .any(|card| card.name == "potentiometer-wh148")
    );
    let axial = resistors
        .iter()
        .position(|card| card.name == "resistor-axial")
        .unwrap();
    let pot = resistors
        .iter()
        .position(|card| card.name == "potentiometer-wh148")
        .unwrap();
    assert!(axial < pot);
}

#[test]
fn an_unknown_id_names_the_closest_parts() {
    assert!(card("LED-5MM").is_none());
    assert_eq!(
        near_names("LED-5MM").first().map(String::as_str),
        Some("led-5mm")
    );
    assert_eq!(
        near_names("resistor-axil").first().map(String::as_str),
        Some("resistor-axial")
    );
    assert!(
        near_names("resistor-axial")
            .iter()
            .all(|id| id != "resistor-axial")
    );
    assert!(near_names("not-a-part-zzzz").len() <= 8);
}
