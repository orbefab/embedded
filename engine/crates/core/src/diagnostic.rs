use serde::{Serialize, Serializer};

use crate::name::{NameError, Ref};
use crate::netlist::Schedule;
use crate::time::Time;

/// `Error` means the engine has no trustworthy result.
/// `Warning` means the result stands and the circuit still has a problem.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

/// Which numeric rule a netlist value broke.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueRule {
    Positive,
    FiniteReciprocal,
    /// `n*VT` is not normal, or `vcrit` is not finite and positive.
    FiniteJunction,
    /// `bounce.window` is less than `2 * max_bounces * MIN_GAP_PS`.
    BounceRoom,
    NonNegative,
}

/// Which rule a transient analysis broke.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisRule {
    Positive,
    /// `max_step_ps` is below 1000, the first step after a contact edge or a source jump.
    #[serde(rename = "step_under_1ns")]
    StepUnder1ns,
    /// `max_step_ps` is greater than `stop_ps`.
    StepOverStop,
}

/// Which rule a switch event broke.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleRule {
    /// The event is at t = 0, where the operating point already holds `initial`.
    AtZero,
    /// The event does not change the contact.
    SameState,
    /// The event is not strictly after the previous event.
    NotIncreasing,
    /// The event is less than `MIN_GAP_PS` after the previous event's last possible edge.
    TooClose,
    /// The event's bounce window ends past the largest `u64` picosecond.
    Overflow,
}

/// Which rule a source waveform's time broke.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WaveformRule {
    /// The first jump is at t = 0, where the operating point already holds `initial`.
    AtZero,
    /// Two jumps are less than `MIN_GAP_PS` apart.
    TooClose,
    /// The first fall is past the largest `u64` picosecond.
    Overflow,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Diagnostic {
    /// JSON is malformed or does not match the v0 schema.
    NetlistSyntax {
        path: String,
        line: usize,
        column: usize,
        detail: String,
    },
    /// `version` is not one this engine reads.
    NetlistVersion { found: u64, supported: u64 },
    /// A name breaks the grammar, or a part is named `net`.
    NetlistName {
        path: String,
        name: String,
        problem: NameError,
    },
    /// Two parts, or two nets, share a name.
    NetlistDuplicate { name: Ref },
    /// A pin names a net that is not in `nets`.
    NetlistUnknownNet { pin: Ref, net: String },
    /// A part's pin keys differ from its kind's pin set.
    NetlistPins {
        part: Ref,
        missing: Vec<&'static str>,
        unexpected: Vec<String>,
    },
    /// A value's `unit` is not the unit its field is measured in.
    NetlistUnit {
        part: Ref,
        field: &'static str,
        expected: &'static str,
        found: String,
    },
    /// A value is outside its field's domain.
    NetlistValue {
        part: Ref,
        field: &'static str,
        value: f64,
        rule: ValueRule,
    },
    /// `ground` names a net that is not in `nets`.
    NetlistGround { net: String },
    /// A transient analysis field breaks a rule.
    NetlistAnalysis {
        field: &'static str,
        value: u64,
        rule: AnalysisRule,
    },
    /// A switch's `events[event]` breaks a schedule rule.
    NetlistSchedule {
        part: Ref,
        event: usize,
        rule: ScheduleRule,
    },
    /// A source waveform's time field breaks a rule.
    NetlistWaveform {
        part: Ref,
        field: &'static str,
        value: u64,
        rule: WaveformRule,
    },
    /// The net has no DC path to ground. Its voltage is held only by gmin.
    NetFloating { net: Ref },
    /// The MNA matrix is singular at `unknown`.
    SolverSingular { at: Time, unknown: Ref },
    /// A solution component or an observed quantity is not finite.
    SolverNonFinite { at: Time, unknown: Ref },
    /// The dense system has more unknowns than it will allocate.
    SolverTooLarge {
        at: Time,
        unknowns: usize,
        limit: usize,
    },
    /// The chips kept changing state for `rounds` solves at one time. `chips` changed in the last.
    ChipUnsettled {
        at: Time,
        chips: Vec<Ref>,
        rounds: u32,
    },
    /// A transient would store more than `limit` trace rows.
    TraceTooLarge {
        at: Time,
        limit: usize,
        columns: usize,
    },
    /// Newton found no operating point.
    SolverConvergence {
        at: Time,
        /// The junction that failed worst in the last iteration, else the unknown.
        unknown: Ref,
        /// Newton iterations over every attempt.
        iterations: u32,
        /// Smallest extra node conductance that converged, in siemens.
        gmin: Option<f64>,
        /// Largest source scale that converged, from 0 to 1.
        source_scale: f64,
        solve: Solve,
    },
    /// A rated current went over its limit.
    RatingCurrent(Excursion),
    /// A rated power went over its limit.
    RatingPower(Excursion),
}

