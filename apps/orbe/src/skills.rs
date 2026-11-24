//! The skills this binary serves. Each is a markdown file compiled in.

pub struct Skill {
    pub name: &'static str,
    pub about: &'static str,
    text: &'static str,
}

pub const SKILLS: &[Skill] = &[
    Skill {
        name: "core",
        about: "Read first: the rules, the loop, one example, troubleshooting, and which topic to load.",
        text: include_str!("../skills/core.md"),
    },
    Skill {
        name: "arduino",
        about: "Load when building Nano or Uno firmware, or when a chip warning names a pin.",
        text: include_str!("../skills/arduino.md"),
    },
    Skill {
        name: "esp32",
        about: "Load when building ESP32-S3 firmware for an esp32s3-devkitc-1.",
        text: include_str!("../skills/esp32.md"),
    },
    Skill {
        name: "parts",
        about: "Load when keeping a part copy, a version, a Block, a buy link, or an addressable LED.",
        text: include_str!("../skills/parts.md"),
    },
    Skill {
        name: "scenes",
        about: "Load when writing a scene key, a part name, a param, or world.",
        text: include_str!("../skills/scenes.md"),
    },
    Skill {
        name: "breadboards",
        about: "Load when seating a part on a breadboard, or a hole or a strip is wrong.",
        text: include_str!("../skills/breadboards.md"),
    },
    Skill {
        name: "sessions-and-time",
        about: "Load when a step, a set or a bounce lands, or a session's name or stop matters.",
        text: include_str!("../skills/sessions-and-time.md"),
    },
    Skill {
        name: "tests",
        about: "Load when a test file does more than the example in core.",
        text: include_str!("../skills/tests.md"),
    },
    Skill {
        name: "ratings",
        about: "Load when a warning names a rating, or an LED is lit, dim or off.",
        text: include_str!("../skills/ratings.md"),
    },
    Skill {
        name: "errors",
        about: "Load when an error code is not in core's troubleshooting.",
        text: include_str!("../skills/errors.md"),
    },
    Skill {
        name: "page",
        about: "Load when someone watches the 3D page, or a knob turns.",
        text: include_str!("../skills/page.md"),
    },
    Skill {
        name: "not-simulated",
        about: "Load when a pin or a feature does nothing, or a wire is not_simulated.",
        text: include_str!("../skills/not-simulated.md"),
    },
];

pub fn find(name: &str) -> Option<&'static Skill> {
    SKILLS.iter().find(|s| s.name == name)
}

pub fn names() -> String {
    SKILLS.iter().map(|s| s.name).collect::<Vec<_>>().join(", ")
}

