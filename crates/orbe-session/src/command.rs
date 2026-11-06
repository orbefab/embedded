//! The live command table. Each command is one typed entry, parsed from the words a person or
//! a test file writes.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::condition::Condition;
use crate::duration::Duration;
use crate::target::Ref;

/// A command's wire form, for a session process and its client, is its serde form.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Command {
    Open(PathBuf),
    Close,
    Press(Ref),
    Release(Ref),
    Get(Ref),
    Step(Step),
    Snapshot,
    Expect(Condition),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Step {
    For(Duration),
    Until {
        condition: Condition,
        timeout: Duration,
    },
}

/// `step --until` gives up after 10 s of simulated time unless `--timeout` says otherwise.
pub const UNTIL_TIMEOUT: Duration = Duration::from_ms(10_000);

/// One row of the table: the name, the usage line, what it does, and its parser. `root` is
/// the project folder relative paths start from.
pub struct Spec {
    pub name: &'static str,
    pub usage: &'static str,
    pub about: &'static str,
    parse: fn(&[String], &Path) -> Result<Command, String>,
}

pub const COMMANDS: &[Spec] = &[
    Spec {
        name: "open",
        usage: "open [<scene>]",
        about: "Checks the scene and opens it at t = 0, paused. The scene is scene.jsonc by default.",
        parse: |args, root| match args {
            [] => Ok(Command::Open(root.join("scene.jsonc"))),
            [dashes, scene] if dashes == "--" => Ok(Command::Open(root.join(scene))),
            [flag] if flag.starts_with('-') => Err(format!(
                "`{flag}` is a flag, not a scene; write a scene named so as `./{flag}`"
            )),
            [scene] => Ok(Command::Open(root.join(scene))),
            _ => Err("takes at most one scene".into()),
        },
    },
    Spec {
        name: "close",
        usage: "close",
        about: "Ends the session.",
        parse: |args, _| none(args, Command::Close),
    },
    Spec {
        name: "press",
        usage: "press <ref>",
        about: "Closes a switch, named by its part, such as @btn1.",
        parse: |args, _| one_ref(args).map(Command::Press),
    },
    Spec {
        name: "release",
        usage: "release <ref>",
        about: "Opens a switch, named by its part, such as @btn1.",
        parse: |args, _| one_ref(args).map(Command::Release),
    },
    Spec {
        name: "get",
        usage: "get <ref>",
        about: "Reads one part, pin or quantity now.",
        parse: |args, _| one_ref(args).map(Command::Get),
    },
    Spec {
        name: "step",
        usage: "step <duration> | step --until <condition> [--timeout <duration>]",
        about: "Advances time and reports what changed.",
        parse: |args, _| parse_step(args).map(Command::Step),
    },
    Spec {
        name: "snapshot",
        usage: "snapshot",
        about: "Shows each part's state, serial and diagnostics now.",
        parse: |args, _| none(args, Command::Snapshot),
    },
    Spec {
        name: "expect",
        usage: "expect <condition>",
        about: "Checks a condition now, and fails when it does not hold.",
        parse: |args, _| Condition::parse(args).map(Command::Expect),
    },
];

impl Spec {
    /// Parses the words after the command's name.
    pub fn command(&self, args: &[String], root: &Path) -> Result<Command, String> {
        (self.parse)(args, root).map_err(|reason| format!("{reason}; usage: {}", self.usage))
    }
}

pub fn spec(name: &str) -> Option<&'static Spec> {
    COMMANDS.iter().find(|s| s.name == name)
}

impl Command {
    /// Parses one line. `Ok(None)` for a blank line or a comment.
    pub fn parse(line: &str, root: &Path) -> Result<Option<Command>, String> {
        let words = words(line)?;
        let Some((name, args)) = words.split_first() else {
            return Ok(None);
        };
        let spec = spec(name).ok_or_else(|| {
            let names: Vec<&str> = COMMANDS.iter().map(|s| s.name).collect();
            format!(
                "`{name}` is not a command; the commands are {}",
                names.join(", ")
            )
        })?;
        spec.command(args, root).map(Some)
    }
}

fn none(args: &[String], command: Command) -> Result<Command, String> {
    match args {
        [] => Ok(command),
        _ => Err("takes no arguments".into()),
    }
}

fn one_ref(args: &[String]) -> Result<Ref, String> {
    match args {
        [r] => Ref::parse(r),
        _ => Err("takes one ref, such as `@btn1`".into()),
    }
}

fn parse_step(args: &[String]) -> Result<Step, String> {
    match args {
        [flag, rest @ ..] if flag == "--until" => {
            let (words, timeout) = match rest {
                [words @ .., flag, value] if flag == "--timeout" => {
                    (words, Duration::parse(value)?)
                }
                [words @ .., flag] if flag.starts_with("--timeout=") => {
                    (words, Duration::parse(&flag["--timeout=".len()..])?)
                }
                words => (words, UNTIL_TIMEOUT),
            };
            Ok(Step::Until {
                condition: Condition::parse(words)?,
                timeout,
            })
        }
        [duration] => Ok(Step::For(Duration::parse(duration)?)),
        _ => Err("takes a duration, such as `100ms`, or `--until <condition>`".into()),
    }
}

/// Splits a line at spaces. `"..."` holds spaces, with `\"` and `\\` inside, and `#` outside
/// quotes starts a comment.
fn words(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut chars = line.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        match chars.peek() {
            None | Some('#') => return Ok(words),
            Some('"') => {
                chars.next();
                let mut word = String::new();
                loop {
                    match chars.next() {
                        None => return Err("a quote is not closed".into()),
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(c @ ('"' | '\\')) => word.push(c),
                            _ => return Err("only `\\\"` and `\\\\` escape inside quotes".into()),
                        },
                        Some(c) => word.push(c),
                    }
                }
                words.push(word);
            }
            Some(_) => {
                let mut word = String::new();
                while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
                    word.push(c);
                }
                words.push(word);
            }
        }
    }
}
