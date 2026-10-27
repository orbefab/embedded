use std::borrow::Cow;

use serde::{Serialize, Serializer};

use crate::name::{Name, NameError, Ref};
use crate::netlist::MIN_GAP_PS;
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
    /// On a zener, `nbvt` is not normal, or `Isb` or its `vcrit` is not.
    FiniteJunction,
    /// `bounce.window` is less than `2 * max_bounces * MIN_GAP_PS`.
    BounceRoom,
    NonNegative,
    /// Not inside `(0, 1)`.
    Fraction,
    /// Not at least 0 and below 1. Zero is allowed. One is not.
    BelowOne,
    /// A threshold's `low` is above its `high`.
    AboveHigh,
    /// A relay's `drop_out_current` is at or above its `pull_in_current`.
    Hysteresis,
    /// A zener's `zener_impedance` does not exceed `series_resistance + breakdown_resistance`.
    AboveResistance,
    /// A zener's `zener_voltage` does not exceed `zener_current` times those two resistances.
    AboveDrop,
    /// A fuse byte selects a mode the model does not run.
    Unsupported,
}

/// Why `firmware` is not a program for the part's flash.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FirmwareRule {
    /// The line is not an Intel HEX record: no `:`, a digit that is not hexadecimal, or a
    /// length that does not match its byte count or its type.
    Record,
    /// The record's bytes do not sum to 0.
    Checksum,
    /// A record type other than 00 to 05.
    RecordType,
    /// A record after the end-of-file record.
    AfterEnd,
    /// No end-of-file record.
    NoEnd,
    /// A data byte at or past the end of the flash.
    PastFlash,
    /// A data byte an earlier record already placed.
    Overlap,
}

/// Which rule a transient analysis broke.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisRule {
    Positive,
    /// `max_step_ps` is below 1000, the first step of a segment.
    #[serde(rename = "step_under_1ns")]
    StepUnder1ns,
    /// `max_step_ps` is greater than `stop_ps`.
    StepOverStop,
    /// `grid_ps` is below 1000.
    #[serde(rename = "grid_under_1ns")]
    GridUnder1ns,
    /// `grid_ps` is greater than `stop_ps`.
    GridOverStop,
}

