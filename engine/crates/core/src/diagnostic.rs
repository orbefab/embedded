use serde::{Serialize, Serializer};

use crate::name::{NameError, Ref};
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
            Diagnostic::NetFloating { .. } => "net.floating",
            Diagnostic::SolverSingular { .. } => "solver.singular",
            Diagnostic::SolverNonFinite { .. } => "solver.nonfinite",
            Diagnostic::SolverTooLarge { .. } => "solver.too_large",
        }
    }

    pub fn severity(&self) -> Severity {
        match self {
            Diagnostic::NetFloating { .. } => Severity::Warning,
            Diagnostic::NetlistSyntax { .. }
            | Diagnostic::NetlistVersion { .. }
            | Diagnostic::NetlistName { .. }
            | Diagnostic::NetlistDuplicate { .. }
            | Diagnostic::NetlistUnknownNet { .. }
            | Diagnostic::NetlistPins { .. }
            | Diagnostic::NetlistUnit { .. }
            | Diagnostic::NetlistValue { .. }
            | Diagnostic::NetlistGround { .. }
            | Diagnostic::SolverSingular { .. }
            | Diagnostic::SolverNonFinite { .. }
            | Diagnostic::SolverTooLarge { .. } => Severity::Error,
        }
    }

    pub fn at(&self) -> Option<Time> {
        match self {
            Diagnostic::SolverSingular { at, .. }
            | Diagnostic::SolverNonFinite { at, .. }
            | Diagnostic::SolverTooLarge { at, .. } => Some(*at),
            Diagnostic::NetlistSyntax { .. }
            | Diagnostic::NetlistVersion { .. }
            | Diagnostic::NetlistName { .. }
            | Diagnostic::NetlistDuplicate { .. }
            | Diagnostic::NetlistUnknownNet { .. }
            | Diagnostic::NetlistPins { .. }
            | Diagnostic::NetlistUnit { .. }
            | Diagnostic::NetlistValue { .. }
            | Diagnostic::NetlistGround { .. }
            | Diagnostic::NetFloating { .. } => None,
        }
    }

    pub fn refs(&self) -> Vec<Ref> {
        match self {
            Diagnostic::NetlistSyntax { .. }
            | Diagnostic::NetlistVersion { .. }
            | Diagnostic::NetlistName { .. }
            | Diagnostic::NetlistGround { .. } => Vec::new(),
            Diagnostic::NetlistDuplicate { name } => vec![name.clone()],
            Diagnostic::NetlistUnknownNet { pin, .. } => vec![pin.clone()],
            Diagnostic::NetlistPins { part, .. }
            | Diagnostic::NetlistUnit { part, .. }
            | Diagnostic::NetlistValue { part, .. } => vec![part.clone()],
            Diagnostic::NetFloating { net } => vec![net.clone()],
            Diagnostic::SolverSingular { unknown, .. }
            | Diagnostic::SolverNonFinite { unknown, .. } => vec![unknown.clone()],
            Diagnostic::SolverTooLarge { .. } => Vec::new(),
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
            } => match rule {
                ValueRule::Positive => format!("{part} {field} is {value}; it must be positive"),
                ValueRule::FiniteReciprocal => {
                    format!("{part} {field} is {value}; its reciprocal must be finite")
                }
            },
            Diagnostic::NetlistGround { net } => format!("ground `{net}` is not in `nets`"),
            Diagnostic::NetFloating { net } => format!("{net} has no DC path to ground"),
            Diagnostic::SolverSingular { unknown, .. } => format!("singular matrix at {unknown}"),
            Diagnostic::SolverNonFinite { unknown, .. } => {
                format!("non-finite number at {unknown}")
            }
            Diagnostic::SolverTooLarge {
                unknowns, limit, ..
            } => format!("{unknowns} unknowns exceeds the dense solver limit of {limit}"),
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
            Diagnostic::SolverSingular { .. } | Diagnostic::SolverNonFinite { .. } => {
                serde_json::json!({})
            }
            Diagnostic::SolverTooLarge {
                unknowns, limit, ..
            } => serde_json::json!({ "limit": limit, "unknowns": unknowns }),
        }
    }
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
        assert_eq!(seen.len(), 13);
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
            ]
        );
        for diagnostic in &found {
            let warning = diagnostic.code() == "net.floating";
            assert_eq!(diagnostic.severity() == Severity::Warning, warning);
            if diagnostic.code().starts_with("solver.") {
                assert_eq!(diagnostic.at(), Some(Time::ZERO));
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
}