/// The solve that did not converge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Solve {
    /// The operating point, or the restart solve after an edge.
    Ladder,
    /// A time step, halved down to `step_ps`.
    Step { step_ps: u64 },
    /// A probe locating a chip event, `step_ps` after the last accepted time.
    Probe { step_ps: u64 },
}

/// One rated quantity over its limit, merged over the whole run.
#[derive(Clone, Debug, PartialEq)]
pub struct Excursion {
    /// `#<part>.i` or `#<part>.p`, which is also the observation key.
    pub quantity: Ref,
    /// The rating, in SI units.
    pub limit: f64,
    /// Largest magnitude seen.
    pub peak: f64,
    pub peak_at: Time,
    /// First sample over the limit.
    pub first: Time,
    /// Total time over the limit. `None` for an operating point, which has no duration.
    pub over_ps: Option<u64>,
}

impl Diagnostic {
    pub fn code(&self) -> &'static str {
        match self {
            Diagnostic::NetlistSyntax { .. } => "netlist.syntax",
            Diagnostic::NetlistVersion { .. } => "netlist.version",
            Diagnostic::NetlistName { .. } => "netlist.name",
            Diagnostic::NetlistDuplicate { .. } => "netlist.duplicate",
            Diagnostic::NetlistUnknownNet { .. } => "netlist.unknown_net",
            Diagnostic::NetlistPins { .. } => "netlist.pins",
            Diagnostic::NetlistUnit { .. } => "netlist.unit",
            Diagnostic::NetlistValue { .. } => "netlist.value",
            Diagnostic::NetlistGround { .. } => "netlist.ground",
            Diagnostic::NetlistAnalysis { .. } => "netlist.analysis",
            Diagnostic::NetlistSchedule { .. } => "netlist.schedule",
            Diagnostic::NetlistWaveform { .. } => "netlist.waveform",
            Diagnostic::NetFloating { .. } => "net.floating",
            Diagnostic::SolverSingular { .. } => "solver.singular",
            Diagnostic::SolverNonFinite { .. } => "solver.nonfinite",
            Diagnostic::SolverTooLarge { .. } => "solver.too_large",
            Diagnostic::SolverConvergence { .. } => "solver.convergence",
            Diagnostic::ChipUnsettled { .. } => "chip.unsettled",
            Diagnostic::TraceTooLarge { .. } => "trace.too_large",
            Diagnostic::RatingCurrent(_) => "rating.current",
            Diagnostic::RatingPower(_) => "rating.power",
        }
    }

    pub fn severity(&self) -> Severity {
        match self {
            Diagnostic::NetFloating { .. }
            | Diagnostic::RatingCurrent(_)
            | Diagnostic::RatingPower(_) => Severity::Warning,
            Diagnostic::NetlistSyntax { .. }
            | Diagnostic::NetlistVersion { .. }
            | Diagnostic::NetlistName { .. }
            | Diagnostic::NetlistDuplicate { .. }
            | Diagnostic::NetlistUnknownNet { .. }
            | Diagnostic::NetlistPins { .. }
            | Diagnostic::NetlistUnit { .. }
            | Diagnostic::NetlistValue { .. }
            | Diagnostic::NetlistGround { .. }
            | Diagnostic::NetlistAnalysis { .. }
            | Diagnostic::NetlistSchedule { .. }
            | Diagnostic::NetlistWaveform { .. }
            | Diagnostic::SolverSingular { .. }
            | Diagnostic::SolverNonFinite { .. }
            | Diagnostic::SolverTooLarge { .. }
            | Diagnostic::SolverConvergence { .. }
            | Diagnostic::ChipUnsettled { .. }
            | Diagnostic::TraceTooLarge { .. } => Severity::Error,
        }
    }

    pub fn at(&self) -> Option<Time> {
        match self {
            Diagnostic::SolverSingular { at, .. }
            | Diagnostic::SolverNonFinite { at, .. }
            | Diagnostic::SolverTooLarge { at, .. }
            | Diagnostic::SolverConvergence { at, .. }
            | Diagnostic::ChipUnsettled { at, .. }
            | Diagnostic::TraceTooLarge { at, .. } => Some(*at),
            Diagnostic::RatingCurrent(excursion) | Diagnostic::RatingPower(excursion) => {
                Some(excursion.first)
            }
            Diagnostic::NetlistSyntax { .. }
            | Diagnostic::NetlistVersion { .. }
            | Diagnostic::NetlistName { .. }
            | Diagnostic::NetlistDuplicate { .. }
            | Diagnostic::NetlistUnknownNet { .. }
            | Diagnostic::NetlistPins { .. }
            | Diagnostic::NetlistUnit { .. }
            | Diagnostic::NetlistValue { .. }
            | Diagnostic::NetlistGround { .. }
            | Diagnostic::NetlistAnalysis { .. }
            | Diagnostic::NetlistSchedule { .. }
            | Diagnostic::NetlistWaveform { .. }
            | Diagnostic::NetFloating { .. } => None,
        }
    }

    pub fn refs(&self) -> Vec<Ref> {
        match self {
            Diagnostic::NetlistSyntax { .. }
            | Diagnostic::NetlistVersion { .. }
            | Diagnostic::NetlistName { .. }
            | Diagnostic::NetlistGround { .. }
            | Diagnostic::NetlistAnalysis { .. } => Vec::new(),
            Diagnostic::NetlistDuplicate { name } => vec![name.clone()],
            Diagnostic::NetlistUnknownNet { pin, .. } => vec![pin.clone()],
            Diagnostic::NetlistPins { part, .. }
            | Diagnostic::NetlistUnit { part, .. }
            | Diagnostic::NetlistValue { part, .. }
            | Diagnostic::NetlistSchedule { part, .. }
            | Diagnostic::NetlistWaveform { part, .. } => vec![part.clone()],
            Diagnostic::NetFloating { net } => vec![net.clone()],
            Diagnostic::SolverSingular { unknown, .. }
            | Diagnostic::SolverNonFinite { unknown, .. }
            | Diagnostic::SolverConvergence { unknown, .. } => vec![unknown.clone()],
            Diagnostic::ChipUnsettled { chips, .. } => chips.clone(),
            Diagnostic::SolverTooLarge { .. } | Diagnostic::TraceTooLarge { .. } => Vec::new(),
            Diagnostic::RatingCurrent(excursion) | Diagnostic::RatingPower(excursion) => {
                vec![excursion.quantity.clone()]
            }
        }
    }

    /// `(time, code, refs)`. `Results` sorts by this and then by JSON bytes.
    pub fn sort_key(&self) -> (Option<Time>, &'static str, Vec<Ref>) {
        (self.at(), self.code(), self.refs())
    }

    fn message(&self) -> String {
        match self {
            Diagnostic::NetlistSyntax { path, detail, .. } => {
                format!("schema mismatch at `{path}` ({detail})")
            }
            Diagnostic::NetlistVersion { found, supported } => {
                format!("netlist version {found} is not supported; this engine reads {supported}")
            }
            Diagnostic::NetlistName {
                path,
                name,
                problem,
            } => {
                format!(
                    "name `{name}` at `{path}` is not valid ({})",
                    name_problem(*problem)
                )
            }
            Diagnostic::NetlistDuplicate { name } => format!("{name} is declared more than once"),
            Diagnostic::NetlistUnknownNet { pin, net } => {
                format!("pin {pin} names net `{net}`, which is not in `nets`")
            }
            Diagnostic::NetlistPins { part, .. } => format!("{part} pins do not match its kind"),
            Diagnostic::NetlistUnit {
                part,
                field,
                expected,
                found,
            } => format!(
                "{part} {field} is in `{found}`; it must be in `{expected}` (SI base units)"
            ),
            Diagnostic::NetlistValue {
                part,
                field,
                value,
                rule,
            } => {
                let value = number_text(*value);
                match rule {
                    ValueRule::Positive => {
                        format!("{part} {field} is {value}; it must be positive")
                    }
                    ValueRule::FiniteReciprocal => {
                        format!("{part} {field} is {value}; its reciprocal must be finite")
                    }
                    ValueRule::FiniteJunction => format!(
                        "{part} {field} is {value}; n*VT must be normal, and vcrit must be finite and positive"
                    ),
                    ValueRule::NonNegative => {
                        format!("{part} {field} is {value}; it must not be negative")
                    }
                    ValueRule::BounceRoom => format!(
                        "{part} {field} is {value}; it must fit 2 * max_bounces edges {} ps apart",
                        Schedule::MIN_GAP_PS
                    ),
                }
            }
            Diagnostic::NetlistGround { net } => format!("ground `{net}` is not in `nets`"),
            Diagnostic::NetlistAnalysis { field, value, rule } => {
                let must = match rule {
                    AnalysisRule::Positive => "it must be positive",
                    AnalysisRule::StepUnder1ns => "it must be at least 1000",
                    AnalysisRule::StepOverStop => "it must not exceed stop_ps",
                };
                format!("analysis {field} is {value}; {must}")
            }
            Diagnostic::NetlistSchedule { part, event, rule } => {
                let what = match rule {
                    ScheduleRule::AtZero => {
                        "is at 0 ps; an event must come after t = 0".to_string()
                    }
                    ScheduleRule::SameState => "does not change the contact".to_string(),
                    ScheduleRule::NotIncreasing => "is not after the previous event".to_string(),
                    ScheduleRule::TooClose => format!(
                        "is less than {} ps after the previous event and its bounce window",
                        Schedule::MIN_GAP_PS
                    ),
                    ScheduleRule::Overflow => {
                        "has a bounce window that ends past the largest time".to_string()
                    }
                };
                format!("{part} events[{event}] {what}")
            }
            Diagnostic::NetlistWaveform {
                part,
                field,
                value,
                rule,
            } => {
                let must = match rule {
                    WaveformRule::AtZero => "a jump must come after t = 0".to_string(),
                    WaveformRule::TooClose => format!(
                        "two jumps must be at least {} ps apart",
                        Schedule::MIN_GAP_PS
                    ),
                    WaveformRule::Overflow => "the first fall is past the largest time".to_string(),
                };
                format!("{part} {field} is {value}; {must}")
            }
            Diagnostic::NetFloating { net } => format!("{net} has no DC path to ground"),
            Diagnostic::SolverSingular { unknown, .. } => format!("singular matrix at {unknown}"),
            Diagnostic::SolverNonFinite { unknown, .. } => {
                format!("non-finite number at {unknown}")
            }
            Diagnostic::SolverTooLarge {
                unknowns, limit, ..
            } => format!("{unknowns} unknowns exceeds the dense solver limit of {limit}"),
            Diagnostic::SolverConvergence {
                at,
                unknown,
                iterations,
                source_scale,
                solve: Solve::Ladder,
                ..
            } if *at > Time::ZERO => format!(
                "Newton did not converge at {unknown} after {iterations} iterations in the restart solve at {} ps; source stepping reached {source_scale}",
                at.as_ps()
            ),
            Diagnostic::SolverConvergence {
                unknown,
                iterations,
                source_scale,
                solve: Solve::Ladder,
                ..
            } => format!(
                "Newton did not converge at {unknown} after {iterations} iterations; source stepping reached {source_scale}"
            ),
            Diagnostic::SolverConvergence {
                unknown,
                iterations,
                solve: Solve::Step { step_ps },
                ..
            } => format!(
                "Newton did not converge at {unknown} after {iterations} iterations; the time step halved to {step_ps} ps"
            ),
            Diagnostic::SolverConvergence {
                at,
                unknown,
                iterations,
                solve: Solve::Probe { .. },
                ..
            } => format!(
                "Newton did not converge at {unknown} after {iterations} iterations in the probe at {} ps locating a chip event",
                at.as_ps()
            ),
            Diagnostic::ChipUnsettled { at, rounds, .. } => format!(
                "the chips kept changing state for {rounds} solves at {} ps",
                at.as_ps()
            ),
            Diagnostic::TraceTooLarge { limit, .. } => format!(
                "the trace reached its limit of {limit} rows; rows come from stop_ps, the switch edges, the source jumps, the chip events and the error control, and max_step_ps only caps the step"
            ),
            Diagnostic::RatingCurrent(excursion) => over_rating(excursion, "A"),
            Diagnostic::RatingPower(excursion) => over_rating(excursion, "W"),
        }
    }

    fn data(&self) -> serde_json::Value {
        match self {
            Diagnostic::NetlistSyntax {
                path,
                line,
                column,
                detail,
            } => serde_json::json!({
                "path": path,
                "line": line,
                "column": column,
                "detail": detail,
            }),
            Diagnostic::NetlistVersion { found, supported } => {
                serde_json::json!({ "found": found, "supported": supported })
            }
            Diagnostic::NetlistName {
                path,
                name,
                problem,
            } => {
                serde_json::json!({ "path": path, "name": name, "problem": problem })
            }
            Diagnostic::NetlistDuplicate { .. } | Diagnostic::NetFloating { .. } => {
                serde_json::json!({})
            }
            Diagnostic::NetlistUnknownNet { net, .. } => serde_json::json!({ "net": net }),
            Diagnostic::NetlistPins {
                missing,
                unexpected,
                ..
            } => serde_json::json!({ "missing": missing, "unexpected": unexpected }),
            Diagnostic::NetlistUnit {
                field,
                expected,
                found,
                ..
            } => serde_json::json!({
                "field": field,
                "expected": expected,
                "found": found,
            }),
            Diagnostic::NetlistValue {
                field, value, rule, ..
            } => serde_json::json!({ "field": field, "value": value, "rule": rule }),
            Diagnostic::NetlistGround { net } => serde_json::json!({ "net": net }),
            Diagnostic::NetlistAnalysis { field, value, rule } => {
                serde_json::json!({ "field": field, "rule": rule, "value": value })
            }
            Diagnostic::NetlistSchedule { event, rule, .. } => {
                serde_json::json!({ "event": event, "rule": rule })
            }
            Diagnostic::NetlistWaveform {
                field, value, rule, ..
            } => serde_json::json!({ "field": field, "rule": rule, "value": value }),
            Diagnostic::SolverSingular { .. } | Diagnostic::SolverNonFinite { .. } => {
                serde_json::json!({})
            }
            Diagnostic::SolverTooLarge {
                unknowns, limit, ..
            } => serde_json::json!({ "limit": limit, "unknowns": unknowns }),
            Diagnostic::SolverConvergence {
                iterations,
                gmin,
                source_scale,
                solve,
                ..
            } => {
                let mut data = serde_json::json!({
                    "gmin": gmin,
                    "iterations": iterations,
                    "source_scale": source_scale,
                });
                if let Solve::Step { step_ps } | Solve::Probe { step_ps } = solve {
                    data["step_ps"] = serde_json::json!(step_ps);
                }
                data
            }
            Diagnostic::ChipUnsettled { rounds, .. } => serde_json::json!({ "rounds": rounds }),
            Diagnostic::TraceTooLarge { limit, columns, .. } => {
                serde_json::json!({ "columns": columns, "limit": limit })
            }
            Diagnostic::RatingCurrent(excursion) | Diagnostic::RatingPower(excursion) => {
                serde_json::json!({
                    "limit": excursion.limit,
                    "over_ps": excursion.over_ps,
                    "peak": excursion.peak,
                    "peak_ps": excursion.peak_at,
                })
            }
        }
    }
}

