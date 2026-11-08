//! Buy URLs the checker refuses, and one host the seller table allows.

mod common;

use std::fs;

use orbe_scene::{Code, Diagnostic, Files};
use serde_json::{Value, json};

const SELLER_HINT: &str = "sellers are listed in `packages/scene/buy-sellers.jsonc`. a new seller is a one-line addition, reviewed for affiliate and tracking links";

struct Rejected {
    seller: &'static str,
    url: &'static str,
    message: &'static str,
    hint: &'static str,
    /// Write a valid copy of this entry first. This row is the second copy.
    repeat: bool,
}

const REJECTED: &[Rejected] = &[
    Rejected {
        seller: "DigiKey",
        url: "https://www.digikey.com@evil.example/phish",
        message: "`url` has userinfo",
        hint: "no `@`",
        repeat: false,
    },
    Rejected {
        seller: "DigiKey",
        url: "https://www.digikey.com/a b",
        message: "`url` has whitespace or a control character",
        hint: "whitespace or control character",
        repeat: false,
    },
    Rejected {
        seller: "DigiKey",
        url: "https://www.digikey.com/trail ",
        message: "`url` has whitespace or a control character",
        hint: "whitespace or control character",
        repeat: false,
    },
    Rejected {
        seller: "DigiKey",
        url: "https://www.digikey.com/p\n",
        message: "`url` has whitespace or a control character",
        hint: "whitespace or control character",
        repeat: false,
    },
    Rejected {
        seller: "DigiKey",
        url: "https:// /p",
        message: "`url` has whitespace or a control character",
        hint: "whitespace or control character",
        repeat: false,
    },
    Rejected {
        seller: "DigiKey",
        url: "https://.",
        message: "`url` host is not this seller's",
        hint: SELLER_HINT,
        repeat: false,
    },
    Rejected {
        seller: "DigiKey",
        url: "https://d\u{0456}gikey.com/",
        message: "`url` host is not this seller's",
        hint: SELLER_HINT,
        repeat: false,
    },
    Rejected {
        seller: "DigiKey",
        url: "HTTPS://www.digikey.com/p",
        message: "`url` is not an https URL",
        hint: "lowercase https://",
        repeat: false,
    },
    Rejected {
        seller: "Amazon",
        url: "https://amzn.to/3xYz",
        message: "`seller` is not an allowed seller",
        hint: SELLER_HINT,
        repeat: false,
    },
    Rejected {
        seller: "DigiKey",
        url: "https://store.arduino.cc/allowed",
        message: "`url` host is not this seller's",
        hint: SELLER_HINT,
        repeat: false,
    },
    Rejected {
        seller: "DigiKey",
        url: "https://www.digikey.com/duplicate",
        message: "`buy` repeats an entry",
        hint: "leave one of the two entries",
        repeat: true,
    },
];

fn entry(seller: &str, url: &str) -> Value {
    json!({
        "seller": seller,
        "url": url,
        "checked": "2026-11-08",
    })
}

fn read_buy(entries: Vec<Value>) -> Result<(), Vec<Diagnostic>> {
    let dir = common::project("nano-button");
    let part = json!({
        "format": 1,
        "joins_only": true,
        "buy": entries,
    });
    fs::write(
        dir.path().join("parts/probe.jsonc"),
        serde_json::to_string_pretty(&part).unwrap(),
    )
    .unwrap();
    let scene_path = dir.path().join("scene.jsonc");
    let scene = fs::read_to_string(&scene_path).unwrap();
    let scene = scene.replacen(
        "\"led1\": { \"type\": \"led-5mm\", \"color\": \"red\" },",
        "\"led1\": { \"type\": \"led-5mm\", \"color\": \"red\" },\n    \"probe\": { \"type\": \"./parts/probe.jsonc\" },",
        1,
    );
    assert!(
        scene.contains("probe"),
        "the fixture scene no longer has the led line"
    );
    fs::write(&scene_path, scene).unwrap();
    orbe_scene::read(&Files::disk(), &scene_path).map(|_| ())
}

#[test]
fn rejected_buy_urls_name_the_fault() {
    let mut entries = Vec::new();
    for row in REJECTED {
        if row.repeat {
            entries.push(entry(row.seller, row.url));
        }
        entries.push(entry(row.seller, row.url));
    }
    let found = read_buy(entries).expect_err("the table is refused");
    assert_eq!(
        found.len(),
        REJECTED.len(),
        "findings:\n{}",
        found
            .iter()
            .map(|finding| format!("{} | {}", finding.message, finding.hint))
            .collect::<Vec<_>>()
            .join("\n")
    );
    for (row, finding) in REJECTED.iter().zip(&found) {
        assert_eq!(finding.code, Code::Syntax, "{}", row.url);
        assert_eq!(finding.message, row.message, "{}", row.url);
        assert!(
            finding.hint.contains(row.hint),
            "{}\nhint `{}` does not contain `{}`",
            row.url,
            finding.hint,
            row.hint
        );
    }
}

#[test]
fn an_allowed_host_passes() {
    let found = read_buy(vec![
        entry("DigiKey", "https://www.digikey.com/allowed"),
        entry("Arduino", "https://store.arduino.cc/allowed"),
    ]);
    assert!(found.is_ok(), "{found:?}");
}
