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
        if !part.ratings.is_empty() {
            let warns: Vec<String> = part
                .ratings
                .iter()
                .map(|(name, above)| format!("`rating.{name}` above {above}"))
                .collect();
            out.push_str(&format!("- It warns {}.\n", warns.join(" and ")));
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
        Takes::Number { unit, above, below } => {
            let mut text = match unit.as_str() {
                "1" => "a number".to_string(),
                unit => format!("a value in {unit}"),
            };
            if let Some(above) = above {
                text.push_str(&format!(" above {above}"));
            }
            if let Some(below) = below {
                let joint = if above.is_some() { " and" } else { "" };
                text.push_str(&format!("{joint} below {below}"));
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
        let core = find("core").unwrap().text();
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
        let core = find("core").unwrap().text();
        assert!(core.contains(&listed), "core does not list {listed}");
    }

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
            | Add::LockSyntax
            | Add::Registry
            | Add::UnknownPart
            | Add::UnknownVersion
            | Add::PartConflict
            | Add::PartDiffers
            | Add::PartModified => code.as_str(),
        };
        let added = [
            Add::Busy,
            Add::Unreadable,
            Add::NotAProject,
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
        let core = find("core").unwrap().text();
        let table: Vec<&str> = core
            .lines()
            .skip_while(|l| !l.starts_with("| Code |"))
            .skip(2)
            .take_while(|l| l.starts_with('|'))
            .collect();
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

    #[test]
    fn the_core_skill_lists_every_catalog_part() {
        let text = find("core").unwrap().text();
        assert!(!text.contains(PARTS));
        for part in catalog_parts() {
            assert!(text.contains(&format!("`{}`", part.id)), "{}", part.id);
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
