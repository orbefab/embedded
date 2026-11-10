//! The one condition grammar bound to an open scene: what each condition reads, and
//! whether it holds now.

use std::fmt::Write;

use orbe_engine::name::Ref as EngineRef;

use super::{Open, Outcome, engine_name, fail, failed, quantity, t, unit_of};
use crate::condition::{Condition, Op, State as Want, Unit};
use crate::reply::{Check, Code, Data, Diagnostic, Error, Level, number};
use crate::target::Target;

/// What a condition reads, resolved against the open scene.
pub(super) enum Bound {
    On {
        part: String,
        i: EngineRef,
        above: f64,
        want: bool,
    },
    Level {
        pin: String,
        signal: usize,
        want: Level,
    },
    Compare {
        label: String,
        column: EngineRef,
        op: Op,
        value: f64,
        unit: Unit,
    },
    SerialContains(String),
    NoDiagnostics,
}

impl Open {
    pub(super) fn bind(&self, condition: &Condition) -> Result<Bound, Error> {
        Ok(match condition {
            Condition::NoDiagnostics => Bound::NoDiagnostics,
            Condition::SerialContains(text) => Bound::SerialContains(text.as_str().to_string()),
            Condition::State(r, want) => {
                match (self.resolve(r)?, want) {
                    (Target::Part(path), Want::On | Want::Off) => {
                        let (i, above) = self.on_threshold(&path)?;
                        Bound::On {
                            part: path,
                            i,
                            above,
                            want: *want == Want::On,
                        }
                    }
                    (
                        Target::Pin {
                            path,
                            pin,
                            net: None,
                        },
                        Want::High | Want::Low,
                    ) => {
                        return Err(self.not_simulated(&path, &pin));
                    }
                    (
                        Target::Pin {
                            path,
                            pin,
                            net: Some(net),
                        },
                        Want::High | Want::Low,
                    ) => {
                        let name = format!("{path}.{pin}");
                        let signal = self.signals.get(&net).copied().ok_or_else(|| {
                        Error::new(
                            Code::NoReading,
                            format!("`{name}` has no logic level, because no chip pin shares its net"),
                            format!("compare its voltage instead, such as `{r} > 2.5V`"),
                        )
                    })?;
                        let want = if *want == Want::High {
                            Level::High
                        } else {
                            Level::Low
                        };
                        Bound::Level {
                            pin: name,
                            signal,
                            want,
                        }
                    }
                    (_, want) => {
                        return Err(Error::new(
                            Code::NoReading,
                            format!("{r} cannot be {}", want.as_str()),
                            "a part with an `on` reading is on or off, and a pin on a chip's net is high or low",
                        ));
                    }
                }
            }
            Condition::Compare(r, op, amount) => {
                let (label, column, unit) = match self.resolve(r)? {
                    Target::Pin {
                        path,
                        pin,
                        net: Some(net),
                    } => (
                        format!("{path}.{pin}"),
                        EngineRef::Net(engine_name(&net)),
                        Unit::Volt,
                    ),
                    Target::Quantity {
                        path,
                        engine,
                        quantity: q,
                    } => (
                        format!("{path}.{}", q.as_str()),
                        quantity(&engine, q),
                        unit_of(q),
                    ),
                    Target::Pin {
                        path,
                        pin,
                        net: None,
                    } => return Err(self.not_simulated(&path, &pin)),
                    Target::Part(_) => {
                        return Err(Error::new(
                            Code::NoReading,
                            format!("{r} has no one value to compare"),
                            "compare a pin's voltage or a quantity, such as `@led1.i > 5mA`",
                        ));
                    }
                };
                if unit != amount.unit {
                    return Err(Error::new(
                        Code::NoReading,
                        format!("{r} is in {}, not {}", unit.as_str(), amount.unit.as_str()),
                        format!("write the value in {}", unit.as_str()),
                    ));
                }
                Bound::Compare {
                    label,
                    column,
                    op: *op,
                    value: amount.value,
                    unit,
                }
            }
        })
    }

    /// Whether the condition holds now, and what was seen.
    pub(super) fn observe(&self, bound: &Bound) -> (bool, String) {
        match bound {
            Bound::On {
                part,
                i,
                above,
                want,
            } => {
                let value = self.read(i);
                let on = value > *above;
                (
                    on == *want,
                    format!(
                        "{part} {}, i={} A",
                        if on { "on" } else { "off" },
                        number(value)
                    ),
                )
            }
            Bound::Level { pin, signal, want } => {
                let level = self.level_before_now(*signal);
                (level == *want, format!("{pin} {}", level.as_str()))
            }
            Bound::Compare {
                label,
                column,
                op,
                value,
                unit,
            } => {
                let seen = self.read(column);
                (
                    op.holds(seen, *value),
                    format!("{label} = {} {}", number(seen), unit.as_str()),
                )
            }
            Bound::SerialContains(text) => {
                let found = self.serial.contains(text);
                let mut actual = self.serial.actual();
                if !found {
                    let _ = write!(
                        actual,
                        "; serial can reach orbe up to one step ({}) late",
                        t(self.max_step_ps)
                    );
                }
                (found, actual)
            }
            Bound::NoDiagnostics => {
                let found: Vec<Diagnostic> = self
                    .diagnostics(&self.engine.probe())
                    .into_iter()
                    .filter(Diagnostic::runtime)
                    .collect();
                let codes: Vec<&str> = found.iter().map(|d| d.code.as_str()).collect();
                (
                    found.is_empty(),
                    if codes.is_empty() {
                        format!(
                            "no diagnostics; a chip warning can show up to {} late",
                            t(self.max_step_ps)
                        )
                    } else {
                        codes.join(", ")
                    },
                )
            }
        }
    }

    pub(super) fn expect(&self, condition: &Condition) -> Outcome {
        let bound = self.bind(condition).map_err(fail)?;
        let (passed, actual) = self.observe(&bound);
        let check = Check {
            passed,
            expected: condition.to_string(),
            actual,
        };
        if passed {
            return Ok(Data::Checked(check));
        }
        let error = Error::new(
            Code::ExpectFailed,
            format!("expected {}", check.expected),
            "",
        );
        Err(failed(error, Data::Checked(check)))
    }
}
