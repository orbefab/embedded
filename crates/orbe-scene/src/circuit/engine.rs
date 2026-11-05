//! The engine's findings on the netlist the reader wrote, placed in the files that wrote it.

use serde_json::{Map, Value};
use sim_core::diagnostic::{Diagnostic, ScheduleRule};

use super::{Circuit, Head, SetBy, Setter, Written, setter_of};
use crate::diagnostic::{Code, Reported};
use crate::params::{dotted, within};
use crate::source::Span;
use crate::syntax::Node;
use crate::units::Unit;

/// Engine findings that repeat what the reader reports itself: names, duplicates and nets.
const REPEATS: &[&str] = &[
    "netlist.name",
    "netlist.duplicate",
    "netlist.unknown_net",
    "netlist.ground",
];

impl<'p> Circuit<'p> {
    /// An engine finding placed at the value that set the field, or else at the field in the
    /// part file's model. When `quiet`, the reader already has findings, so a finding that
    /// repeats one, or that belongs to no part, is left out. A part whose own params have a
    /// finding keeps its engine findings for the next run.
    pub(super) fn engine_finding(
        &mut self,
        diagnostic: &Diagnostic,
        value: &Value,
        origin: Option<&Written>,
        quiet: bool,
    ) {
        let code = diagnostic.code();
        if quiet && REPEATS.contains(&code) {
            return;
        }
        let message = value["message"].as_str().unwrap_or_default().to_string();
        let Some(origin) = origin else {
            if !quiet {
                self.out.push(
                    Code::Engine(code),
                    Span::start_of(self.project.scene.file),
                    message,
                    "change the value so it meets the engine's rule".into(),
                );
            }
            return;
        };
        if self.failed.contains_key(&origin.inst) {
            return;
        }
        // The fields the rule reads, the message, what it asks of a value that set one of
        // them, and the fix.
        let (fields, message, requirement, hint) = match diagnostic {
            Diagnostic::NetlistSchedule {
                part,
                field,
                event,
                rule,
                ..
            } => {
                let at = format!("{part} {field}[{event}]");
                // The kind of the object that holds the list, such as a source's waveform.
                let holder = field
                    .rsplit_once('.')
                    .map_or(String::new(), |(h, _)| format!("/{}", h.replace('.', "/")));
                let kind = origin
                    .fields
                    .pointer(&format!("{holder}/kind"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let (said, fix) = schedule(kind, *rule, &at, &message);
                let requirement = requirement(&said);
                let fields = schedule_fields(kind, field, *event, *rule);
                (fields, said, requirement, fix)
            }
            _ => {
                let fields = read_fields(value);
                match value["data"]["rule"].as_str().unwrap_or_default() {
                    "bounce_room" => {
                        let edges = origin
                            .fields
                            .pointer("/bounce/max_bounces/value")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                        let least = duration(2 * edges * sim_core::netlist::MIN_GAP_PS);
                        (
                            fields,
                            message,
                            format!("with max_bounces {edges}, it must be at least {least}"),
                            format!("write a bounce of at least {least}, or lower max_bounces"),
                        )
                    }
                    rule => {
                        let hint = if rule == "positive" {
                            "write a value greater than zero"
                        } else {
                            "change the value so it meets the engine's rule"
                        };
                        let requirement = requirement(&message);
                        (fields, message, requirement, hint.to_string())
                    }
                }
            }
        };
        let pointer = fields[0].clone();
        let setter = set_by_instance(&origin.set_by, &fields)
            .or_else(|| setter_of(&origin.set_by, &pointer))
            .or_else(|| {
                origin
                    .set_by
                    .iter()
                    .find(|(p, _)| !pointer.is_empty() && within(p, &pointer))
                    .map(|(_, s)| s)
            });
        if let Some(setter) = setter {
            self.out.push(
                Code::Param,
                setter.span,
                format!("`{}` is {}; {requirement}", setter.site, setter.shown),
                hint,
            );
            return;
        }
        let span = self.body_model(origin).at_pointer(&pointer).span;
        self.out.push(
            Code::Engine(code),
            span,
            format!("`{}`: {message}", self.insts[origin.inst].dotted()),
            hint,
        );
    }

    /// The part file's model for one engine part.
    fn body_model(&self, origin: &Written) -> &'p Node {
        let part = self.part(origin.inst).expect("an engine part has a part");
        let model = part.model.as_ref().expect("an engine part has a model");
        origin
            .body
            .as_ref()
            .and_then(|name| model.get(name))
            .unwrap_or(model)
    }

    /// The engine's schema refused a part. The finding goes to the value that set the field
    /// the engine refuses, or else to that field in the part file's model.
    pub(super) fn schema_finding(
        &mut self,
        origin: &Written,
        head: &Head,
        value: &Value,
    ) -> Reported {
        let detail = detail(value);
        let at = culprit(&origin.fields, &detail, |part| refusal(head, part));
        let parent = at.rsplit_once('/').map_or("", |(parent, _)| parent);
        let fields: Vec<String> = std::iter::once(at.clone())
            .chain(
                rule(&detail)
                    .into_iter()
                    .flat_map(|(_, _, reads, _)| reads.iter().map(|f| format!("{parent}/{f}"))),
            )
            .collect();
        let setter =
            set_by_instance(&origin.set_by, &fields).or_else(|| setter_of(&origin.set_by, &at));
        let (message, hint) = words(&detail, &at, origin.fields.pointer(&at), setter);
        let before = self.out.len();
        match setter {
            Some(setter) => self.out.push(
                Code::Param,
                setter.span,
                format!("`{}`: {message}", setter.site),
                hint,
            ),
            None => {
                let span = self.body_model(origin).at_pointer(&at).span;
                self.out.push(
                    Code::Engine("netlist.syntax"),
                    span,
                    format!("`{}`: {message}", self.insts[origin.inst].dotted()),
                    hint,
                );
            }
        }
        self.out.since(before).expect("a finding was pushed")
    }
}

/// A value an instance set, or a choice it picked, at or inside one of the fields a rule
/// reads. A finding goes there first, since that value is what broke a rule the part file
/// alone keeps.
fn set_by_instance<'s>(set_by: &'s SetBy, fields: &[String]) -> Option<&'s Setter> {
    fields.iter().find_map(|f| {
        let inside = set_by
            .iter()
            .filter(|(p, _)| !f.is_empty() && within(p, f))
            .map(|(_, s)| s);
        setter_of(set_by, f)
            .into_iter()
            .chain(inside)
            .find(|s| s.explicit)
    })
}

