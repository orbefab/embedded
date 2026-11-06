//! The skills this binary serves. Each is a markdown file compiled in. The `core` skill's
//! parts section is rendered from the built-in catalog when it is printed, so it lists the
//! parts `orbe add` can copy.

use orbe_scene::{ParamDefault, ParamSummary, PartSummary, Takes, catalog_parts};

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
        about: "Building Arduino Nano firmware with arduino-cli and naming it in the scene.",
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
    pub fn text(&self) -> String {
        self.text.replace(PARTS, &parts(&catalog_parts()))
    }
}

/// The parts a scene uses directly, in full, then the parts only other parts are built from.
fn parts(catalog: &[PartSummary]) -> String {
    let (top, inner): (Vec<&PartSummary>, Vec<&PartSummary>) =
        catalog.iter().partition(|p| p.used_by.is_empty());
    let mut out = String::new();
    for part in top {
        out.push_str(&format!("### `{}`: {}\n\n", part.id, part.title));
        if let Some(summary) = &part.summary {
            out.push_str(&format!("{summary}\n\n"));
        }
        out.push_str(&format!("- Pins: {}.\n", quoted(&part.pins)));
        for param in &part.params {
            out.push_str(&format!("- `{}`: {}.\n", param.name, param_text(param)));
        }
        if let Some((of, above)) = &part.on {
            out.push_str(&format!("- It reads `on` while `{of}` is above {above}.\n"));
        }
        if part.pressable {
            out.push_str("- `press` and `release` close and open it.\n");
        }
        out.push('\n');
    }
    let inner: Vec<String> = inner
        .iter()
        .map(|p| format!("`{}` ({})", p.id, p.title))
        .collect();
    out.push_str(&format!(
        "The catalog also holds the parts the boards above are built from: {}. `orbe add` copies them with the board. Each one's pins and params are in its `parts/<id>/part.jsonc`.",
        inner.join(", ")
    ));
    out
}

fn param_text(param: &ParamSummary) -> String {
    let takes = match &param.takes {
        Takes::Number { unit, above } => {
            let mut text = match unit.as_str() {
                "1" => "a number".to_string(),
                unit => format!("a value in {unit}"),
            };
            if let Some(above) = above {
                text.push_str(&format!(" above {above}"));
            }
            text
        }
        Takes::Choice(choices) => format!("one of {}", quoted(choices)),
        Takes::File { ext: Some(ext) } => format!("the path of a `.{ext}` file"),
        Takes::File { ext: None } => "the path of a file".to_string(),
    };
    match &param.default {
        ParamDefault::Required => format!("{takes}, required"),
        ParamDefault::Unset => format!("{takes}, not set by default"),
        ParamDefault::Value(value) => format!("{takes}, {value} by default"),
    }
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

    /// Splits a shell line at spaces, keeping `'…'` and `"…"` whole.
    fn shell_words(line: &str) -> Vec<String> {
        let mut words = Vec::new();
        let mut word: Option<String> = None;
        let mut quote = None;
        for c in line.chars() {
            match (quote, c) {
                (None, '\'' | '"') => {
                    quote = Some(c);
                    word.get_or_insert_default();
                }
                (Some(q), _) if c == q => quote = None,
                (None, ' ') => words.extend(word.take()),
                _ => word.get_or_insert_default().push(c),
            }
        }
        assert!(quote.is_none(), "an unclosed quote in `{line}`");
        words.extend(word);
        words
    }

    fn check_line(line: &str) {
        let words = shell_words(line);
        let usage = line.contains('<') || line.contains('…') || words.len() == 1;
        if words[0].starts_with('<') {
            return;
        }
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
        let commands = cli();
        let known = commands
            .find_subcommand(&words[0])
            .unwrap_or_else(|| panic!("`orbe {line}` names no command"));
        if usage {
            if let Some(sub) = words.get(1).filter(|w| !w.starts_with(['<', '-'])) {
                assert!(
                    known.has_subcommands() == known.find_subcommand(sub).is_some(),
                    "`orbe {line}` names no command"
                );
            }
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
        for text in SKILLS.iter().map(Skill::text).chain([stub.to_string()]) {
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
        ] {
            let caught = std::panic::catch_unwind(|| check_line(line));
            assert!(caught.is_err(), "`orbe {line}` passed");
        }
    }

    #[test]
    fn the_core_skill_lists_every_catalog_part() {
        let text = find("core").unwrap().text();
        assert!(!text.contains(PARTS));
        for part in catalog_parts() {
            assert!(text.contains(&format!("`{}`", part.id)), "{}", part.id);
        }
        assert!(text.contains("### `nano-board`: Nano (CH340 clone)"));
        assert!(text.contains("- `resistance`: a value in ohm above 0, required."));
        assert!(text.contains("- It reads `on` while `i` is above 1 mA."));
    }
}