/// Which rule a schedule event broke. A switch, a `steps` source, a potentiometer, and a
/// `serial` source share it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleRule {
    /// The event is at t = 0, where the operating point already holds `initial`.
    AtZero,
    /// The event does not change the value in force.
    SameState,
    /// The event is not strictly after the previous event.
    NotIncreasing,
    /// The event is less than `MIN_GAP_PS` after the previous event's last possible edge.
    TooClose,
    /// The event ends past the largest `u64` picosecond. For a switch that is the end of its
    /// bounce window.
    Overflow,
    /// The event starts before the previous serial event ends.
    Overlap,
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
    /// `mark` and `space` are the same SI value, so the line never changes.
    Equal,
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
    /// `version` is not one this engine reads. `supported` is every version it reads.
    NetlistVersion {
        found: u64,
        supported: &'static [u64],
    },
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
        field: Cow<'static, str>,
        expected: &'static str,
        found: String,
    },
    /// A value is outside its field's domain.
    NetlistValue {
        part: Ref,
        field: Cow<'static, str>,
        value: f64,
        rule: ValueRule,
    },
    /// A part's `firmware` is not a program for its flash. `line` is 1-based, `None` when no
    /// one line is at fault.
    NetlistFirmware {
        part: Ref,
        line: Option<usize>,
        rule: FirmwareRule,
    },
    /// `pin` is on net `found`, and its kind ties it to `with`, on net `expected`.
    NetlistSameNet {
        pin: Ref,
        with: Ref,
        found: Name,
        expected: Name,
    },
    /// `ground` names a net that is not in `nets`.
    NetlistGround { net: String },
    /// A transient analysis field breaks a rule.
    NetlistAnalysis {
        field: &'static str,
        value: u64,
        rule: AnalysisRule,
    },
    /// `field[event]` breaks a schedule rule. `field` is `events`, `voltage.events`,
    /// `current.events`, or `position.events`. `end_ps` is the previous event's end. Only an
    /// `overlap` message reads either one. `data` does not hold them.
    NetlistSchedule {
        part: Ref,
        field: &'static str,
        event: usize,
        rule: ScheduleRule,
        end_ps: u64,
    },
    /// A source waveform's time field breaks a rule. `field` is owned when it names one event.
    NetlistWaveform {
        part: Ref,
        field: Cow<'static, str>,
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
    /// The run accepted `limit` steps and had not reached its stop.
    SolverStepLimit { at: Time, limit: u64 },
    /// The chips kept changing state for `rounds` solves at one time. `chips` changed in the last.
    ChipUnsettled {
        at: Time,
        chips: Vec<Ref>,
        rounds: u32,
    },
    /// The grid gives more rows than the trace holds, found before the first solve, when
    /// `at` is `None`. The test-only step-rows mode can also fill the trace and refuse the
    /// next step. Then `at` is that step, and the rows already written stay.
    TraceTooLarge {
        rows: u64,
        limit: usize,
        columns: usize,
        at: Option<Time>,
    },
    /// The row at `at` did not solve, twice, so it is the line between the accepted steps at
    /// `from` and `to`.
    TraceRowInterpolated { at: Time, from: Time, to: Time },
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
            Diagnostic::NetlistFirmware { .. } => "netlist.firmware",
            Diagnostic::NetlistSameNet { .. } => "netlist.same_net",
            Diagnostic::NetlistGround { .. } => "netlist.ground",
            Diagnostic::NetlistAnalysis { .. } => "netlist.analysis",
            Diagnostic::NetlistSchedule { .. } => "netlist.schedule",
            Diagnostic::NetlistWaveform { .. } => "netlist.waveform",
            Diagnostic::NetFloating { .. } => "net.floating",
            Diagnostic::SolverSingular { .. } => "solver.singular",
            Diagnostic::SolverNonFinite { .. } => "solver.nonfinite",
            Diagnostic::SolverTooLarge { .. } => "solver.too_large",
            Diagnostic::SolverStepLimit { .. } => "solver.step_limit",
            Diagnostic::SolverConvergence { .. } => "solver.convergence",
            Diagnostic::ChipUnsettled { .. } => "chip.unsettled",
            Diagnostic::TraceTooLarge { .. } => "trace.too_large",
            Diagnostic::TraceRowInterpolated { .. } => "trace.row_interpolated",
            Diagnostic::RatingCurrent(_) => "rating.current",
            Diagnostic::RatingPower(_) => "rating.power",
        }
    }

    pub fn severity(&self) -> Severity {
        match self {
            Diagnostic::NetFloating { .. }
            | Diagnostic::TraceRowInterpolated { .. }
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
            | Diagnostic::NetlistFirmware { .. }
            | Diagnostic::NetlistSameNet { .. }
            | Diagnostic::NetlistGround { .. }
            | Diagnostic::NetlistAnalysis { .. }
            | Diagnostic::NetlistSchedule { .. }
            | Diagnostic::NetlistWaveform { .. }
            | Diagnostic::SolverSingular { .. }
            | Diagnostic::SolverNonFinite { .. }
            | Diagnostic::SolverTooLarge { .. }
            | Diagnostic::SolverStepLimit { .. }
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
            | Diagnostic::SolverStepLimit { at, .. }
            | Diagnostic::SolverConvergence { at, .. }
            | Diagnostic::ChipUnsettled { at, .. }
            | Diagnostic::TraceRowInterpolated { at, .. } => Some(*at),
            Diagnostic::TraceTooLarge { at, .. } => *at,
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
            | Diagnostic::NetlistFirmware { .. }
            | Diagnostic::NetlistSameNet { .. }
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
            Diagnostic::NetlistSameNet { pin, with, .. } => vec![pin.clone(), with.clone()],
            Diagnostic::NetlistPins { part, .. }
            | Diagnostic::NetlistUnit { part, .. }
            | Diagnostic::NetlistValue { part, .. }
            | Diagnostic::NetlistFirmware { part, .. }
            | Diagnostic::NetlistSchedule { part, .. }
            | Diagnostic::NetlistWaveform { part, .. } => vec![part.clone()],
            Diagnostic::NetFloating { net } => vec![net.clone()],
            Diagnostic::SolverSingular { unknown, .. }
            | Diagnostic::SolverNonFinite { unknown, .. }
            | Diagnostic::SolverConvergence { unknown, .. } => vec![unknown.clone()],
            Diagnostic::ChipUnsettled { chips, .. } => chips.clone(),
            Diagnostic::SolverTooLarge { .. }
            | Diagnostic::SolverStepLimit { .. }
            | Diagnostic::TraceTooLarge { .. }
            | Diagnostic::TraceRowInterpolated { .. } => Vec::new(),
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
                let supported: Vec<_> = supported.iter().map(u64::to_string).collect();
                format!(
                    "netlist version {found} is not supported; this engine reads {}",
                    supported.join(", ")
                )
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
                let byte = *value as u8;
                let value = number_text(*value);
                match rule {
                    ValueRule::Positive => {
                        format!("{part} {field} is {value}; it must be positive")
                    }
                    ValueRule::FiniteReciprocal => {
                        format!("{part} {field} is {value}; its reciprocal must be finite")
                    }
                    ValueRule::FiniteJunction => {
                        let requirement = if field.as_ref() == "zener_voltage" {
                            "Isb must be normal and positive, and vcrit must be finite and positive"
                        } else if field.as_ref() == "zener_impedance" {
                            "nbvt must be normal"
                        } else {
                            "n*VT must be normal, and vcrit must be finite and positive"
                        };
                        format!("{part} {field} is {value}; {requirement}")
                    }
                    ValueRule::NonNegative => {
                        format!("{part} {field} is {value}; it must not be negative")
                    }
                    ValueRule::BounceRoom => format!(
                        "{part} {field} is {value}; it must fit 2 * max_bounces edges {} ps apart",
                        MIN_GAP_PS
                    ),
                    ValueRule::Fraction => {
                        format!("{part} {field} is {value}; it must be between 0 and 1")
                    }
                    ValueRule::BelowOne => {
                        format!("{part} {field} is {value}; it must be at least 0 and below 1")
                    }
                    ValueRule::AboveHigh => {
                        format!("{part} {field} is {value}; it must not be above high")
                    }
                    ValueRule::Hysteresis => {
                        format!("{part} {field} is {value}; it must be below pull_in_current")
                    }
                    ValueRule::AboveResistance => format!(
                        "{part} {field} is {value}; it must exceed series_resistance + breakdown_resistance"
                    ),
                    ValueRule::AboveDrop => format!(
                        "{part} {field} is {value}; it must exceed zener_current * (series_resistance + breakdown_resistance)"
                    ),
                    ValueRule::Unsupported => {
                        format!("{part} {field} is 0x{byte:02X}; the model does not run that mode")
                    }
                }
            }
            Diagnostic::NetlistFirmware { part, line, rule } => {
                let what = match rule {
                    FirmwareRule::Record => "is not an Intel HEX record",
                    FirmwareRule::Checksum => "has a bad checksum",
                    FirmwareRule::RecordType => "has a record type other than 00 to 05",
                    FirmwareRule::AfterEnd => "comes after the end-of-file record",
                    FirmwareRule::NoEnd => "has no end-of-file record",
                    FirmwareRule::PastFlash => "puts a byte past the end of the flash",
                    FirmwareRule::Overlap => "places a byte an earlier record placed",
                };
                match line {
                    Some(line) => format!("{part} firmware line {line} {what}"),
                    None => format!("{part} firmware {what}"),
                }
            }
            Diagnostic::NetlistSameNet {
                pin,
                with,
                found,
                expected,
            } => format!(
                "pin {pin} is on `{}`; it must be on `{}`, as {with} is",
                found.as_str(),
                expected.as_str()
            ),
            Diagnostic::NetlistGround { net } => format!("ground `{net}` is not in `nets`"),
            Diagnostic::NetlistAnalysis { field, value, rule } => {
                let must = match rule {
                    AnalysisRule::Positive => "it must be positive",
                    AnalysisRule::StepUnder1ns | AnalysisRule::GridUnder1ns => {
                        "it must be at least 1000"
                    }
                    AnalysisRule::StepOverStop | AnalysisRule::GridOverStop => {
                        "it must not exceed stop_ps"
                    }
                };
                format!("analysis {field} is {value}; {must}")
            }
            Diagnostic::NetlistSchedule {
                part,
                field,
                event,
                rule,
                end_ps,
            } => {
                let what = match rule {
                    ScheduleRule::AtZero => {
                        "is at 0 ps; an event must come after t = 0".to_string()
                    }
                    ScheduleRule::SameState if *field == "events" => {
                        "does not change the contact".to_string()
                    }
                    ScheduleRule::SameState => "does not change the level".to_string(),
                    ScheduleRule::NotIncreasing => "is not after the previous event".to_string(),
                    ScheduleRule::TooClose if *field == "events" => format!(
                        "is less than {} ps after the previous event and its bounce window",
                        MIN_GAP_PS
                    ),
                    ScheduleRule::TooClose => {
                        format!("is less than {} ps after the previous event", MIN_GAP_PS)
                    }
                    ScheduleRule::Overflow if *field == "events" => {
                        "has a bounce window that ends past the largest time".to_string()
                    }
                    ScheduleRule::Overflow => "ends past the largest time".to_string(),
                    ScheduleRule::Overlap => {
                        format!("starts before the previous event ends at {end_ps} ps")
                    }
                };
                format!("{part} {field}[{event}] {what}")
            }
            Diagnostic::NetlistWaveform {
                part,
                field,
                value,
                rule,
            } => match rule {
                WaveformRule::Equal => {
                    let source = field.strip_suffix(".space").unwrap_or(field.as_ref());
                    format!("{part} {source}.mark and {source}.space are equal")
                }
                WaveformRule::AtZero => {
                    format!("{part} {field} is {value}; a jump must come after t = 0")
                }
                WaveformRule::TooClose => format!(
                    "{part} {field} is {value}; two jumps must be at least {} ps apart",
                    MIN_GAP_PS
                ),
                WaveformRule::Overflow => {
                    format!("{part} {field} is {value}; the first fall is past the largest time")
                }
            },
            Diagnostic::NetFloating { net } => format!("{net} has no DC path to ground"),
            Diagnostic::SolverSingular { unknown, .. } => format!("singular matrix at {unknown}"),
            Diagnostic::SolverNonFinite { unknown, .. } => {
                format!("non-finite number at {unknown}")
            }
            Diagnostic::SolverTooLarge {
                unknowns, limit, ..
            } => format!("{unknowns} unknowns exceeds the dense solver limit of {limit}"),
            Diagnostic::SolverStepLimit { limit, .. } => format!(
                "the run accepted {limit} steps, its limit; dense switch or source schedules cost steps, not rows"
            ),
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
            Diagnostic::TraceTooLarge {
                rows,
                limit,
                columns,
                at: None,
            } => format!(
                "the grid gives {rows} rows; the trace holds {limit} rows of {columns} columns; raise grid_ps or shorten stop_ps"
            ),
            Diagnostic::TraceTooLarge {
                limit,
                at: Some(at),
                ..
            } => format!(
                "the trace holds {limit} rows; the step at {} ps did not fit",
                at.as_ps()
            ),
            Diagnostic::TraceRowInterpolated { at, from, to } => format!(
                "the row at {} ps did not solve; it is the line between the steps at {} ps and {} ps",
                at.as_ps(),
                from.as_ps(),
                to.as_ps()
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
            Diagnostic::NetlistFirmware { line, rule, .. } => {
                serde_json::json!({ "field": "firmware", "line": line, "rule": rule })
            }
            Diagnostic::NetlistSameNet {
                found, expected, ..
            } => serde_json::json!({ "expected": expected.as_str(), "found": found.as_str() }),
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
            Diagnostic::SolverStepLimit { limit, .. } => serde_json::json!({ "limit": limit }),
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
            Diagnostic::TraceTooLarge {
                rows,
                limit,
                columns,
                ..
            } => serde_json::json!({ "columns": columns, "limit": limit, "rows": rows }),
            Diagnostic::TraceRowInterpolated { from, to, .. } => {
                serde_json::json!({ "from_ps": from, "to_ps": to })
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
