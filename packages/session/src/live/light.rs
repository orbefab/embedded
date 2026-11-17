//! An LED's brightness is its average forward current over [`BRIGHTNESS_WINDOW`].
//!
//! Arduino `analogWrite` on pins 9 and 10 is about 490 Hz, so 20 ms covers about ten cycles.
//! A shorter window aliases that PWM. Averaging over the step itself would smear a fade when
//! the step is long, and `get` has no step. The name in a reading is this span, or the time
//! since t = 0 when the session is younger. The state is that average: `lit` above the part's
//! `lit` reading, `dim` above its `dim` reading, and `off` at or below that.

use std::collections::VecDeque;

use orbe_engine::Probe;
use orbe_engine::name::{Quantity, Ref as EngineRef};
use orbe_scene::{RefMap, si_amount};

use super::quantity;
use crate::duration::Duration;
use crate::reply::{Code, Error, Light};
use crate::target::one_engine;

/// How long an LED's brightness averages its current.
pub const BRIGHTNESS_WINDOW: Duration = Duration::from_ms(20);

/// A light-state change is listed once it has held this long, about one cycle of the slowest
/// Arduino PWM, so one sample of that ripple is not listed as lit and then off. `--until` on
/// that state lists it at the slice where the condition holds.
pub(super) const LIGHT_HOLD: Duration = Duration::from_ms(2);

#[derive(Clone, Debug)]
pub(super) struct Lamp {
    pub path: String,
    pub column: EngineRef,
    pub lit: f64,
    pub dim: f64,
}

#[derive(Clone, Copy, Debug)]
struct Span {
    t_ps: u64,
    integral: f64,
    instant: f64,
}

/// The forward-current integral of each lamp, sampled as time advances, kept for one window.
pub(super) struct Log {
    lamps: Vec<Lamp>,
    samples: Vec<VecDeque<Span>>,
}

/// One lamp's brightness at one time.
#[derive(Clone, Copy, Debug)]
pub(super) struct Glow {
    pub state: Light,
    pub amperes: f64,
    pub over_ps: u64,
}

impl Log {
    pub(super) fn build(
        refs: &RefMap,
        reports: &std::collections::BTreeMap<String, Vec<Quantity>>,
    ) -> Log {
        let mut lamps = Vec::new();
        for (path, part) in &refs.parts {
            let Some(lit) = amperes(part, "lit") else {
                continue;
            };
            let Some(dim) = amperes(part, "dim") else {
                continue;
            };
            let Ok(engine) = one_engine(path, part, Code::NoReading) else {
                continue;
            };
            let reported = reports.get(&engine).map(Vec::as_slice).unwrap_or(&[]);
            if !reported.contains(&Quantity::I) {
                continue;
            }
            lamps.push(Lamp {
                path: path.clone(),
                column: quantity(&engine, Quantity::I),
                lit,
                dim,
            });
        }
        let samples = vec![VecDeque::new(); lamps.len()];
        Log { lamps, samples }
    }

    pub(super) fn lamps(&self) -> &[Lamp] {
        &self.lamps
    }

    pub(super) fn position(&self, path: &str) -> Option<usize> {
        self.lamps.iter().position(|lamp| lamp.path == path)
    }

    /// The lamp `path` asks to read, or why the part file states no brightness.
    pub(super) fn require(&self, refs: &RefMap, path: &str) -> Result<usize, Error> {
        if let Some(index) = self.position(path) {
            return Ok(index);
        }
        let part = &refs.parts[path];
        for name in ["lit", "dim"] {
            let Some(reading) = part.readings.get(name) else {
                return Err(missing(path, &part.ty, name));
            };
            let Some(above) = &reading.above else {
                return Err(missing(path, &part.ty, name));
            };
            if reading.of != "i" || above.unit != "A" {
                return Err(Error::new(
                    Code::NoReading,
                    format!(
                        "`{path}`'s `{name}` reading must be `i` in A, not {} in {}",
                        reading.of, above.unit
                    ),
                    format!("fix readings.{name} in {}", part.ty),
                ));
            }
        }
        one_engine(path, part, Code::NoReading)?;
        Err(Error::new(
            Code::NoReading,
            format!("the engine reports no `{path}.i`"),
            format!("`{}` has no forward current to average", part.ty),
        ))
    }