/// A kind whose events each send something, in its own terms.
struct Sender {
    kind: &'static str,
    /// The fields of an event that hold what it sends.
    sends: &'static [&'static str],
    /// The fields that set how long an event takes. One that starts with `/` is the part's.
    pace: &'static [&'static str],
    /// What an event that sends nothing lacks. The engine calls such an event `same_state`.
    lacks: &'static str,
    /// The fixes for an event that sends nothing, that ends past the largest time, and that
    /// starts before the previous one ends.
    empty: &'static str,
    overflow: &'static str,
    overlap: &'static str,
}

const SENDERS: &[Sender] = &[
    Sender {
        kind: "spi_replay",
        sends: &["phases", "bytes"],
        pace: &["lanes", "/clock_hz"],
        lacks: "no phase and no byte",
        empty: "give each event a phase or a byte, or remove the event",
        overflow: "send fewer bits, raise `clock_hz`, or move this event earlier",
        overlap: "move this event later, or raise `clock_hz` or send fewer bits in the previous event",
    },
    Sender {
        kind: "serial",
        sends: &["bytes"],
        pace: &["baud"],
        lacks: "no byte",
        empty: "give each event a byte, or remove the event",
        overflow: "send fewer bytes, raise `baud`, or move this event earlier",
        overlap: "move this event later, or raise `baud` or send fewer bytes in the previous event",
    },
    Sender {
        kind: "pulses_replay",
        sends: &["pulses"],
        pace: &[],
        lacks: "no pulse",
        empty: "give each event a pulse, or remove the event",
        overflow: "shorten this event's pulses, or move it earlier",
        overlap: "move this event later, or shorten the previous event's pulses",
    },
];

/// The JSON pointers of the fields a schedule rule reads for `field[event]`, its time first.
/// A switch's event ends with its bounce window. A sender's event ends once what it sends has
/// gone out, so its end reads what it sends and how fast.
fn schedule_fields(kind: &str, field: &str, event: usize, rule: ScheduleRule) -> Vec<String> {
    let list = format!("/{}", field.replace('.', "/"));
    let item = |index: usize, key: &str| match key.strip_prefix('/') {
        Some(_) => key.to_string(),
        None => format!("{list}/{index}/{key}"),
    };
    let previous = event.checked_sub(1);
    let sender = SENDERS.iter().find(|s| s.kind == kind);
    let ends = |index: usize| -> Vec<String> {
        match sender {
            Some(s) => s
                .sends
                .iter()
                .chain(s.pace)
                .map(|key| item(index, key))
                .collect(),
            None if kind == "switch" => vec!["/bounce/window".into()],
            None => Vec::new(),
        }
    };
    let mut fields = vec![item(event, "t_ps")];
    match rule {
        ScheduleRule::SameState => match sender {
            Some(s) => fields = s.sends.iter().map(|key| item(event, key)).collect(),
            None => {
                let to = if field == "events" { "state" } else { "level" };
                let before = match previous {
                    Some(previous) => item(previous, to),
                    None => format!("{}initial", list.trim_end_matches("events")),
                };
                fields = vec![item(event, to), before];
            }
        },
        ScheduleRule::NotIncreasing => fields.extend(previous.map(|p| item(p, "t_ps"))),
        ScheduleRule::TooClose | ScheduleRule::Overlap => {
            if let Some(previous) = previous {
                fields.push(item(previous, "t_ps"));
                fields.extend(ends(previous));
            }
        }
        ScheduleRule::Overflow => fields.extend(ends(event)),
        ScheduleRule::AtZero => {}
    }
    fields
}

