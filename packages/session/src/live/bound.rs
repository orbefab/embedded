//! The one condition grammar bound to an open scene: what each condition reads, and
//! whether it holds now.

use std::fmt::Write;

use orbe_engine::name::Ref as EngineRef;

use super::{Open, Outcome, engine_name, fail, failed, quantity, t, unit_of};
use crate::condition::{Condition, Op, Rgb, State as Want, Unit};
use crate::reply::{Check, Code, Data, Diagnostic, Error, Level, Light, current, number};
use crate::target::{Target, one_engine, rgb_reading};

/// What a condition reads, resolved against the open scene.
pub(super) enum Bound {
    Light {
        part: String,
        index: usize,
        want: Light,
    },
    /// `@led1.brightness > 2mA`, the average forward current.
    Brightness {
        label: String,
        index: usize,
        op: Op,
        value: f64,
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
    /// `@servo.angle > 60deg`, the shaft angle in degrees.
    Angle {
        label: String,
        engine: String,
        op: Op,
        value: f64,
    },
    /// `@pixel rgb(255,0,0)`, every LED of the part's chain.
    Color {
        label: String,
        engine: String,
        want: Rgb,
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
                    (Target::Part(path), want) if want.light().is_some() => {
                        let index = self.light.require(&self.refs, &path)?;
                        Bound::Light {
                            part: path,
                            index,
                            want: want.light().expect("just matched"),
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
                            "a part with `lit` and `dim` readings is lit, dim or off, and a pin on a chip's net is high or low",
                        ));
                    }
                }
            }
            Condition::Color(r, want) => {
                let (label, engine) = match self.resolve(r)? {
                    Target::Color {
                        path,
                        reading,
                        engine,
                    } => (format!("{path}.{reading}"), engine),
                    Target::Part(path) => {
                        let part = &self.refs.parts[&path];
                        let Some(reading) = rgb_reading(part) else {
                            return Err(no_color(r));
                        };
                        let engine = one_engine(&path, part, Code::NoReading)?;
                        (format!("{path}.{reading}"), engine)
                    }
                    _ => return Err(no_color(r)),
                };
                Bound::Color {
                    label,
                    engine,
                    want: *want,
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
                    Target::Brightness { path } => {
                        let index = self.light.require(&self.refs, &path)?;
                        if amount.unit != Unit::Ampere {
                            return Err(Error::new(
                                Code::NoReading,
                                format!(
                                    "{r} is in {}, not {}",
                                    Unit::Ampere.as_str(),
                                    amount.unit.as_str()
                                ),
                                format!("write the value in {}", Unit::Ampere.as_str()),
                            ));
                        }
                        return Ok(Bound::Brightness {
                            label: format!("{path}.brightness"),
                            index,
                            op: *op,
                            value: amount.value,
                        });
                    }
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
                    Target::Color { .. } => {
                        return Err(Error::new(
                            Code::NoReading,
                            format!("{r} is a colour, not a value to compare"),
                            format!("expect the colour itself, such as `{r} rgb(255,0,0)`"),
                        ));
                    }
                    Target::Angle {
                        path,
                        reading,
                        engine,
                    } => {
                        if amount.unit != Unit::Degree {
                            return Err(Error::new(
                                Code::NoReading,
                                format!("{r} is in deg, not {}", amount.unit.as_str()),
                                "write the angle in deg, such as `60deg`".to_string(),
                            ));
                        }
                        return Ok(Bound::Angle {
                            label: format!("{path}.{reading}"),
                            engine,
                            op: *op,
                            value: amount.value,
                        });
                    }
                    Target::Param { .. } => {
                        return Err(Error::new(
                            Code::NoReading,
                            format!("{r} is a param, which `expect` does not compare"),
                            format!(
                                "read it with `get {r}`, or expect what it drives, such as a pin's voltage"
                            ),
                        ));
                    }
                    Target::Part(_) => {
                        return Err(Error::new(
                            Code::NoReading,
                            format!("{r} has no one value to compare"),
                            "compare a pin's voltage, a quantity or a brightness, such as `@led1.i > 5mA` or `@led1.brightness > 2mA`",
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
            Bound::Light { part, index, want } => {
                let probe = self.engine.probe();
                let glow = self.light.glow(*index, &probe);
                let instant = probe
                    .get(&self.light.lamps()[*index].column)
                    .expect("a checked column")
                    .get();
                (
                    glow.state == *want,
                    format!(
                        "{part} {}, i={}, brightness={} over {}",
                        glow.state.as_str(),
                        current(instant),
                        current(glow.amperes),
                        t(glow.over_ps)
                    ),
                )
            }
            Bound::Brightness {
                label,
                index,
                op,
                value,
            } => {
                let probe = self.engine.probe();
                let glow = self.light.glow(*index, &probe);
                (
                    op.holds(glow.amperes, *value),
                    format!(
                        "{label} = {} over {}",
                        current(glow.amperes),
                        t(glow.over_ps)
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
                    format!(
                        "{label} = {}",
                        match unit {
                            Unit::Ampere => current(seen),
                            _ => format!("{} {}", number(seen), unit.as_str()),
                        }
                    ),
                )
            }
            Bound::Angle {
                label,
                engine,
                op,
                value,
            } => {
                let seen = self.angle_of(engine);
                (
                    op.holds(seen, *value),
                    format!("{label} = {} deg", number(seen)),
                )
            }
            Bound::Color {
                label,
                engine,
                want,
            } => {
                let colors = self.colors(engine);
                (
                    colors.leds.iter().all(|&led| led == want.0),
                    format!("{label} {}", colors.text()),
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

fn no_color(r: &crate::target::Ref) -> Error {
    Error::new(
        Code::NoReading,
        format!("{r} has no colour"),
        "an addressable LED, a part with a reading of rgb, has a colour, such as `@pixel rgb(255,0,0)`",
    )
}
