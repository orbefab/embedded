//! The one condition grammar bound to an open scene: what each condition reads, and
//! whether it holds now.

use std::fmt::Write;
use std::path::Path;

use orbe_engine::name::Ref as EngineRef;

use super::{Open, Outcome, engine_name, fail, failed, quantity, t, unit_of};
use crate::condition::{Condition, Op, Rgb, State as Want, Unit};
use crate::reply::{Check, Code, Data, Diagnostic, Error, Level, Light, current, number};
use crate::target::{Target, lcd_engine, one_engine, rgb_reading, screen_reading, text_reading};

/// What a condition reads, resolved against the open scene.
pub(super) enum Bound {
    Light {
        part: String,
        index: usize,
        want: Light,
    },
    /// `@relay closed` or `@relay open`.
    Contact {
        label: String,
        signal: usize,
        closed: bool,
    },
    /// `@led1.brightness > 2mA`, the average forward current.
    Brightness {
        label: String,
        index: usize,
        op: Op,
        value: f64,
    },
    /// `@matrix.brightness > 2mA`, the average lit-dot current of a MAX7219 matrix.
    MatrixBrightness {
        label: String,
        path: String,
        op: Op,
        value: f64,
    },
    /// `@bz.hz > 400Hz`, the frequency across a part whose reading is `"of": "hz"`.
    Hertz {
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
    /// `@oled.display.screen lit` or `off`: whether any pixel of the glass is lit.
    ScreenLit {
        label: String,
        engines: Vec<String>,
        want: bool,
    },
    /// `@oled.display.screen shows <file>`: each row of the glass, as the file writes it.
    Shows {
        label: String,
        engines: Vec<String>,
        file: String,
        rows: Vec<String>,
    },
    /// `@lcd.text contains "Hello"`.
    TextContains {
        label: String,
        engine: String,
        needle: String,
    },
    /// `@lcd.text line 0 "Hello"`.
    TextLine {
        label: String,
        engine: String,
        row: u8,
        want: String,
    },
    /// `@lcd.text blank`.
    TextBlank {
        label: String,
        engine: String,
    },
    /// `@lcd.backlight lit` or `off`.
    Backlight {
        label: String,
        engine: String,
        want: bool,
    },
    SerialContains(String),
    DiagnosticContains(String),
    NoDiagnostics,
    /// `@disp.digit 8`.
    Digit {
        path: String,
        want: String,
    },
}

impl Open {
    pub(super) fn bind(&self, condition: &Condition) -> Result<Bound, Error> {
        Ok(match condition {
            Condition::NoDiagnostics => Bound::NoDiagnostics,
            Condition::DiagnosticContains(text) => {
                Bound::DiagnosticContains(text.as_str().to_string())
            }
            Condition::Digit(r, want) => match self.resolve(r)? {
                Target::Digit { path } => {
                    self.shown_digit(&path)?;
                    Bound::Digit {
                        path,
                        want: want.clone(),
                    }
                }
                _ => {
                    return Err(Error::new(
                        Code::NoReading,
                        format!("{r} is not a 7-segment digit"),
                        "expect a digit on a part whose readings include digit, such as `@disp.digit 8`",
                    ));
                }
            },
            Condition::SerialContains(text) => Bound::SerialContains(text.as_str().to_string()),
            Condition::Shows(r, file) => {
                let (label, engines) = self.screen_target(r)?;
                let rows = self.picture(file)?;
                Bound::Shows {
                    label,
                    engines,
                    file: file.clone(),
                    rows,
                }
            }
            Condition::TextContains(r, needle) => {
                let (label, engine) = self.text_target(r)?;
                Bound::TextContains {
                    label,
                    engine,
                    needle: needle.as_str().to_string(),
                }
            }
            Condition::TextLine(r, row, want) => {
                let (label, engine) = self.text_target(r)?;
                Bound::TextLine {
                    label,
                    engine,
                    row: *row,
                    want: want.clone(),
                }
            }
            Condition::TextBlank(r) => {
                let (label, engine) = self.text_target(r)?;
                Bound::TextBlank { label, engine }
            }
            Condition::State(r, want) => {
                match (self.resolve(r)?, want) {
                    (Target::Part(path), Want::Closed | Want::Open) => {
                        let signal = self.contact_signal(&path)?;
                        Bound::Contact {
                            label: path,
                            signal,
                            closed: *want == Want::Closed,
                        }
                    }
                    (
                        Target::Screen {
                            path,
                            reading,
                            engine,
                        },
                        Want::Lit | Want::Off,
                    ) => Bound::ScreenLit {
                        label: format!("{path}.{reading}"),
                        engines: vec![engine],
                        want: *want == Want::Lit,
                    },
                    (
                        Target::Backlight {
                            path,
                            reading,
                            engine,
                        },
                        Want::Lit | Want::Off,
                    ) => Bound::Backlight {
                        label: format!("{path}.{reading}"),
                        engine,
                        want: *want == Want::Lit,
                    },
                    (Target::Part(path), want)
                        if matches!(want, Want::Lit | Want::Off)
                            && !self.refs.parts[&path].readings.contains_key("lit")
                            && !self.ordered_screen_engines(&path).is_empty() =>
                    {
                        let engines = self.ordered_screen_engines(&path);
                        Bound::ScreenLit {
                            label: path,
                            engines,
                            want: *want == Want::Lit,
                        }
                    }
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
                            "a part with `lit` and `dim` readings is lit, dim or off, a relay contact is closed or open, and a pin on a chip's net is high or low",
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
                    Target::MatrixBrightness { path } => {
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
                        return Ok(Bound::MatrixBrightness {
                            label: format!("{path}.brightness"),
                            path,
                            op: *op,
                            value: amount.value,
                        });
                    }
                    Target::Hertz { path } => {
                        let index = self.tone.position(&path).ok_or_else(|| {
                            Error::new(
                                Code::NoReading,
                                format!("`{path}` has no frequency reading"),
                                "the part file's reading `of` `hz` needs a voltage the engine reports",
                            )
                        })?;
                        if amount.unit != Unit::Hertz {
                            return Err(Error::new(
                                Code::NoReading,
                                format!(
                                    "{r} is in {}, not {}",
                                    Unit::Hertz.as_str(),
                                    amount.unit.as_str()
                                ),
                                format!("write the value in {}", Unit::Hertz.as_str()),
                            ));
                        }
                        return Ok(Bound::Hertz {
                            label: format!("{path}.{}", self.tone.reading(index)),
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
                    Target::Screen { .. } => {
                        return Err(Error::new(
                            Code::NoReading,
                            format!("{r} is a screen, not a value to compare"),
                            format!(
                                "expect what it shows, such as `{r} lit` or `{r} shows tests/screen.txt`"
                            ),
                        ));
                    }
                    Target::Text { .. } => {
                        return Err(Error::new(
                            Code::NoReading,
                            format!("{r} is LCD text, not a value to compare"),
                            format!(
                                "expect what it shows, such as `{r} contains \"Hello\"` or `{r} line 0 \"Hello\"`"
                            ),
                        ));
                    }
                    Target::Backlight { .. } => {
                        return Err(Error::new(
                            Code::NoReading,
                            format!("{r} is a backlight, not a value to compare"),
                            format!("expect its state, such as `{r} lit` or `{r} off`"),
                        ));
                    }
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
                    Target::Digit { .. } => {
                        return Err(Error::new(
                            Code::NoReading,
                            format!("{r} is a digit, not a value to compare"),
                            format!("expect the digit itself, such as `{r} 8`"),
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
            Bound::MatrixBrightness {
                label,
                path,
                op,
                value,
            } => {
                let seen = self.matrix_brightness(path);
                (
                    op.holds(seen, *value),
                    format!("{label} = {}", current(seen)),
                )
            }
            Bound::Hertz {
                label,
                index,
                op,
                value,
            } => {
                let hz = self.tone.hz(*index, self.engine.now().as_ps());
                (op.holds(hz, *value), format!("{label} = {} Hz", number(hz)))
            }
            Bound::Contact {
                label,
                signal,
                closed,
            } => {
                let level = self.level_before_now(*signal);
                let is_closed = level == Level::High;
                (
                    is_closed == *closed,
                    format!("{label} {}", if is_closed { "closed" } else { "open" }),
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
            Bound::ScreenLit {
                label,
                engines,
                want,
            } => {
                let screen = self.screens(engines);
                (
                    (screen.lit > 0) == *want,
                    format!("{label} {}", screen.brief()),
                )
            }
            Bound::Shows {
                label,
                engines,
                file,
                rows,
            } => {
                let screen = self.screens(engines);
                let differs = (0..rows.len().max(screen.rows.len()))
                    .find(|&k| rows.get(k) != screen.rows.get(k));
                match differs {
                    None => (true, format!("{label} {}, as {file}", screen.brief())),
                    Some(k) => (
                        false,
                        format!(
                            "{label} {}; row {k} is `{}`, and {file} has `{}`",
                            screen.brief(),
                            screen.rows.get(k).map_or("", String::as_str),
                            rows.get(k).map_or("", String::as_str),
                        ),
                    ),
                }
            }
            Bound::TextContains {
                label,
                engine,
                needle,
            } => {
                let lcd = self.lcd_text(engine);
                let joined = lcd.text();
                (
                    joined.contains(needle.as_str()),
                    format!("{label} is {}", lcd.brief()),
                )
            }
            Bound::TextLine {
                label,
                engine,
                row,
                want,
            } => {
                let lcd = self.lcd_text(engine);
                let actual = lcd.lines[usize::from(*row)].trim_end();
                (
                    actual == want.trim_end(),
                    format!("{label} line {row} is {actual:?} ({})", lcd.brief()),
                )
            }
            Bound::TextBlank { label, engine } => {
                let lcd = self.lcd_text(engine);
                let blank = lcd.lines.iter().all(|line| line.trim().is_empty());
                (blank, format!("{label} is {}", lcd.brief()))
            }
            Bound::Backlight {
                label,
                engine,
                want,
            } => {
                let on = self.lcd_backlight(engine);
                (
                    on == *want,
                    format!("{label} {}", if on { "lit" } else { "off" }),
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
            Bound::Digit { path, want } => {
                let shown = self.shown_digit(path).expect("bind checked the segments");
                (shown == *want, format!("{path}.digit is {shown}"))
            }
            Bound::DiagnosticContains(text) => {
                let found: Vec<String> = self
                    .diagnostics(&self.engine.probe())
                    .into_iter()
                    .filter(Diagnostic::runtime)
                    .map(|d| d.text())
                    .collect();
                let matched = found.iter().any(|line| line.contains(text));
                let actual = if found.is_empty() {
                    format!(
                        "no diagnostics; a chip warning can show up to {} late",
                        t(self.max_step_ps)
                    )
                } else {
                    found.join("; ")
                };
                (matched, actual)
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

impl Open {
    /// The screen(s) a `shows` condition reads, as its label and its engine part(s).
    fn screen_target(&self, r: &crate::target::Ref) -> Result<(String, Vec<String>), Error> {
        match self.resolve(r)? {
            Target::Screen {
                path,
                reading,
                engine,
            } => Ok((format!("{path}.{reading}"), vec![engine])),
            Target::Part(path) => {
                let engines = self.ordered_screen_engines(&path);
                if engines.is_empty() {
                    return Err(no_screen(r));
                }
                let part = &self.refs.parts[&path];
                let label = match screen_reading(part) {
                    Some(reading) => format!("{path}.{reading}"),
                    None => path,
                };
                Ok((label, engines))
            }
            _ => Err(no_screen(r)),
        }
    }

    /// The character LCD a text condition reads, as its label and its engine part.
    fn text_target(&self, r: &crate::target::Ref) -> Result<(String, String), Error> {
        match self.resolve(r)? {
            Target::Text {
                path,
                reading,
                engine,
            } => Ok((format!("{path}.{reading}"), engine)),
            Target::Part(path) => {
                let part = &self.refs.parts[&path];
                let Some(reading) = text_reading(part) else {
                    return Err(no_text(r));
                };
                let engine = lcd_engine(&self.refs, &path)?;
                Ok((format!("{path}.{reading}"), engine))
            }
            _ => Err(no_text(r)),
        }
    }

    /// The rows of `file`, by its path from the scene's folder: one line a row, `#` lit and
    /// `.` dark, as `get` writes them.
    fn picture(&self, file: &str) -> Result<Vec<String>, Error> {
        let dir = Path::new(&self.scene).parent().unwrap_or(Path::new(""));
        let text = self.files.read_to_string(&dir.join(file)).map_err(|err| {
            Error::new(
                Code::NoReading,
                format!("cannot read `{file}`: {err}"),
                "name a file by its path from the scene's folder, with the rows `get` prints for the screen",
            )
        })?;
        Ok(text
            .lines()
            .map(|row| row.trim_end().to_string())
            .filter(|row| !row.is_empty())
            .collect())
    }
}

fn no_screen(r: &crate::target::Ref) -> Error {
    Error::new(
        Code::NoReading,
        format!("{r} is not a screen"),
        "a display, a part with a reading of screen, shows rows, such as `@oled.display.screen shows tests/hello.screen`",
    )
}

fn no_text(r: &crate::target::Ref) -> Error {
    Error::new(
        Code::NoReading,
        format!("{r} has no LCD text"),
        "a character LCD with a reading of text shows lines, such as `@lcd.text contains \"Hello\"` or `@lcd.text line 0 \"Hello\"`",
    )
}

fn no_color(r: &crate::target::Ref) -> Error {
    Error::new(
        Code::NoReading,
        format!("{r} has no colour"),
        "an addressable LED, a part with a reading of rgb, has a colour, such as `@pixel rgb(255,0,0)`",
    )
}