impl Skill {
    pub fn text(&self) -> Result<String, String> {
        Ok(self.text.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use clap::FromArgMatches;

    use super::*;
    use crate::{Cli, cli};

    /// Each `orbe …` the skills show: `$ orbe` lines in a console block, `orbe` lines in a
    /// bash or sh block, and every inline `orbe …` span. A `#` line in a bash or sh block is
    /// a comment. A line with a placeholder, such as `<id>`, is a usage form.
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
                Some("bash" | "sh") => {
                    let trimmed = line.trim();
                    if !trimmed.is_empty()
                        && !trimmed.starts_with('#')
                        && let Some(command) = trimmed
                            .strip_prefix("$ orbe ")
                            .or_else(|| trimmed.strip_prefix("orbe "))
                    {
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

    #[cfg(unix)]
    #[test]
    fn every_code_in_the_error_tables_is_one_orbe_reports() {
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
            Live::VersionMismatch,
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
            Scene::PartFormat,
            Scene::EngineModel,
            Scene::Io,
            Scene::World,
        ];
        let added = [
            "not_a_project",
            "scene_exists",
            "file_exists",
            "symlink",
            "registry",
            "unknown_part",
            "part_exists",
            "net.short",
        ];
        let known: Vec<&str> = live
            .iter()
            .map(|code| code.as_str())
            .chain(scene.into_iter().map(|code| code.as_str()))
            .chain(added)
            .collect();
        let texts: Vec<String> = SKILLS.iter().map(|skill| skill.text().unwrap()).collect();
        let mut listed = std::collections::BTreeSet::new();
        let mut tables = 0;
        for text in &texts {
            if !text.contains("## Errors") {
                continue;
            }
            let table = table_in(text, "## Errors");
            assert!(table.len() > 5, "{table:?}");
            tables += 1;
            for row in table {
                let codes = row.split('|').nth(1).unwrap();
                for code in codes.split('`').skip(1).step_by(2) {
                    assert!(known.contains(&code), "orbe reports no `{code}`");
                    assert!(listed.insert(code), "`{code}` has two rows");
                }
            }
        }
        assert!(tables >= 2, "expected the parts table and the errors table");
    }

    /// Each row of the world table in the scenes skill is one `orbe set` reads: its default and
    /// both ends set, and a value past either end is refused with the row's range and unit.
    #[test]
    fn the_scenes_world_table_is_the_one_set_reads() {
        let scenes = find("scenes").unwrap().text().unwrap();
        let rows: Vec<Vec<&str>> = scenes
            .lines()
            .map(str::trim_start)
            .skip_while(|l| !l.starts_with("| Quantity |"))
            .skip(2)
            .take_while(|l| l.starts_with('|'))
            .map(|l| {
                l.split('|')
                    .map(str::trim)
                    .filter(|c| !c.is_empty())
                    .collect()
            })
            .collect();
        let names: Vec<&str> = rows.iter().map(|r| r[0].trim_matches('`')).collect();
        assert_eq!(names, ["temperature", "humidity", "light"]);
        let set = orbe_session::spec("set").unwrap();
        let parse = |words: &[&str]| {
            let words: Vec<String> = words.iter().map(|w| w.to_string()).collect();
            set.command(&words, Path::new("."))
        };
        for row in &rows {
            let (name, unit, default) = (row[0].trim_matches('`'), row[1], row[3]);
            let (min, max) = row[2].split_once(" to ").unwrap();
            for value in [min, max, default] {
                parse(&[name, value]).unwrap_or_else(|e| panic!("set {name} {value}: {e}"));
            }
            let under = min.parse::<f64>().unwrap() - 1.0;
            let over = max.parse::<f64>().unwrap() + 1.0;
            for value in [under, over] {
                let error = parse(&[name, &value.to_string()]).unwrap_err();
                let says = format!("{name} is from {min} to {max} {unit}, not {value}");
                assert!(error.starts_with(&says), "{error}");
            }
        }
        let error = parse(&["lite", "50"]).unwrap_err();
        assert!(
            error.starts_with(
                "`lite` is not a world quantity; the quantities are temperature, humidity, light"
            ),
            "{error}"
        );
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
    fn the_ratings_warnings_table_has_one_row_for_each_lint_rule() {
        use orbe_scene::{Code, Lint};
        // The match stops the build when a rule is added, until it is listed here.
        let code = |lint: Lint| match lint {
            Lint::Unplaced
            | Lint::Unconnected
            | Lint::NoModel
            | Lint::Floating
            | Lint::LedNoResistor
            | Lint::Shorted
            | Lint::CoveredHole
            | Lint::Overlap => Code::Lint(lint).as_str(),
        };
        let rules: std::collections::BTreeSet<&str> = [
            Lint::Unplaced,
            Lint::Unconnected,
            Lint::NoModel,
            Lint::Floating,
            Lint::LedNoResistor,
            Lint::Shorted,
            Lint::CoveredHole,
            Lint::Overlap,
        ]
        .map(code)
        .into();
        let ratings = find("ratings").unwrap().text().unwrap();
        let listed: Vec<&str> = table_in(&ratings, "## Warnings")
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
        let ids = ["nano-board".to_string(), "uno-board".to_string()];
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
            if id == "nano-board" {
                assert!(pins.is_empty(), "{id} still leaves a pin out: {pins:?}");
            } else {
                assert!(!pins.is_empty(), "{id} reports no unsimulated pin");
            }
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
            let parts = repo_parts();
            let board = orbe_scene::card(&parts, id).unwrap();
            let chip = board
                .uses
                .keys()
                .find(|used| used.starts_with("atmega328p"))
                .unwrap();
            let number = orbe_scene::card(&parts, chip).unwrap().real.number.unwrap();
            assert!(block.contains(&number), "{id} omits {number}");
        }
    }

    fn repo_parts() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../parts")
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
        orbe_scene::view(
            &orbe_scene::one_part(&repo_parts(), id),
            Path::new("scene.jsonc"),
        )
        .unwrap_or_else(|found| {
            panic!(
                "{id}: {}",
                found
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        })
    }

    /// Core points to `orbe search` and `orbe part` for parts, and pastes no catalog, which
    /// would push it past what an agent reads in one output.
    #[test]
    fn the_core_skill_pastes_no_catalog() {
        let core = find("core").unwrap().text().unwrap();
        assert!(!core.contains("<!--"), "core holds a marker");
        assert!(!core.contains("\n### `"), "core holds a part's section");
        for line in ["`orbe search <words>`", "`orbe part <id>`"] {
            assert!(core.contains(line), "core does not name {line}");
        }
    }
}