/// A schedule finding's message and fix in the terms of the part's kind. `at` is the event as
/// the engine names it, such as `#rp events[0]`.
fn schedule(kind: &str, rule: ScheduleRule, at: &str, message: &str) -> (String, String) {
    let gap = sim_core::netlist::MIN_GAP_PS;
    let last = u64::MAX;
    let sender = SENDERS.iter().find(|s| s.kind == kind);
    let switch = kind == "switch";
    let fix = match (rule, sender) {
        (ScheduleRule::AtZero, _) => {
            "make this time later than 0, such as 1 ms, which is 1000000000 ps".into()
        }
        (ScheduleRule::NotIncreasing, _) => {
            "make each event's time later than the one before it".into()
        }
        (ScheduleRule::SameState, Some(s)) => s.empty.into(),
        (ScheduleRule::SameState, None) => "make each event change the state or level in force, which for the first event is `initial`".into(),
        (ScheduleRule::TooClose, _) if switch => format!("shorten `bounce.window`, or move this event later, so it starts at least {gap} ps after the previous event's bounce window ends"),
        (ScheduleRule::TooClose, _) => format!("move this event later, so it starts at least {gap} ps after the previous one"),
        (ScheduleRule::Overflow, _) if switch => format!("shorten `bounce.window`, or move this event earlier, so its bounce window ends by {last} ps"),
        (ScheduleRule::Overflow, Some(s)) => format!("{}, so it ends by {last} ps", s.overflow),
        (ScheduleRule::Overflow, None) => format!("move this event earlier, so it ends by {last} ps"),
        (ScheduleRule::Overlap, Some(s)) => format!("{}, so it starts once the previous event ends", s.overlap),
        (ScheduleRule::Overlap, None) => "move this event later, so it starts once the previous event ends".into(),
    };
    let said = match (rule, sender) {
        (ScheduleRule::SameState, Some(s)) => format!("{at} sends nothing: it has {}", s.lacks),
        _ => message.to_string(),
    };
    (said, fix)
}

/// The JSON pointers of the fields an engine finding's rule reads, the one it names first.
fn read_fields(value: &Value) -> Vec<String> {
    let named = match value["data"]["field"].as_str() {
        Some(field) if !field.is_empty() => format!("/{}", field.replace('.', "/")),
        _ => String::new(),
    };
    match value["data"]["rule"].as_str() {
        Some("bounce_room") => vec![named, "/bounce/max_bounces".into()],
        _ => vec![named],
    }
}

/// A schema finding's detail without the place in the netlist text, which is where the
/// engine finished reading the part and not where the field is.
fn detail(value: &Value) -> String {
    let detail = value["data"]["detail"].as_str().unwrap_or_default();
    detail
        .rsplit_once(" at line ")
        .map_or(detail, |(d, _)| d)
        .to_string()
}

/// The schema refusal of a netlist holding only `part`, if any.
fn refusal(head: &Head, part: &Value) -> Option<String> {
    let text = serde_json::to_string(&head.with([part])).expect("a JSON value");
    let diagnostics = sim_core::netlist::parse(text.as_bytes()).err()?;
    diagnostics
        .iter()
        .find(|d| d.code() == "netlist.syntax")
        .map(|d| detail(&serde_json::to_value(d).unwrap_or_default()))
}

/// Engine rules on one object, by the engine's words: the field each is about, the other
/// fields of the object it reads, and its fix. The first two are checked once all the object's
/// fields are read, so the search below finds only the object's last field for them.
const RULES: &[Rule] = &[
    (
        "a phase has 1 to 32 bits",
        "bits",
        &[],
        "give `bits` a value from 1 to 32",
    ),
    (
        "a phase's value fits in its bits",
        "value",
        &["bits"],
        "make `value` less than 2 to the power of `bits`, or raise `bits`",
    ),
    (
        "lanes is 1, 2 or 4",
        "lanes",
        &[],
        "give `lanes` the value 1, 2 or 4",
    ),
];

