//! The skills this binary serves. Each is a markdown file compiled in. The `core` skill's
//! parts section and the `arduino` skill's board section are rendered from the built-in
//! catalog when a skill is printed.

use std::collections::{BTreeMap, BTreeSet};

use orbe_scene::{BoardFacts, Card, Gap, board_facts, catalog_cards, param_sentence, si_amount};

pub struct Skill {
    pub name: &'static str,
    pub about: &'static str,
    text: &'static str,
}

pub const SKILLS: &[Skill] = &[
    Skill {
        name: "core",
        about: "How to use orbe: the loop, scenes, parts, refs, time, tests, sessions and errors. Read it first.",
        text: include_str!("../skills/core.md"),
    },
    Skill {
        name: "arduino",
        about: "Building Arduino Nano and Uno firmware with arduino-cli and naming it in the scene.",
        text: include_str!("../skills/arduino.md"),
    },
];

const PARTS: &str = "<!-- catalog parts -->";

pub fn find(name: &str) -> Option<&'static Skill> {
    SKILLS.iter().find(|s| s.name == name)
}

pub fn names() -> String {
    SKILLS.iter().map(|s| s.name).collect::<Vec<_>>().join(", ")
}

impl Skill {
    pub fn text(&self) -> Result<String, String> {
        fill_boards(&self.text.replace(PARTS, &parts(&catalog_cards())))
    }
}

/// Replaces a `<!-- boards nano-board uno-board -->` marker with one block per named board.
/// A skill with no marker is unchanged, so a board that does not read cannot break it.
fn fill_boards(text: &str) -> Result<String, String> {
    let Some(start) = text.find("<!-- boards") else {
        return Ok(text.to_string());
    };
    let tail = &text[start..];
    let Some(close) = tail.find("-->") else {
        return Err("the boards marker does not close".into());
    };
    let body = tail["<!-- boards".len()..close].trim();
    if body.is_empty() {
        return Err("the boards marker names no board".into());
    }
    let ids: Vec<&str> = body.split_whitespace().collect();
    let rendered = boards(&ids)?;
    let mut out = String::with_capacity(text.len() + rendered.len());
    out.push_str(&text[..start]);
    out.push_str(&rendered);
    out.push_str(&tail[close + 3..]);
    Ok(out)
}

/// One heading and one list per board, from its part file: the chip and its clock, the pins
/// that work, the pins the reader leaves unsimulated, and the L LED.
fn boards(ids: &[&str]) -> Result<String, String> {
    let mut blocks = Vec::with_capacity(ids.len());
    for id in ids {
        blocks.push(board_text(&board_facts(id)?));
    }
    Ok(blocks.join("\n"))
}

fn board_text(board: &BoardFacts) -> String {
    let mut out = format!("### `{}`\n\n", board.id);
    out.push_str(&format!(
        "- `{}`, {}, runs the {} at {}.\n",
        board.id,
        board.title,
        board.chip,
        si_amount(board.clock_hz, "Hz"),
    ));
    out.push_str(&format!("- These pins work: {}.\n", pin_list(&board.works)));
    for names in &board.same {
        out.push_str(&format!("- {} are one net.\n", pin_list(names)));
    }
    for supply in &board.supplies {
        out.push_str(&format!(
            "- `{}` comes from `{}`, a {} regulator.\n",
            supply.pin,
            supply.from,
            si_amount(supply.volts, "V"),
        ));
    }
    for group in group_whys(&board.gaps) {
        let names = pin_list(&group.0);
        let verb = if group.0.len() == 1 { "is" } else { "are" };
        let wire = if group.0.len() == 1 {
            "A wire to it is a `not_simulated` error."
        } else {
            "A wire to one is a `not_simulated` error."
        };
        out.push_str(&format!(
            "- {names} {verb} not simulated. {} {wire}\n",
            group.1
        ));
    }
    for led in &board.leds {
        out.push_str(&format!(
            "- `{}` drives `{}` through `{}`, {}.\n",
            led.pin,
            led.name,
            led.resistor,
            si_amount(led.ohms, "ohm"),
        ));
    }
    out
}

