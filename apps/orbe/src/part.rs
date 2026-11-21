//! Text for `orbe part` and `orbe search`.

use std::collections::BTreeMap;

use orbe_scene::{BlockCard, Card, holes_text, param_sentence, si_amount, strip_text};
use orbe_session::BRIGHTNESS_WINDOW;

pub fn text(card: &Card) -> String {
    let mut lines = Vec::new();
    lines.push(format!("{}  {}", card.name, card.version));
    lines.push(card.title.clone());
    if let Some(line) = maker_line(card) {
        lines.push(line);
    }
    if let Some(datasheet) = &card.real.datasheet {
        lines.push(format!("datasheet  {datasheet}"));
    }
    lines.push(format!("license  {}", card.license));
    lines.push(format!("hash  {}", card.hash));
    if let Some(description) = &card.description {
        lines.push(String::new());
        lines.push(description.clone());
    }
    if let Some(categories) = &card.categories {
        lines.push(format!("categories  {}", categories.join(", ")));
    }
    lines.push(String::new());
    lines.push("pins".into());
    let hole = |name: &str| {
        card.holes
            .as_ref()
            .is_some_and(|holes| holes.strips.iter().flatten().any(|h| h == name))
    };
    let pins: Vec<_> = card.pins.iter().filter(|pin| !hole(&pin.name)).collect();
    if pins.is_empty() && card.holes.is_some() {
        lines.push("  only its holes".into());
    } else {
        push_or_none(&mut lines, pins.is_empty());
    }
    for pin in pins {
        let detail = if let Some(leads) = &pin.leads {
            leads
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        } else if let Some(refs) = &pin.refs {
            refs.join(", ")
        } else {
            String::new()
        };
        if detail.is_empty() {
            lines.push(format!("  {}", pin.name));
        } else {
            lines.push(format!("  {}  {detail}", pin.name));
        }
    }
    if let Some(holes) = &card.holes {
        lines.push(String::new());
        lines.push(format!(
            "holes  {} mm deep, each a pin with one lead; a strip joins its holes",
            holes.depth
        ));
        lines.push(format!(
            "  {}",
            holes_text(
                card.pins
                    .iter()
                    .map(|p| p.name.as_str())
                    .filter(|n| hole(n))
            )
        ));
        let mut sizes: Vec<usize> = holes.strips.iter().map(Vec::len).collect();
        sizes.sort_unstable();
        sizes.dedup();
        for size in sizes {
            let strips: Vec<String> = holes
                .strips
                .iter()
                .filter(|strip| strip.len() == size)
                .map(|strip| strip_text(strip, ""))
                .collect();
            lines.push(format!("  strips of {size}  {}", strips.join("; ")));
        }
    }
    lines.push(String::new());
    lines.push("params".into());
    push_or_none(&mut lines, card.params.is_empty());
    for param in &card.params {
        lines.push(format!("  {}: {}", param.name, param_sentence(param)));
    }
    lines.push(String::new());
    lines.push("ratings".into());
    push_or_none(&mut lines, card.ratings.is_empty());
    for rating in &card.ratings {
        let shown = match (rating.value, rating.unit.as_deref()) {
            (Some(value), Some(unit)) => si_amount(value, unit),
            _ => "unrated".to_string(),
        };
        lines.push(format!("  {}  {shown}", rating.name));
    }
    lines.push(String::new());
    lines.push("not simulated".into());
    push_or_none(&mut lines, card.not_simulated.is_empty());
    for gap in &card.not_simulated {
        let pins = gap
            .pins
            .as_ref()
            .map(|pins| format!(" ({})", pins.join(", ")))
            .unwrap_or_default();
        lines.push(format!("  {}{pins}: {}", gap.what, gap.why));
    }
    if let Some(docs) = &card.docs {
        lines.push(String::new());
        lines.push("docs".into());
        for doc in docs {
            match &doc.url {
                Some(url) => lines.push(format!("  {}  {url}", doc.name)),
                None => lines.push(format!("  {}", doc.name)),
            }
        }
    }
    lines.push(String::new());
    lines.push("buy".into());
    let links = card.buy.as_deref().unwrap_or_default();
    push_or_none(&mut lines, links.is_empty());
    for link in links {
        lines.push(format!("  {}  {}  {}", link.seller, link.url, link.checked));
    }
    lines.push(String::new());
    lines.push("files".into());
    for file in &card.files {
        lines.push(format!(
            "  {}  {} bytes  {}  {}",
            file.path, file.bytes, file.sha256, file.license
        ));
    }
    if !card.uses.is_empty() {
        lines.push(String::new());
        lines.push("uses".into());
        for (id, version) in &card.uses {
            lines.push(format!("  {id}  {version}"));
        }
    }
    if !card.used_by.is_empty() {
        lines.push(String::new());
        lines.push("used by".into());
        for id in &card.used_by {
            lines.push(format!("  {id}"));
        }
    }
    if card.lit.is_some() || card.dim.is_some() {
        lines.push(String::new());
        lines.push("brightness".into());
        lines.push(format!("  the average i over {BRIGHTNESS_WINDOW}"));
        if let Some(lit) = &card.lit {
            lines.push(format!(
                "  lit while that average is above {}",
                si_amount(lit.above, &lit.unit)
            ));
        }
        if let Some(dim) = &card.dim {
            let above = si_amount(dim.above, &dim.unit);
            if let Some(lit) = &card.lit {
                lines.push(format!(
                    "  dim while that average is above {above} and at or below {}",
                    si_amount(lit.above, &lit.unit)
                ));
            } else {
                lines.push(format!("  dim while that average is above {above}"));
            }
            lines.push(format!("  off while that average is at or below {above}"));
        }
    }
    if card.pressable {
        lines.push("press and release close and open it".into());
    }
    if let Some(line) = orbe_scene::breadboard_line(&card.name) {
        lines.push(String::new());
        lines.push(line);
    }
    lines.push(String::new());
    lines.join("\n")
}