type Rule = (
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static str,
);

fn rule(detail: &str) -> Option<&'static Rule> {
    RULES.iter().find(|(words, ..)| *words == detail)
}

/// The JSON pointer of the field the engine refuses in `part`. The engine names only the part,
/// so this asks the engine again on cut-down copies. The engine reads an object's fields in
/// order and stops at the first bad one, so the first field that, kept with those before it,
/// still gives the same refusal holds the cause. A list is searched by halves, since a list can
/// be long and each try reads the whole part. A missing field is placed at the object that
/// lacks it: giving it that field there changes the refusal, and giving it to any other
/// object leaves it or makes the field unknown.
fn culprit(part: &Value, detail: &str, refusal: impl Fn(&Value) -> Option<String>) -> String {
    let missing = detail
        .strip_prefix("missing field `")
        .and_then(|rest| rest.split('`').next());
    let rule_field = rule(detail).map(|(_, field, ..)| *field);
    let mut at = String::new();
    while let Some(node) = part.pointer(&at) {
        if let (Some(key), Some(object)) = (missing, node.as_object())
            && !object.contains_key(key)
        {
            let mut trial = part.clone();
            if let Some(Value::Object(map)) = trial.pointer_mut(&at) {
                map.insert(key.to_string(), Value::Null);
            }
            let unknown = format!("unknown field `{key}`");
            if refusal(&trial).is_none_or(|d| d != detail && !d.starts_with(&unknown)) {
                return at;
            }
        }
        let same =
            |children: &[String]| refusal(&cut(part, &at, children)).as_deref() == Some(detail);
        let (children, first) = match node {
            Value::Object(map) => {
                let children: Vec<String> = map.keys().filter(|k| *k != "kind").cloned().collect();
                let first = (0..=children.len()).find(|&kept| same(&children[..kept]));
                if let (Some(kept), Some(field)) = (first, rule_field)
                    && kept == children.len()
                    && map.contains_key(field)
                {
                    return format!("{at}/{field}");
                }
                (children, first)
            }
            // A cut list has another length, so a refusal of the length stays with the list.
            Value::Array(_) if detail.starts_with("invalid length") => return at,
            Value::Array(items) => {
                let children: Vec<String> = (0..items.len()).map(|i| i.to_string()).collect();
                let first = shortest(children.len(), |kept| same(&children[..kept]));
                (children, first)
            }
            _ => return at,
        };
        match first {
            Some(0) if node.get("kind").is_some() => return format!("{at}/kind"),
            Some(0) | None => return at,
            Some(kept) => at = format!("{at}/{}", children[kept - 1]),
        }
    }
    at
}