/// The same characters `data.value` uses. `Display` for a subnormal is hundreds of digits.
fn number_text(value: f64) -> String {
    serde_json::to_string(&value).expect("a netlist value is a finite number")
}

fn over_rating(excursion: &Excursion, unit: &str) -> String {
    let Excursion {
        quantity,
        limit,
        peak,
        ..
    } = excursion;
    format!("{quantity} peaks at {peak} {unit}, over its {limit} {unit} rating")
}

fn name_problem(problem: NameError) -> &'static str {
    match problem {
        NameError::Empty => "it is empty",
        NameError::TooLong => "it is longer than 64 bytes",
        NameError::BadChar => "it must be an ASCII letter, then letters, digits, or underscore",
        NameError::Reserved => "`net` is reserved",
    }
}

impl Serialize for Diagnostic {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Wire<'a> {
            code: &'static str,
            severity: Severity,
            refs: Vec<Ref>,
            t_ps: Option<Time>,
            message: String,
            data: &'a serde_json::Value,
        }
        let data = self.data();
        Wire {
            code: self.code(),
            severity: self.severity(),
            refs: self.refs(),
            t_ps: self.at(),
            message: self.message(),
            data: &data,
        }
        .serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::name::{Name, NameError, Ref};

    fn name(text: &str) -> Name {
        Name::new(text).unwrap()
    }

    fn part(text: &str) -> Ref {
        Ref::Part(name(text))
    }

    fn samples() -> Vec<Diagnostic> {
        vec![
            Diagnostic::NetlistSyntax {
                path: "parts[0]".into(),
                line: 7,
                column: 21,
                detail: "unknown field `resistence`".into(),
            },
            Diagnostic::NetlistVersion {
                found: 1,
                supported: 0,
            },
            Diagnostic::NetlistName {
                path: "parts[0].name".into(),
                name: "net".into(),
                problem: NameError::Reserved,
            },
            Diagnostic::NetlistDuplicate { name: part("r1") },
            Diagnostic::NetlistUnknownNet {
                pin: Ref::Pin(name("r2"), name("b")),
                net: "gnd2".into(),
            },
            Diagnostic::NetlistPins {
                part: part("r1"),
                missing: vec!["b"],
                unexpected: vec!["c".into()],
            },
            Diagnostic::NetlistUnit {
                part: part("r1"),
                field: "resistance",
                expected: "ohm",
                found: "kohm".into(),
            },
            Diagnostic::NetlistValue {
                part: part("r1"),
                field: "resistance",
                value: 0.0,
                rule: ValueRule::Positive,
            },
            Diagnostic::NetlistGround {
                net: "earth".into(),
            },
            Diagnostic::NetFloating {
                net: Ref::Net(name("isle")),
            },
            Diagnostic::SolverSingular {
                at: Time::ZERO,
                unknown: part("v2"),
            },
            Diagnostic::SolverNonFinite {
                at: Time::ZERO,
                unknown: part("r1"),
            },
            Diagnostic::SolverTooLarge {
                at: Time::ZERO,
                unknowns: 4097,
                limit: 4096,
            },
            Diagnostic::SolverConvergence {
                at: Time::ZERO,
                unknown: part("led1"),
                iterations: 1310,
                gmin: None,
                source_scale: 0.75,
                solve: Solve::Ladder,
            },
            Diagnostic::RatingCurrent(Excursion {
                quantity: "#led1.i".parse().unwrap(),
                limit: 0.03,
                peak: 0.5,
                peak_at: Time::ZERO,
                first: Time::ZERO,
                over_ps: None,
            }),
            Diagnostic::RatingPower(Excursion {
                quantity: "#r1.p".parse().unwrap(),
                limit: 0.25,
                peak: 3.0,
                peak_at: Time::from_ps(7),
                first: Time::from_ps(5),
                over_ps: Some(4),
            }),
            Diagnostic::NetlistSchedule {
                part: part("sw1"),
                event: 1,
                rule: ScheduleRule::TooClose,
            },
            Diagnostic::NetlistAnalysis {
                field: "max_step_ps",
                value: 999,
                rule: AnalysisRule::StepUnder1ns,
            },
            Diagnostic::TraceTooLarge {
                at: Time::from_ps(3_000),
                limit: 4,
                columns: 2,
            },
            Diagnostic::ChipUnsettled {
                at: Time::from_ps(3_000),
                chips: vec![part("u1"), part("u2")],
                rounds: 8,
            },
            Diagnostic::NetlistWaveform {
                part: part("i1"),
                field: "current.width_ps",
                value: 1500,
                rule: WaveformRule::TooClose,
            },
        ]
    }

    fn is_code(code: &str) -> bool {
        let mut parts = code.split('.');
        let Some(head) = parts.next() else {
            return false;
        };
        if head.is_empty() || !head.bytes().all(|b| b.is_ascii_lowercase()) {
            return false;
        }
        let mut tails = 0;
        for part in parts {
            tails += 1;
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
                return false;
            }
        }
        tails >= 1
    }

    #[test]
    fn codes_are_unique_and_match_the_grammar() {
        let found = samples();
        let mut seen = std::collections::BTreeSet::new();
        for diagnostic in &found {
            let code = diagnostic.code();
            assert!(is_code(code), "{code}");
            assert!(seen.insert(code), "duplicate code {code}");
        }
        assert_eq!(seen.len(), 21);
    }

    #[test]
    fn severity_refs_and_time_follow_the_variant() {
        let found = samples();
        let codes: Vec<_> = found.iter().map(Diagnostic::code).collect();
        assert_eq!(
            codes,
            [
                "netlist.syntax",
                "netlist.version",
                "netlist.name",
                "netlist.duplicate",
                "netlist.unknown_net",
                "netlist.pins",
                "netlist.unit",
                "netlist.value",
                "netlist.ground",
                "net.floating",
                "solver.singular",
                "solver.nonfinite",
                "solver.too_large",
                "solver.convergence",
                "rating.current",
                "rating.power",
                "netlist.schedule",
                "netlist.analysis",
                "trace.too_large",
                "chip.unsettled",
                "netlist.waveform",
            ]
        );
        for diagnostic in &found {
            let code = diagnostic.code();
            let warning = code == "net.floating" || code.starts_with("rating.");
            assert_eq!(diagnostic.severity() == Severity::Warning, warning);
            if code.starts_with("solver.") || code == "rating.current" {
                assert_eq!(diagnostic.at(), Some(Time::ZERO));
            } else if code == "rating.power" {
                assert_eq!(diagnostic.at(), Some(Time::from_ps(5)));
            } else if code == "trace.too_large" || code == "chip.unsettled" {
                assert_eq!(diagnostic.at(), Some(Time::from_ps(3_000)));
            } else {
                assert_eq!(diagnostic.at(), None);
            }
        }
        assert!(found[0].refs().is_empty());
        assert!(found[1].refs().is_empty());
        assert!(found[2].refs().is_empty());
        assert_eq!(found[3].refs(), vec!["#r1".parse().unwrap()]);
        assert_eq!(found[4].refs(), vec!["#r2.pin.b".parse().unwrap()]);
        assert_eq!(found[5].refs(), vec!["#r1".parse().unwrap()]);
        assert_eq!(found[6].refs(), vec!["#r1".parse().unwrap()]);
        assert_eq!(found[7].refs(), vec!["#r1".parse().unwrap()]);
        assert!(found[8].refs().is_empty());
        assert_eq!(found[9].refs(), vec!["#net.isle".parse().unwrap()]);
        assert_eq!(found[10].refs(), vec!["#v2".parse().unwrap()]);
        assert_eq!(found[11].refs(), vec!["#r1".parse().unwrap()]);
        assert!(found[12].refs().is_empty());
        let too_large = serde_json::to_string(&found[12]).unwrap();
        assert!(
            too_large.contains(r#""data":{"limit":4096,"unknowns":4097}"#),
            "{too_large}"
        );
        assert_eq!(found[13].refs(), vec!["#led1".parse().unwrap()]);
        let convergence = serde_json::to_string(&found[13]).unwrap();
        assert!(
            convergence.contains(r#""data":{"gmin":null,"iterations":1310,"source_scale":0.75}"#),
            "{convergence}"
        );
        assert_eq!(found[14].refs(), vec!["#led1.i".parse().unwrap()]);
        let current = serde_json::to_string(&found[14]).unwrap();
        assert!(
            current.contains(
                r##""message":"#led1.i peaks at 0.5 A, over its 0.03 A rating","data":{"limit":0.03,"over_ps":null,"peak":0.5,"peak_ps":0}"##
            ),
            "{current}"
        );
        let schedule = serde_json::to_string(&found[16]).unwrap();
        assert!(
            schedule.contains(
                r##""refs":["#sw1"],"t_ps":null,"message":"#sw1 events[1] is less than 2000 ps after the previous event and its bounce window","data":{"event":1,"rule":"too_close"}"##
            ),
            "{schedule}"
        );
        let analysis = serde_json::to_string(&found[17]).unwrap();
        assert!(
            analysis.contains(
                r#""refs":[],"t_ps":null,"message":"analysis max_step_ps is 999; it must be at least 1000","data":{"field":"max_step_ps","rule":"step_under_1ns","value":999}"#
            ),
            "{analysis}"
        );
        let rows = serde_json::to_string(&found[18]).unwrap();
        assert!(
            rows.contains(
                r#""severity":"error","refs":[],"t_ps":3000,"message":"the trace reached its limit of 4 rows; rows come from stop_ps, the switch edges, the source jumps, the chip events and the error control, and max_step_ps only caps the step","data":{"columns":2,"limit":4}"#
            ),
            "{rows}"
        );
        let unsettled = serde_json::to_string(&found[19]).unwrap();
        assert_eq!(
            unsettled,
            r##"{"code":"chip.unsettled","severity":"error","refs":["#u1","#u2"],"t_ps":3000,"message":"the chips kept changing state for 8 solves at 3000 ps","data":{"rounds":8}}"##
        );
        let waveform = serde_json::to_string(&found[20]).unwrap();
        assert!(
            waveform.contains(
                r##""refs":["#i1"],"t_ps":null,"message":"#i1 current.width_ps is 1500; two jumps must be at least 2000 ps apart","data":{"field":"current.width_ps","rule":"too_close","value":1500}"##
            ),
            "{waveform}"
        );
        let power = serde_json::to_string(&found[15]).unwrap();
        assert!(
            power.contains(
                r##""refs":["#r1.p"],"t_ps":5,"message":"#r1.p peaks at 3 W, over its 0.25 W rating","data":{"limit":0.25,"over_ps":4,"peak":3.0,"peak_ps":7}"##
            ),
            "{power}"
        );
    }

    #[test]
    fn a_failed_restart_names_its_edge_and_keeps_the_operating_point_data() {
        let restart = Diagnostic::SolverConvergence {
            at: Time::from_ps(1_000_000),
            unknown: part("led1"),
            iterations: 12,
            gmin: None,
            source_scale: 0.0,
            solve: Solve::Ladder,
        };
        let text = serde_json::to_string(&restart).unwrap();
        assert!(
            text.contains(
                r##""t_ps":1000000,"message":"Newton did not converge at #led1 after 12 iterations in the restart solve at 1000000 ps; source stepping reached 0","data":{"gmin":null,"iterations":12,"source_scale":0.0}"##
            ),
            "{text}"
        );
    }

    #[test]
    fn a_failed_time_step_adds_its_step_to_the_operating_point_data() {
        let step = Diagnostic::SolverConvergence {
            at: Time::from_ps(2_000_000_000),
            unknown: part("led1"),
            iterations: 50,
            gmin: None,
            source_scale: 1.0,
            solve: Solve::Step { step_ps: 1 },
        };
        let text = serde_json::to_string(&step).unwrap();
        assert!(
            text.contains(
                r##""t_ps":2000000000,"message":"Newton did not converge at #led1 after 50 iterations; the time step halved to 1 ps","data":{"gmin":null,"iterations":50,"source_scale":1.0,"step_ps":1}"##
            ),
            "{text}"
        );
    }

    #[test]
    fn json_uses_the_catalog_field_order() {
        let unknown = &samples()[4];
        let text = serde_json::to_string(unknown).unwrap();
        assert!(text.starts_with(
            r##"{"code":"netlist.unknown_net","severity":"error","refs":["#r2.pin.b"],"t_ps":null,"##
        ));
        assert!(
            text.contains(
                r##""message":"pin #r2.pin.b names net `gnd2`, which is not in `nets`""##
            )
        );
        assert!(text.ends_with(r#""data":{"net":"gnd2"}}"#));

        let unit = &samples()[6];
        let text = serde_json::to_string(unit).unwrap();
        assert!(text.contains(
            r##""message":"#r1 resistance is in `kohm`; it must be in `ohm` (SI base units)""##
        ));
        assert!(text.contains(r#""data":{"expected":"ohm","field":"resistance","found":"kohm"}"#));
    }

    #[test]
    fn a_value_message_uses_the_same_characters_as_data_value() {
        for rule in [
            ValueRule::Positive,
            ValueRule::FiniteReciprocal,
            ValueRule::FiniteJunction,
            ValueRule::BounceRoom,
            ValueRule::NonNegative,
        ] {
            let diagnostic = Diagnostic::NetlistValue {
                part: part("d1"),
                field: "saturation_current",
                value: 1e-308,
                rule,
            };
            let text = serde_json::to_string(&diagnostic).unwrap();
            let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
            let number = doc["data"]["value"].to_string();
            let message = doc["message"].as_str().unwrap();
            assert!(
                message.contains(&format!("is {number};")),
                "{rule:?} {message} vs {number}"
            );
            assert!(!message.contains("000000"), "{rule:?} {message}");
        }
    }
}