    /// Records each lamp's integral at the probe's time. A second record at the same time
    /// replaces the first. Samples older than the window are dropped, except the newest one
    /// at or before that, which is the anchor [`average`] subtracts.
    pub(super) fn note(&mut self, probe: &Probe) {
        let now = probe.at().as_ps();
        let horizon = now.saturating_sub(BRIGHTNESS_WINDOW.as_ps());
        for (lamp, samples) in self.lamps.iter().zip(self.samples.iter_mut()) {
            let integral = integral(probe, &lamp.column);
            let instant = probe.get(&lamp.column).expect("a checked column").get();
            if let Some(last) = samples.back_mut()
                && last.t_ps == now
            {
                last.integral = integral;
                last.instant = instant;
            } else {
                samples.push_back(Span {
                    t_ps: now,
                    integral,
                    instant,
                });
            }
            while samples.len() >= 2 && samples[1].t_ps <= horizon {
                samples.pop_front();
            }
        }
    }

    pub(super) fn glow(&self, index: usize, probe: &Probe) -> Glow {
        let lamp = &self.lamps[index];
        let now = probe.at().as_ps();
        let instant = probe.get(&lamp.column).expect("a checked column").get();
        let integral_now = integral(probe, &lamp.column);
        let (amperes, over_ps) = average(&self.samples[index], now, integral_now, instant);
        Glow {
            state: Light::from_average(amperes, lamp.lit, lamp.dim),
            amperes,
            over_ps,
        }
    }

    pub(super) fn glows(&self, probe: &Probe) -> Vec<Glow> {
        (0..self.lamps.len())
            .map(|index| self.glow(index, probe))
            .collect()
    }

    /// The timeout hint when `want` is unmet at `end`. `None` when the average does not
    /// yet cover a full window, so the caller asks for a longer step.
    ///
    /// A full window states the measured average, the state that average reads, and the
    /// range `want` needs, then says to check the circuit with `snapshot`.
    pub(super) fn steady_hint(&self, index: usize, want: Light, end: Glow) -> Option<String> {
        if end.over_ps < BRIGHTNESS_WINDOW.as_ps() {
            return None;
        }
        let lamp = &self.lamps[index];
        Some(steady_sentence(&lamp.path, want, end, lamp.lit, lamp.dim))
    }
}

/// `readings.lit` or `readings.dim` when it is a current in amperes.
fn amperes(part: &orbe_scene::MapPart, name: &str) -> Option<f64> {
    let reading = part.readings.get(name)?;
    let above = reading.above.as_ref()?;
    (reading.of == "i" && above.unit == "A").then_some(above.value)
}

fn missing(path: &str, ty: &str, name: &str) -> Error {
    Error::new(
        Code::NoReading,
        format!("`{path}` states no `{name}` reading"),
        format!("{ty} has no readings.{name} with `above` in its part file"),
    )
}

fn integral(probe: &Probe, column: &EngineRef) -> f64 {
    let index = probe
        .refs()
        .iter()
        .position(|r| r == column)
        .expect("a checked column");
    probe.integrals()[index]
}

/// The average from the newest sample at or before `now` minus the window, or from the oldest
/// sample when the session is younger. At one instant, `dt` is 0 and the average is `instant`.
fn average(samples: &VecDeque<Span>, now: u64, integral_now: f64, instant: f64) -> (f64, u64) {
    let horizon = now.saturating_sub(BRIGHTNESS_WINDOW.as_ps());
    let start = samples
        .iter()
        .rev()
        .find(|span| span.t_ps <= horizon)
        .or_else(|| samples.front());
    let Some(start) = start else {
        return (instant, 0);
    };
    let dt = now.saturating_sub(start.t_ps);
    if dt == 0 {
        return (instant, 0);
    }
    ((integral_now - start.integral) / dt as f64, dt)
}