/// The fewest of `len` items for which `holds` is true, when it holds for every count from
/// some point on, found by halves.
fn shortest(len: usize, holds: impl Fn(usize) -> bool) -> Option<usize> {
    if !holds(len) {
        return None;
    }
    let (mut low, mut high) = (0, len);
    while low < high {
        let middle = low + (high - low) / 2;
        if holds(middle) {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    Some(low)
}

/// `part` with the object or list at `at` cut down to `kept` and its `kind`.
fn cut(part: &Value, at: &str, kept: &[String]) -> Value {
    let mut trial = part.clone();
    match trial.pointer_mut(at) {
        Some(Value::Object(map)) => {
            let old = std::mem::take(map);
            *map = old
                .into_iter()
                .filter(|(k, _)| k == "kind" || kept.contains(k))
                .collect::<Map<String, Value>>();
        }
        Some(Value::Array(items)) => items.truncate(kept.len()),
        _ => {}
    }
    trial
}

/// A schema finding's message and hint. A whole-number refusal of a param's value is put in
/// that param's unit and string form.
fn words(
    detail: &str,
    at: &str,
    found: Option<&Value>,
    setter: Option<&Setter>,
) -> (String, String) {
    let expected = detail.split_once(", expected ").map(|(_, e)| e);
    if let (Some(found), Some((low, high))) = (
        found.filter(|v| v.is_number()),
        expected.and_then(integer_range),
    ) {
        return whole(found, low, high, setter);
    }
    let field = dotted(at);
    let place = if field.is_empty() {
        "the model".to_string()
    } else {
        format!("`{field}`")
    };
    if let Some(key) = detail
        .strip_prefix("missing field `")
        .and_then(|rest| rest.split('`').next())
    {
        return (
            format!("the engine needs `{key}` in {place}, and nothing writes it"),
            format!("add `{key}` to {place} in `model`"),
        );
    }
    if let Some(key) = detail
        .strip_prefix("unknown field `")
        .and_then(|rest| rest.split('`').next())
    {
        return (
            format!("the engine has no field `{key}` here"),
            format!(
                "point `sets` or the model at a field it has: {}",
                expected.unwrap_or("see the engine's netlist docs")
            ),
        );
    }
    let hint = if let Some((.., fix)) = rule(detail) {
        (*fix).to_string()
    } else if detail.starts_with("invalid type: map") && setter.is_some_and(|s| s.unit.is_some()) {
        format!(
            "a number param writes {{ value, unit, source }}, and the engine takes a bare value at {place}; point the param's `sets` at a field that takes a value object"
        )
    } else {
        "write the value as the engine's field takes it".into()
    };
    (format!("the engine does not read {place}: {detail}"), hint)
}

/// The refusal of `number` where the engine takes a whole number from `low` to `high`.
fn whole(found: &Value, low: i128, high: i128, setter: Option<&Setter>) -> (String, String) {
    let number = found.as_f64().unwrap_or_default();
    let unit = setter.and_then(|s| s.unit);
    let shown = match setter {
        Some(s) if !s.choice => s.shown.clone(),
        _ => found.to_string(),
    };
    // A u64 or i64 is the natural range of a whole number, so it is not worth printing.
    let narrow =
        (low, high) != (0, u64::MAX.into()) && (low, high) != (i64::MIN.into(), i64::MAX.into());
    let range = if narrow {
        format!(" from {low} to {high}")
    } else {
        String::new()
    };
    let of = match unit {
        Some(Unit::Second) => " of ps".to_string(),
        Some(Unit::One) | None => String::new(),
        Some(unit) => format!(" of {}", unit.as_str()),
    };
    let fits = (low as f64..=high as f64).contains(&number);
    let message = match (fits, narrow) {
        (true, true) => {
            format!("{shown} is not a whole number{of}; the engine takes one{range} here")
        }
        (true, false) => format!("{shown} is not a whole number{of}"),
        (false, true) => format!("{shown} is outside {low} to {high}{of}"),
        (false, false) if number < 0.0 => format!("{shown} is negative, and this field holds none"),
        (false, false) => format!("{shown} is more than this field holds"),
    };
    if setter.is_some_and(|s| s.choice) {
        return (
            message,
            format!("pick another choice, or make this choice's value a whole number{range}"),
        );
    }
    let example = |n: i128| match unit {
        Some(Unit::Second) => format!("\"{n}ps\", or \"1ms\""),
        Some(Unit::One) | None => n.to_string(),
        Some(_) => format!("\"{n}\""),
    };
    let hint = if fits || narrow {
        let nearest = (number.round() as i128).clamp(low, high);
        format!(
            "write a whole number{of}{range}, such as {}",
            example(nearest)
        )
    } else {
        let such = unit.map_or_else(|| example(0), |u| u.example().to_string());
        if number < 0.0 {
            format!("write a value of 0 or more, such as {such}")
        } else {
            format!("write a smaller value, such as {such}")
        }
    };
    (message, hint)
}

fn integer_range(expected: &str) -> Option<(i128, i128)> {
    Some(match expected {
        "u8" => (0, u8::MAX.into()),
        "u16" => (0, u16::MAX.into()),
        "u32" => (0, u32::MAX.into()),
        "u64" => (0, u64::MAX.into()),
        "i8" => (i8::MIN.into(), i8::MAX.into()),
        "i16" => (i16::MIN.into(), i16::MAX.into()),
        "i32" => (i32::MIN.into(), i32::MAX.into()),
        "i64" => (i64::MIN.into(), i64::MAX.into()),
        _ => return None,
    })
}

/// The engine's requirement, after its own `#part field is value; `.
fn requirement(message: &str) -> String {
    message
        .split_once("; ")
        .map_or(message, |(_, r)| r)
        .to_string()
}

/// A duration in picoseconds in the largest unit that keeps it whole, such as `8ns`.
fn duration(ps: u64) -> String {
    [
        ("s", 1_000_000_000_000),
        ("ms", 1_000_000_000),
        ("us", 1_000_000),
        ("ns", 1_000),
    ]
    .into_iter()
    .find(|(_, scale)| ps != 0 && ps.is_multiple_of(*scale))
    .map_or(format!("{ps}ps"), |(unit, scale)| {
        format!("{}{unit}", ps / scale)
    })
}