pub fn block_text(card: &BlockCard) -> String {
    let mut lines = vec![
        format!("{}  {}", card.name, card.version),
        card.title.clone(),
    ];
    if let Some(description) = &card.description {
        lines.push(String::new());
        lines.push(description.clone());
    }
    lines.push(String::new());
    lines.push(format!(
        "a Block. install it with `orbe add {}` in an empty folder",
        card.name
    ));
    lines.push(String::new());
    lines.join("\n")
}

pub fn blocks_text(cards: &[BlockCard]) -> String {
    let mut lines = Vec::new();
    for card in cards {
        lines.push(format!("{}  {}", card.name, card.title));
        if let Some(description) = &card.description {
            lines.push(format!("  {description}"));
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

/// Every part a scene can place, grouped by its first category, then the Blocks. A part that
/// only exists inside a board is left out.
pub fn catalog_groups() -> Vec<(String, Vec<(String, String)>)> {
    let mut groups: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for card in orbe_scene::catalog_cards() {
        if !card.used_by.is_empty() {
            continue;
        }
        let category = card
            .categories
            .as_ref()
            .and_then(|categories| categories.first())
            .cloned()
            .unwrap_or_else(|| "other".to_string());
        groups
            .entry(category)
            .or_default()
            .push((card.name, card.title));
    }
    let mut groups: Vec<_> = groups.into_iter().collect();
    for (_, items) in &mut groups {
        items.sort();
    }
    let mut blocks: Vec<_> = orbe_scene::builtin_blocks()
        .into_iter()
        .map(|card| (card.name, card.title))
        .collect();
    blocks.sort();
    groups.push(("blocks".to_string(), blocks));
    groups
}

pub fn catalog_text(groups: &[(String, Vec<(String, String)>)]) -> String {
    let mut lines = Vec::new();
    for (index, (category, items)) in groups.iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }
        lines.push(category.clone());
        for (id, title) in items {
            lines.push(format!("{id}  {title}"));
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

pub fn search_text(cards: &[Card], blocks: &[BlockCard]) -> String {
    if cards.is_empty() && blocks.is_empty() {
        return "no parts match\n".to_string();
    }
    let mut lines: Vec<String> = cards
        .iter()
        .map(|card| format!("{}  {}", card.name, card.title))
        .collect();
    lines.extend(
        blocks
            .iter()
            .map(|card| format!("{}  {}", card.name, card.title)),
    );
    lines.push(String::new());
    lines.join("\n")
}

fn maker_line(card: &Card) -> Option<String> {
    let maker = card.real.maker.as_deref().unwrap_or("");
    let number = card.real.number.as_deref().unwrap_or("");
    if maker.is_empty() && number.is_empty() && !card.real.generic {
        return None;
    }
    let mut line = match (maker.is_empty(), number.is_empty()) {
        (false, false) => format!("{maker}  {number}"),
        (false, true) => maker.to_string(),
        (true, false) => number.to_string(),
        (true, true) => String::new(),
    };
    if card.real.generic {
        if line.is_empty() {
            line = "generic".to_string();
        } else {
            line.push_str(", generic");
        }
    }
    Some(line)
}

fn push_or_none(lines: &mut Vec<String>, empty: bool) {
    if empty {
        lines.push("  none".into());
    }
}