/// What a full window measured, and the range `want` needs.
fn steady_sentence(part: &str, want: Light, end: Glow, lit: f64, dim: f64) -> String {
    let need = match want {
        Light::Lit => format!("lit needs the average above {}", si_amount(lit, "A")),
        Light::Dim => format!(
            "dim needs the average above {} and at or below {}",
            si_amount(dim, "A"),
            si_amount(lit, "A")
        ),
        Light::Off => format!("off needs the average at or below {}", si_amount(dim, "A")),
    };
    let how = match end.state {
        Light::Lit => "is lit",
        Light::Dim => "glows dimly",
        Light::Off => "is off",
    };
    format!(
        "{part} carries {} averaged over the last {} and {how}. {need}. Check the circuit with snapshot.",
        amperes_text(end.amperes, lit, dim),
        Duration::from_ps(end.over_ps),
    )
}

/// Three significant figures. From 0.1 mA up this is milliamps, so 0.34 mA rather than 340 uA.
/// Trailing zeros are dropped. When that rounding would fall in another light state than the
/// exact current, more digits are kept so the printed number stays in the real state. The
/// state is a strict `>` of each threshold, the same rule as [`Light::from_average`].
fn amperes_text(amperes: f64, lit: f64, dim: f64) -> String {
    let state = Light::from_average(amperes, lit, dim);
    let abs = amperes.abs();
    let (scale, unit) = if abs == 0.0 || abs >= 1e-4 {
        (1e-3, "mA")
    } else if abs >= 1e-6 {
        (1e-6, "uA")
    } else if abs >= 1e-9 {
        (1e-9, "nA")
    } else {
        (1.0, "A")
    };
    let value = amperes / scale;
    let start = if value.abs() >= 100.0 {
        0
    } else if value.abs() >= 10.0 {
        1
    } else {
        2
    };
    for extra in 0..=16 {
        let digits = start + extra;
        let rendered = format!("{value:.digits$}");
        let text = if rendered.contains('.') {
            rendered
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string()
        } else {
            rendered
        };
        if printed_state(&text, scale, lit, dim) == Some(state) {
            return format!("{text} {unit}");
        }
    }
    format!("{value} {unit}")
}