/// Pins that share a reason, in the order the pins are listed.
fn group_whys(gaps: &[Gap]) -> Vec<(Vec<String>, String)> {
    let mut groups: Vec<(Vec<String>, String)> = Vec::new();
    for gap in gaps {
        if let Some(group) = groups.iter_mut().find(|group| group.1 == gap.why) {
            group.0.push(gap.pin.clone());
        } else {
            groups.push((vec![gap.pin.clone()], gap.why.clone()));
        }
    }
    groups
}

/// `` `D0` to `D13`, `RESET` and `5V` ``. A letter-and-number run of three or more with no
/// gap inside it is one range.
fn pin_list(names: &[String]) -> String {
    let mut sets: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    for name in names {
        if let Some((prefix, n)) = split_pin(name) {
            sets.entry(prefix.to_string()).or_default().insert(n);
        }
    }
    let mut items = Vec::new();
    let mut seen = BTreeSet::new();
    for name in names {
        if !seen.insert(name.clone()) {
            continue;
        }
        if let Some((prefix, _)) = split_pin(name)
            && let Some(set) = sets.get(prefix)
            && set.len() >= 3
            && run(set)
        {
            let start = set.iter().next().unwrap();
            let end = set.iter().next_back().unwrap();
            items.push(format!("`{prefix}{start}` to `{prefix}{end}`"));
            for n in set {
                seen.insert(format!("{prefix}{n}"));
            }
            continue;
        }
        items.push(format!("`{name}`"));
    }
    join_and(&items)
}

fn split_pin(name: &str) -> Option<(&str, u32)> {
    let at = name.find(|c: char| c.is_ascii_digit())?;
    let (prefix, rest) = name.split_at(at);
    if prefix.is_empty() || !prefix.chars().all(|c| c.is_ascii_uppercase()) {
        return None;
    }
    rest.parse().ok().map(|n| (prefix, n))
}

fn run(set: &BTreeSet<u32>) -> bool {
    let (Some(&first), Some(&last)) = (set.iter().next(), set.iter().next_back()) else {
        return false;
    };
    last - first + 1 == set.len() as u32
}

fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [a, b] => format!("{a} and {b}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

/// The parts a scene uses directly, in full, then the parts only other parts are built from.
fn parts(catalog: &[Card]) -> String {
    let (top, inner): (Vec<&Card>, Vec<&Card>) =
        catalog.iter().partition(|part| part.used_by.is_empty());
    let mut out = String::new();
    for part in top {
        out.push_str(&format!("### `{}`: {}\n\n", part.name, part.title));
        if let Some(summary) = &part.description {
            out.push_str(&format!("{summary}\n\n"));
        }
        let pins: Vec<String> = part.pins.iter().map(|pin| pin.name.clone()).collect();
        out.push_str(&format!("- Pins: {}.\n", quoted(&pins)));
        for param in &part.params {
            out.push_str(&format!("- `{}`: {}.\n", param.name, param_sentence(param)));
        }
        if let Some(on) = &part.on {
            out.push_str(&format!(
                "- It reads `on` while `{}` is above {}.\n",
                on.of,
                si_amount(on.above, &on.unit)
            ));
        }
        let warns: Vec<String> = part
            .ratings
            .iter()
            .filter_map(|rating| {
                let value = rating.value?;
                let unit = rating.unit.as_deref()?;
                Some(format!(
                    "`rating.{}` above {}",
                    rating.name,
                    si_amount(value, unit)
                ))
            })
            .collect();
        if !warns.is_empty() {
            out.push_str(&format!("- It warns {}.\n", warns.join(" and ")));
        }
        if part.pressable {
            out.push_str("- `press` and `release` close and open it.\n");
        }
        out.push('\n');
    }
    let inner: Vec<String> = inner
        .iter()
        .map(|p| format!("`{}` ({})", p.name, p.title))
        .collect();
    out.push_str(&format!(
        "The catalog also holds the parts the boards above are built from: {}. `orbe add` copies them with the board. Each one's pins and params are in its `parts/<id>/part.jsonc`.",
        inner.join(", ")
    ));
    out
}

fn quoted(names: &[String]) -> String {
    names
        .iter()
        .map(|n| format!("`{n}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use clap::FromArgMatches;

    use super::*;
    use crate::{Cli, cli};

    /// Each `orbe …` the skills show: the `$ orbe` lines of a `console` block, and every
    /// inline `orbe …` span. A line with a placeholder, such as `<id>`, is a usage form.
    fn command_lines(text: &str) -> Vec<String> {
        let mut lines = Vec::new();
        let mut block: Option<&str> = None;
        for line in text.lines() {
            if let Some(info) = line.strip_prefix("```") {
                block = if block.is_some() { None } else { Some(info) };
                continue;
            }
            match block {
                Some("console") => {
                    if let Some(command) = line.strip_prefix("$ orbe ") {
                        lines.push(command.to_string());
                    }
                }
                Some(_) => {}
                None => lines.extend(
                    line.split('`')
                        .skip(1)
                        .step_by(2)
                        .filter_map(|span| span.strip_prefix("orbe "))
                        .map(str::to_string),
                ),
            }
        }
        lines
    }

    /// Splits a line as `sh` does: `'…'` keeps every character, and `"…"` and a bare `\`
    /// escape the next one. An unquoted `<`, `>`, `|`, `;`, `&`, `(`, `)`, `$` or backquote
    /// is refused, since `sh` reads it as a redirect, an operator or an expansion, and so is a
    /// word that starts with `#`, which `sh` reads as a comment.
    fn shell_words(line: &str) -> Result<Vec<String>, String> {
        let unclosed = || format!("an unclosed quote in `{line}`");
        let mut words = Vec::new();
        let mut word: Option<String> = None;
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            match c {
                ' ' | '\t' => words.extend(word.take()),
                '\'' => {
                    let word = word.get_or_insert_default();
                    loop {
                        match chars.next().ok_or_else(unclosed)? {
                            '\'' => break,
                            c => word.push(c),
                        }
                    }
                }
                '"' => {
                    let word = word.get_or_insert_default();
                    loop {
                        match chars.next().ok_or_else(unclosed)? {
                            '"' => break,
                            '\\' => match chars.next().ok_or_else(unclosed)? {
                                c @ ('"' | '\\' | '$' | '`') => word.push(c),
                                c => word.extend(['\\', c]),
                            },
                            c @ ('$' | '`') => {
                                return Err(format!("`sh` expands the `{c}` in `{line}`"));
                            }
                            c => word.push(c),
                        }
                    }
                }
                '\\' => word
                    .get_or_insert_default()
                    .push(chars.next().ok_or_else(unclosed)?),
                '<' | '>' | '|' | ';' | '&' | '(' | ')' | '$' | '`' => {
                    return Err(format!(
                        "`sh` reads the unquoted `{c}` in `{line}` as a redirect, an operator or an expansion"
                    ));
                }
                '#' if word.is_none() => {
                    return Err(format!("`sh` reads the `#` in `{line}` as a comment"));
                }
                c => word.get_or_insert_default().push(c),
            }
        }
        words.extend(word);
        Ok(words)
    }

    /// A usage form, such as `add <id>…` or `step --until <condition>`, names a placeholder.
    fn is_usage(line: &str) -> bool {
        line.contains('…')
            || line.split('<').skip(1).any(|rest| {
                rest.split_once('>').is_some_and(|(name, _)| {
                    name.starts_with(|c: char| c.is_ascii_lowercase())
                        && name
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || " -_".contains(c))
                })
            })
    }

    fn check_line(line: &str) {
        if is_usage(line) {
            let words: Vec<&str> = line.split(' ').collect();
            let name = words[0];
            assert!(
                name.starts_with('<')
                    || orbe_session::spec(name).is_some()
                    || cli().find_subcommand(name).is_some(),
                "`orbe {line}` names no command"
            );
            if let Some(known) = cli().find_subcommand(name)
                && let Some(sub) = words.get(1).filter(|w| !w.starts_with(['<', '-', '[']))
            {
                assert!(
                    known.has_subcommands() == known.find_subcommand(sub).is_some(),
                    "`orbe {line}` names no command"
                );
            }
            return;
        }
        let words = shell_words(line).unwrap_or_else(|error| panic!("{error}"));
        let usage = words.len() == 1;
        if let Some(spec) = orbe_session::spec(&words[0]) {
            if usage {
                return;
            }
            let mut rest = Vec::new();
            let mut args = words[1..].iter();
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--json" => {}
                    "--all" if spec.name == "close" => {}
                    "--headed" | "--no-browser" if spec.name == "open" => {}
                    "--session" => {
                        args.next().expect("`--session` takes a name");
                    }
                    _ => rest.push(arg.clone()),
                }
            }
            if let Err(error) = spec.command(&rest, Path::new(".")) {
                panic!("`orbe {line}` does not parse: {error}");
            }
            return;
        }
        assert!(
            cli().find_subcommand(&words[0]).is_some(),
            "`orbe {line}` names no command"
        );
        if usage {
            return;
        }
        let all = std::iter::once("orbe").chain(words.iter().map(String::as_str));
        let parsed = cli()
            .try_get_matches_from(all)
            .and_then(|m| Cli::from_arg_matches(&m));
        if let Err(error) = parsed {
            panic!("`orbe {line}` does not parse: {error}");
        }
        if let ["skills", "get", name, ..] =
            &words.iter().map(String::as_str).collect::<Vec<_>>()[..]
        {
            assert!(find(name).is_some(), "`orbe {line}` names no skill");
        }
    }

    #[test]
    fn every_orbe_command_in_the_skills_and_the_stub_parses() {
        let stub = include_str!("../../../skills/orbe/SKILL.md");
        let mut seen = 0;
        for text in SKILLS
            .iter()
            .map(|skill| skill.text().unwrap())
            .chain([stub.to_string()])
        {
            for line in command_lines(&text) {
                check_line(&line);
                seen += 1;
            }
        }
        assert!(seen > 40, "only {seen} command lines found");
    }

    #[test]
    fn a_line_that_is_not_a_command_is_caught() {
        for line in [
            "opne",
            "step 2h",
            "get led1",
            "skills get nope",
            "add --frob",
            "expect @led1.i > 5mA",
            "expect serial contains \"$(printf x)\"",
            "skills get core | less",
            "expect serial contains #ready",
        ] {
            let caught = std::panic::catch_unwind(|| check_line(line));
            assert!(caught.is_err(), "`orbe {line}` passed");
        }
    }

    const HOUR_PS: u64 = 3_600_000_000_000_000;

    #[test]
    fn the_limits_the_core_skill_states_are_this_binary_s() {
        let core = find("core").unwrap().text().unwrap();
        let says = |text: String| assert!(core.contains(&text), "core does not say `{text}`");
        says(format!("It lists at most {},", orbe_session::CHANGES));
        says(format!(
            "`snapshot` shows the last {} lines",
            orbe_session::SERIAL_LINES
        ));
        assert_eq!(orbe_scene::STOP_PS % HOUR_PS, 0);
        says(format!(
            "A session stops at {} hours of simulated time",
            orbe_scene::STOP_PS / HOUR_PS
        ));
        says(format!(
            "up to the timeout, {} by default",
            orbe_session::UNTIL_TIMEOUT
        ));
    }

    #[test]
    fn the_json_states_the_core_skill_lists_are_every_state() {
        use orbe_session::State;
        // The match stops the build when a state is added, until it is listed here.
        let named = |state: State| match state {
            State::Paused | State::Playing | State::Stopped | State::Faulted | State::Closed => {
                format!("`{}`", state.as_str())
            }
        };
        let states = [
            State::Paused,
            State::Playing,
            State::Stopped,
            State::Faulted,
            State::Closed,
        ]
        .map(named);
        let listed = format!(
            "{}, {}, {}, {} or {}",
            states[0], states[1], states[2], states[3], states[4]
        );
        let core = find("core").unwrap().text().unwrap();
        assert!(core.contains(&listed), "core does not list {listed}");
    }

    #[cfg(unix)]
    #[test]
    fn every_code_in_the_core_error_table_is_one_orbe_reports() {
        use orbe_scene::AddCode as Add;
        use orbe_scene::Code as Scene;
        use orbe_session::Code as Live;
        let live = [
            Live::Usage,
            Live::IoError,
            Live::NoSession,
            Live::SceneInvalid,
            Live::PartMissing,
            Live::FirmwareMissing,
            Live::UnknownRef,
            Live::NoReading,
            Live::ExpectFailed,
            Live::UntilTimeout,
            Live::Faulted,
            Live::Ended,
            Live::NotSettable,
            Live::WrongSetting,
            Live::Value,
            Live::PastStop,
            Live::NoChange,
            Live::Cancelled,
            Live::SessionCrashed,
        ];
        let scene = [
            Scene::Syntax,
            Scene::Format,
            Scene::Name,
            Scene::Duplicate,
            Scene::Type,
            Scene::Role,
            Scene::UnknownRef,
            Scene::UnknownPin,
            Scene::Param,
            Scene::Layout,
            Scene::Wires,
            Scene::NotSimulated,
            Scene::Ground,
            Scene::Cycle,
            Scene::NameTooLong,
            Scene::PartMissing,
            Scene::FirmwareMissing,
            Scene::PartModified,
            Scene::Io,
        ];
        // The match stops the build when an `orbe add` code is added, until it is listed here.
        let add = |code: Add| match code {
            Add::Busy
            | Add::Unreadable
            | Add::NotAProject
            | Add::SceneExists
            | Add::FileExists
            | Add::LockSyntax
            | Add::Registry
            | Add::UnknownPart
            | Add::UnknownVersion
            | Add::PartConflict
            | Add::PartDiffers
            | Add::PartModified
            | Add::Symlink => code.as_str(),
        };
        let added = [
            Add::Busy,
            Add::Unreadable,
            Add::NotAProject,
            Add::SceneExists,
            Add::FileExists,
            Add::Symlink,
            Add::LockSyntax,
            Add::Registry,
            Add::UnknownPart,
            Add::UnknownVersion,
            Add::PartConflict,
            Add::PartDiffers,
            Add::PartModified,
        ];
        let known: Vec<&str> = live
            .iter()
            .map(|code| code.as_str())
            .chain(scene.into_iter().map(|code| code.as_str()))
            .chain(added.into_iter().map(add))
            .collect();
        let core = find("core").unwrap().text().unwrap();
        let table = table_in(&core, "## Errors");
        assert!(table.len() > 10, "{} rows", table.len());
        let mut listed = std::collections::BTreeSet::new();
        for row in table {
            let codes = row.split('|').nth(1).unwrap();
            for code in codes.split('`').skip(1).step_by(2) {
                assert!(known.contains(&code), "orbe reports no `{code}`");
                assert!(listed.insert(code), "`{code}` has two rows");
            }
        }
    }

    /// The rows of the first table under `heading`.
    fn table_in<'t>(text: &'t str, heading: &str) -> Vec<&'t str> {
        text.lines()
            .skip_while(|l| *l != heading)
            .skip_while(|l| !l.starts_with("| Code |"))
            .skip(2)
            .take_while(|l| l.starts_with('|'))
            .collect()
    }

    #[test]
    fn the_core_warnings_table_has_one_row_for_each_lint_rule() {
        use orbe_scene::{Code, Lint};
        // The match stops the build when a rule is added, until it is listed here.
        let code = |lint: Lint| match lint {
            Lint::Unplaced
            | Lint::Unconnected
            | Lint::GroundUnnamed
            | Lint::Rotation
            | Lint::NoModel
            | Lint::Floating
            | Lint::LedNoResistor
            | Lint::Shorted => Code::Lint(lint).as_str(),
        };
        let rules: std::collections::BTreeSet<&str> = [
            Lint::Unplaced,
            Lint::Unconnected,
            Lint::GroundUnnamed,
            Lint::Rotation,
            Lint::NoModel,
            Lint::Floating,
            Lint::LedNoResistor,
            Lint::Shorted,
        ]
        .map(code)
        .into();
        let core = find("core").unwrap().text().unwrap();
        let listed: Vec<&str> = table_in(&core, "## Warnings")
            .into_iter()
            .filter_map(|row| row.split('`').nth(1))
            .collect();
        assert_eq!(listed.len(), rules.len(), "{listed:?}");
        assert_eq!(
            listed
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>(),
            rules
        );
    }

    /// Each board pin the reader leaves unsimulated, with the reason it gives, is in that
    /// board's own list. The chip's catalog number and the L LED are too.
    #[test]
    fn the_arduino_skill_names_every_pin_a_board_leaves_unsimulated() {
        let skill = find("arduino").unwrap();
        let text = skill.text().unwrap();
        assert!(!text.contains("<!-- boards"));
        let ids = marker_ids(skill.text);
        assert!(ids.len() >= 2, "{ids:?}");
        for id in &ids {
            let block = section(&text, id);
            let bullets = bullet_list(block);
            assert!(
                bullets.iter().any(|line| line.contains("These pins work")),
                "{id} list:\n{}",
                bullets.join("\n")
            );
            assert!(
                bullets
                    .iter()
                    .any(|line| line.contains("`led_l`") && line.contains("`r_l`")),
                "{id} omits the L LED from its list"
            );
            let view = drawn(id);
            let pins: Vec<_> = view
                .not_simulated
                .iter()
                .filter(|pin| pin.r#ref.chars().filter(|c| *c == '.').count() == 1)
                .collect();
            assert!(!pins.is_empty(), "{id} reports no unsimulated pin");
            let missing: Vec<String> = pins
                .iter()
                .filter(|pin| {
                    let name = pin.r#ref.rsplit('.').next().unwrap();
                    !bullets
                        .iter()
                        .any(|line| line.contains(&format!("`{name}`")) && line.contains(&pin.why))
                })
                .map(|pin| format!("{}: {}", pin.r#ref, pin.why))
                .collect();
            assert!(missing.is_empty(), "{id} omits {missing:#?}");
            let board = orbe_scene::card(id).unwrap();
            let chip = board
                .uses
                .keys()
                .find(|used| used.starts_with("atmega328p"))
                .unwrap();
            let number = orbe_scene::card(chip).unwrap().real.number.unwrap();
            assert!(block.contains(&number), "{id} omits {number}");
        }
    }

    #[test]
    fn a_board_the_marker_names_that_is_not_an_avr_board_is_an_error() {
        let err = fill_boards("<!-- boards resistor-0603 -->\n").unwrap_err();
        assert!(err.contains("resistor-0603"), "{err}");
    }

    #[test]
    fn a_board_the_marker_names_that_does_not_read_is_an_error() {
        let err = fill_boards("<!-- boards not-a-board -->\n").unwrap_err();
        assert!(err.contains("not-a-board"), "{err}");
    }

    #[test]
    fn a_boards_marker_that_names_nothing_is_an_error() {
        let err = fill_boards("<!-- boards -->\n").unwrap_err();
        assert!(err.contains("names no board"), "{err}");
    }

    #[test]
    fn a_boards_marker_that_does_not_close_is_an_error() {
        let err = fill_boards("<!-- boards nano-board").unwrap_err();
        assert!(err.contains("does not close"), "{err}");
    }

    fn marker_ids(raw: &str) -> Vec<String> {
        let start = raw
            .find("<!-- boards")
            .expect("the skill has a boards marker");
        let tail = &raw[start..];
        let close = tail.find("-->").expect("the boards marker closes");
        tail["<!-- boards".len()..close]
            .split_whitespace()
            .map(str::to_string)
            .collect()
    }

    fn section<'a>(text: &'a str, id: &str) -> &'a str {
        let head = format!("### `{id}`\n");
        let start = text
            .find(&head)
            .unwrap_or_else(|| panic!("{id} has no section"));
        let from = &text[start..];
        let end = from[head.len()..]
            .find("\n### ")
            .map(|at| head.len() + at)
            .unwrap_or(from.len());
        &from[..end]
    }

    fn bullet_list(section: &str) -> Vec<&str> {
        section
            .lines()
            .skip_while(|line| !line.starts_with("- "))
            .take_while(|line| line.starts_with("- "))
            .collect()
    }

    fn drawn(id: &str) -> orbe_scene::View {
        orbe_scene::view(&orbe_scene::one_part(id), Path::new("scene.jsonc")).unwrap_or_else(
            |found| {
                panic!(
                    "{id}: {}",
                    found
                        .iter()
                        .map(|d| d.to_string())
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            },
        )
    }

    #[test]
    fn the_core_skill_lists_every_catalog_part() {
        let text = find("core").unwrap().text().unwrap();
        assert!(!text.contains(PARTS));
        for part in catalog_cards() {
            assert!(text.contains(&format!("`{}`", part.name)), "{}", part.name);
        }
        assert!(text.contains("### `nano-board`: Nano (CH340 clone)"));
        assert!(text.contains("- `resistance`: a value in ohm above 0, required."));
        assert!(text.contains("- `position`: a number above 0 and below 1, 0.5 by default."));
        assert!(text.contains("- It reads `on` while `i` is above 1 mA."));
        assert!(
            text.contains(
                "- It warns `rating.current` above 30 mA and `rating.power` above 75 mW."
            )
        );
        assert!(text.contains("- It warns `rating.power` above 250 mW."));
    }
}
