//! The live command table. Each command is one typed entry, parsed from the words a person or
//! a test file writes.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use orbe_engine::world::Quantity;

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
    Set(WorldSet),
    /// A part's live param, such as `set @pot1.position 0.8`.
    SetParam(ParamSet),
    Get(Ref),
    /// A world quantity the scene declares, such as `light`, read by `get light` or `get @world.light`.
    GetWorld(Quantity),
    Step(Step),
    Play,
    Pause,
    Snapshot,
    Expect(Condition),
}

/// A world quantity's new value, in its unit, inside its range.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldSet {
    pub quantity: Quantity,
    pub value: f64,
}

/// A part's live param, named as `@pot1.position`, and its new value in the param's unit. The
/// open scene decides whether the param is live and whether the value is in its range.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParamSet {
    pub target: Ref,
    pub value: f64,
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
        name: "set",
        usage: "set <quantity> <value> | set <ref> <value>",
        about: "Sets a world quantity the scene declares, from now on: temperature in degC, humidity in %RH or light in lx, such as `set light 50`. Or turns a part's live param, such as a potentiometer's position, as in `set @pot1.position 0.8`.",
        parse: |args, _| parse_set(args),
    },
    Spec {
        name: "get",
        usage: "get <ref> | get <quantity>",
        about: "Reads one part, pin or quantity now.",
        parse: |args, _| parse_get(args),
    },
    Spec {
        name: "step",
        usage: "step <duration> | step --until <condition> [--timeout <duration>]",
        about: "Advances time and reports what changed. What `play` held is listed first, then what this step did.",
        parse: |args, _| parse_step(args).map(Command::Step),
    },
    Spec {
        name: "play",
        usage: "play",
        about: "Runs time at real speed and returns at once. Time runs until `pause`, a `step` that runs, the session's stop or a fault. A refused `step`, such as one past the stop, leaves it running. The next `pause` or `step` lists what changed while it ran.",
        parse: |args, _| none(args, Command::Play),
    },
    Spec {
        name: "pause",
        usage: "pause",
        about: "Stops `play`, so time moves only in `step` again. It lists what changed while `play` ran, and says time is stopped even when nothing changed.",
        parse: |args, _| none(args, Command::Pause),
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
        parse: |args, _| {
            condition_words(args).and_then(|words| Condition::parse(&words).map(Command::Expect))
        },
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

fn parse_get(args: &[String]) -> Result<Command, String> {
    match args {
        [one] => {
            if let Some(quantity) = Quantity::parse(one) {
                return Ok(Command::GetWorld(quantity));
            }
            if let Some(name) = one.strip_prefix("@world.")
                && let Some(quantity) = Quantity::parse(name)
            {
                return Ok(Command::GetWorld(quantity));
            }
            Ref::parse(one).map(Command::Get)
        }
        _ => Err("takes one ref, such as `@btn1`, or a world quantity, such as `light`".into()),
    }
}

fn one_ref(args: &[String]) -> Result<Ref, String> {
    match args {
        [r] => Ref::parse(r),
        _ => Err("takes one ref, such as `@btn1`".into()),
    }
}

fn parse_set(args: &[String]) -> Result<Command, String> {
    let names = || {
        let names: Vec<&str> = Quantity::ALL.iter().map(|q| q.name()).collect();
        names.join(", ")
    };
    let [name, value] = args else {
        return Err(
            "takes a quantity and a value, such as `light 50`, or a part's param and a value, such as `@pot1.position 0.8`"
                .into(),
        );
    };
    let world = name.strip_prefix("@world.");
    if let Some(r) = name.strip_prefix('@').filter(|_| world.is_none()) {
        let target = Ref::parse(name)?;
        let value = value
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| {
                format!("`{value}` is not a number; write the param's value, such as 0.5")
            })?;
        if !r.contains('.') {
            return Err(format!(
                "`{name}` is a part; name its param, such as `{name}.position`"
            ));
        }
        return Ok(Command::SetParam(ParamSet { target, value }));
    }
    let name = world.unwrap_or(name);
    let quantity = Quantity::parse(name).ok_or_else(|| {
        format!(
            "`{name}` is not a world quantity; the quantities are {}",
            names()
        )
    })?;
    let row = quantity.row();
    let value = value
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| {
            format!(
                "`{value}` is not a number; write {} in {}, such as {}",
                row.name, row.unit, row.default
            )
        })?;
    if !quantity.holds(value) {
        return Err(format!(
            "{} is from {} to {} {}, not {value}",
            row.name, row.min, row.max, row.unit
        ));
    }
    Ok(Command::Set(WorldSet { quantity, value }))
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
                condition: Condition::parse(&condition_words(words)?)?,
                timeout,
            })
        }
        [duration] => Ok(Step::For(Duration::parse(duration)?)),
        _ => Err("takes a duration, such as `100ms`, or `--until <condition>`".into()),
    }
}

/// A condition that arrived as one argument, such as `'@led1 rgb(255,0,0)'` from a shell,
/// splits the way a test line does. Several arguments are already split.
fn condition_words(args: &[String]) -> Result<Vec<String>, String> {
    match args {
        [one] if one.contains(char::is_whitespace) => words(one),
        _ => Ok(args.to_vec()),
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