/// The light state of the number `text` denotes, in the unit `scale` amperes per unit.
fn printed_state(text: &str, scale: f64, lit: f64, dim: f64) -> Option<Light> {
    let value: f64 = text.parse().ok()?;
    Some(Light::from_average(value * scale, lit, dim))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(t_ms: u64, amperes: f64) -> Span {
        let t_ps = t_ms * 1_000_000_000;
        Span {
            t_ps,
            integral: amperes * t_ps as f64,
            instant: amperes,
        }
    }

    #[test]
    fn a_constant_current_averages_to_itself_over_the_window() {
        let samples: VecDeque<Span> = [span(0, 0.01), span(80, 0.01), span(100, 0.01)]
            .into_iter()
            .collect();
        let now = 100_000_000_000;
        let (amperes, over) = average(&samples, now, 0.01 * now as f64, 0.01);
        assert!((amperes - 0.01).abs() < 1e-12, "{amperes}");
        assert_eq!(over, BRIGHTNESS_WINDOW.as_ps());
    }

    #[test]
    fn a_young_session_averages_since_t_0_and_an_instant_is_the_current() {
        let samples: VecDeque<Span> = [span(0, 0.0)].into_iter().collect();
        let (amperes, over) = average(&samples, 0, 0.0, 3.4e-4);
        assert_eq!((amperes, over), (3.4e-4, 0));
        let now = 4_000_000_000;
        let (amperes, over) = average(&samples, now, 3.4e-4 * now as f64, 3.4e-4);
        assert!((amperes - 3.4e-4).abs() < 1e-12, "{amperes}");
        assert_eq!(over, now);
    }

    #[test]
    fn the_first_window_uses_the_current_at_t_0() {
        let samples: VecDeque<Span> = [span(0, 3.5e-4), span(20, 3.5e-4)].into_iter().collect();
        let (amperes, over) = average(&samples, 0, 0.0, 3.5e-4);
        assert_eq!((amperes, over), (3.5e-4, 0));
        assert_eq!(Light::from_average(amperes, 0.002, 0.0001), Light::Dim);
        let now = 20_000_000_000;
        let (amperes, over) = average(&samples, now, 3.5e-4 * now as f64, 3.5e-4);
        assert_eq!(over, BRIGHTNESS_WINDOW.as_ps());
        assert_eq!(amperes, 3.5e-4);
    }

    /// Samples every 1 ms through `until_ms`. The window average at each sample is `avg_at`.
    fn series(until_ms: u64, avg_at: impl Fn(u64) -> f64) -> VecDeque<Span> {
        let mut integral = vec![0.0; until_ms as usize + 1];
        for ms in 1..=until_ms {
            let avg = avg_at(ms);
            let t_ps = ms as f64 * 1_000_000_000.0;
            if ms >= 20 {
                integral[ms as usize] = integral[ms as usize - 20] + avg * 20_000_000_000.0;
            } else {
                integral[ms as usize] = avg * t_ps;
            }
        }
        (0..=until_ms)
            .map(|ms| Span {
                t_ps: ms * 1_000_000_000,
                integral: integral[ms as usize],
                instant: avg_at(ms),
            })
            .collect()
    }

    fn flat(amperes: f64, until_ms: u64) -> VecDeque<Span> {
        series(until_ms, |_| amperes)
    }

    /// Low for `low_ms`, then high at `peak` for `high_ms`, repeating. The level on the
    /// millisecond that ends at a sample is the level before that sample.
    fn square(low_ms: u64, high_ms: u64, peak: f64, until_ms: u64) -> VecDeque<Span> {
        let period = low_ms + high_ms;
        series(until_ms, |t| {
            if t == 0 {
                return 0.0;
            }
            let from = t.saturating_sub(20);
            let mut on = 0u64;
            for k in (from + 1)..=t {
                if (k - 1) % period >= low_ms {
                    on += 1;
                }
            }
            peak * on as f64 / (t - from) as f64
        })
    }

    fn lamp_log(path: &str, samples: VecDeque<Span>) -> Log {
        Log {
            lamps: vec![Lamp {
                path: path.into(),
                column: "#led1.i".parse().expect("a current ref"),
                lit: 0.002,
                dim: 0.0001,
            }],
            samples: vec![samples],
        }
    }

    fn reading(log: &Log, ms: u64) -> Glow {
        let lamp = &log.lamps[0];
        let t_ps = ms * 1_000_000_000;
        let span = log.samples[0]
            .iter()
            .rev()
            .find(|span| span.t_ps <= t_ps)
            .expect("a sample");
        let (amperes, over_ps) = average(&log.samples[0], span.t_ps, span.integral, span.instant);
        Glow {
            state: Light::from_average(amperes, lamp.lit, lamp.dim),
            amperes,
            over_ps,
        }
    }

    fn hint(log: &Log, want: Light) -> Option<String> {
        let end_ms = log.samples[0].back().expect("a sample").t_ps / 1_000_000_000;
        log.steady_hint(0, want, reading(log, end_ms))
    }

    fn measured(part: &str, current: &str, how: &str, need: &str) -> String {
        format!(
            "{part} carries {current} averaged over the last 20ms and {how}. {need}. Check the circuit with snapshot."
        )
    }

    #[test]
    fn a_full_window_names_the_measured_average_for_lit_dim_and_off() {
        let dim = lamp_log("led1", flat(3.4156866910256e-4, 40));
        assert_eq!(
            hint(&dim, Light::Lit).unwrap(),
            measured(
                "led1",
                "0.34 mA",
                "glows dimly",
                "lit needs the average above 2 mA",
            )
        );
        let first = lamp_log("led10k", flat(3.5e-4, 20));
        assert_eq!(
            hint(&first, Light::Lit).unwrap(),
            measured(
                "led10k",
                "0.35 mA",
                "glows dimly",
                "lit needs the average above 2 mA",
            )
        );
        let lit = lamp_log("led220", flat(0.0139, 40));
        assert_eq!(
            hint(&lit, Light::Off).unwrap(),
            measured(
                "led220",
                "13.9 mA",
                "is lit",
                "off needs the average at or below 100 uA",
            )
        );
        assert_eq!(
            hint(&lit, Light::Dim).unwrap(),
            measured(
                "led220",
                "13.9 mA",
                "is lit",
                "dim needs the average above 100 uA and at or below 2 mA",
            )
        );
        let from_zero = lamp_log("led220", flat(0.0139, 30));
        assert_eq!(
            hint(&from_zero, Light::Off).unwrap(),
            hint(&lit, Light::Off).unwrap()
        );
        let off = lamp_log("led1", flat(3.76e-5, 40));
        assert_eq!(
            hint(&off, Light::Lit).unwrap(),
            measured(
                "led1",
                "37.6 uA",
                "is off",
                "lit needs the average above 2 mA",
            )
        );
    }

    #[test]
    fn a_short_window_asks_for_a_longer_step_and_a_moving_one_names_its_average() {
        let young = lamp_log("led1", flat(1e-3, 5));
        assert!(hint(&young, Light::Lit).is_none());
        let moving = lamp_log("led1", series(40, |ms| 1e-3 + 0.5e-3 * (ms as f64) / 40.0));
        assert_eq!(
            hint(&moving, Light::Lit).unwrap(),
            measured(
                "led1",
                "1.5 mA",
                "glows dimly",
                "lit needs the average above 2 mA",
            )
        );
    }

    #[test]
    fn a_pulse_between_equal_windows_names_the_end_average() {
        let peak = 3.242585439509286e-3;
        let half = measured(
            "led1",
            "1.62 mA",
            "glows dimly",
            "lit needs the average above 2 mA",
        );
        let log = lamp_log("led1", square(20, 20, peak, 50));
        assert_eq!(reading(&log, 40).state, Light::Lit);
        assert_eq!(reading(&log, 50).state, Light::Dim);
        assert_eq!(hint(&log, Light::Lit).unwrap(), half);

        let crossed = lamp_log(
            "led1",
            series(50, |ms| if ms == 40 { 2.01e-3 } else { 1.99e-3 }),
        );
        assert_eq!(reading(&crossed, 40).state, Light::Lit);
        assert_eq!(
            hint(&crossed, Light::Lit).unwrap(),
            measured(
                "led1",
                "1.99 mA",
                "glows dimly",
                "lit needs the average above 2 mA",
            )
        );
    }

    #[test]
    fn a_ten_millisecond_square_wave_names_its_average_on_a_full_window() {
        let peak = 3.242585439509286e-3;
        let named = measured(
            "led1",
            "1.62 mA",
            "glows dimly",
            "lit needs the average above 2 mA",
        );
        for until in [30, 40, 50, 60] {
            let log = lamp_log("led1", square(10, 10, peak, until));
            assert_eq!(hint(&log, Light::Lit).unwrap(), named, "until {until} ms");
        }
    }

    /// 1650 ohm is lit at 2.0001 mA. Three figures print `2 mA`, which reads as dim.
    /// 36440 ohm is dim at 100.025 uA. Three figures print `0.1 mA`, which reads as off.
    #[test]
    fn a_current_next_to_a_threshold_keeps_enough_digits_to_stay_in_its_state() {
        let lit = 0.0020001029114928505;
        let dim = 0.00010002478304662982;
        let glow = |amperes: f64| Glow {
            state: Light::from_average(amperes, 0.002, 0.0001),
            amperes,
            over_ps: BRIGHTNESS_WINDOW.as_ps(),
        };
        assert_eq!(glow(lit).state, Light::Lit);
        assert_eq!(glow(dim).state, Light::Dim);
        assert_eq!(
            steady_sentence("led1", Light::Dim, glow(lit), 0.002, 0.0001),
            measured(
                "led1",
                "2.0001 mA",
                "is lit",
                "dim needs the average above 100 uA and at or below 2 mA",
            )
        );
        assert_eq!(
            steady_sentence("led1", Light::Off, glow(dim), 0.002, 0.0001),
            measured(
                "led1",
                "0.10002 mA",
                "glows dimly",
                "off needs the average at or below 100 uA",
            )
        );
    }
}
