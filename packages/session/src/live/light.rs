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

/// The forward-current integral of each lamp, sampled as time advances, kept for two windows.
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
    pub at_ps: u64,
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
    /// replaces the first. Samples older than two windows are dropped, except the newest one
    /// at or before that. The current average needs the first window, and a timeout hint
    /// compares it with the average one window earlier.
    pub(super) fn note(&mut self, probe: &Probe) {
        let now = probe.at().as_ps();
        let horizon = now.saturating_sub(BRIGHTNESS_WINDOW.as_ps().saturating_mul(2));
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
            at_ps: now,
        }
    }

    pub(super) fn glows(&self, probe: &Probe) -> Vec<Glow> {
        (0..self.lamps.len())
            .map(|index| self.glow(index, probe))
            .collect()
    }

    /// The brightness at the newest sample at or before `t_ps`. Used to see whether the
    /// average one window ago still matches, after an edge that the `--until` itself started on.
    pub(super) fn glow_at(&self, index: usize, t_ps: u64) -> Option<Glow> {
        let lamp = &self.lamps[index];
        let samples = &self.samples[index];
        let end = samples.iter().rev().find(|span| span.t_ps <= t_ps)?;
        let (amperes, over_ps) = average(samples, end.t_ps, end.integral, end.instant);
        Some(Glow {
            state: Light::from_average(amperes, lamp.lit, lamp.dim),
            amperes,
            over_ps,
            at_ps: end.t_ps,
        })
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

/// The timeout hint when a light condition is unmet and the last two windows agree.
/// `start` is one window before `end`, not the moment `--until` began: that moment is often
/// the edge that turned the LED on. The sentence names the span those windows cover. `None`
/// asks for the ordinary "step longer" hint: the window is not full yet, or the average is
/// still moving.
pub(super) fn steady_hint(
    part: &str,
    want: Light,
    start: Glow,
    end: Glow,
    lit: f64,
    dim: f64,
) -> Option<String> {
    if end.over_ps < BRIGHTNESS_WINDOW.as_ps() {
        return None;
    }
    let scale = start.amperes.abs().max(end.amperes.abs()).max(1e-6);
    if (start.amperes - end.amperes).abs() > scale * 0.02 {
        return None;
    }
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
    let held = end
        .at_ps
        .saturating_sub(start.at_ps)
        .saturating_add(start.over_ps);
    Some(format!(
        "{part} carries {} over {} and {how}. {need}. The average has held at that value for the last {}.",
        amperes_text(end.amperes),
        Duration::from_ps(end.over_ps),
        Duration::from_ps(held),
    ))
}

/// Three significant figures. From 0.1 mA up this is milliamps, so 0.34 mA rather than 340 uA.
fn amperes_text(amperes: f64) -> String {
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
    let digits = if value.abs() >= 100.0 {
        0
    } else if value.abs() >= 10.0 {
        1
    } else {
        2
    };
    let mut text = format!("{value:.digits$}");
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    format!("{text} {unit}")
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

    fn glow_at(amperes: f64, state: Light, over_ms: u64, at_ms: u64) -> Glow {
        Glow {
            state,
            amperes,
            over_ps: Duration::from_ms(over_ms).as_ps(),
            at_ps: Duration::from_ms(at_ms).as_ps(),
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
        let log = Log {
            lamps: vec![Lamp {
                path: "led10k".into(),
                column: "#led10k.i".parse().expect("a current ref"),
                lit: 0.002,
                dim: 0.0001,
            }],
            samples: vec![[span(0, 3.5e-4), span(20, 3.5e-4)].into()],
        };
        let at_start = log.glow_at(0, 0).expect("the t = 0 sample");
        assert_eq!(at_start.state, Light::Dim);
        assert_eq!(at_start.over_ps, 0);
        assert_eq!(at_start.amperes, 3.5e-4);
        let window = log.glow_at(0, 20_000_000_000).expect("the 20 ms sample");
        assert_eq!(window.over_ps, BRIGHTNESS_WINDOW.as_ps());
        assert_eq!(window.amperes, 3.5e-4);
    }

    #[test]
    fn a_settled_current_is_named_for_lit_dim_and_off() {
        let dim_then = glow_at(3.4156866910256e-4, Light::Dim, 20, 20);
        let dim_now = glow_at(3.4156866910256e-4, Light::Dim, 20, 40);
        assert_eq!(
            steady_hint("led1", Light::Lit, dim_then, dim_now, 0.002, 0.0001).unwrap(),
            "led1 carries 0.34 mA over 20ms and glows dimly. lit needs the average above 2 mA. The average has held at that value for the last 40ms."
        );
        let first = glow_at(3.5e-4, Light::Dim, 0, 0);
        let full = glow_at(3.5e-4, Light::Dim, 20, 20);
        assert_eq!(
            steady_hint("led10k", Light::Lit, first, full, 0.002, 0.0001).unwrap(),
            "led10k carries 0.35 mA over 20ms and glows dimly. lit needs the average above 2 mA. The average has held at that value for the last 20ms."
        );
        let lit = glow_at(0.0139, Light::Lit, 20, 40);
        let lit_then = glow_at(0.0139, Light::Lit, 20, 20);
        assert_eq!(
            steady_hint("led220", Light::Off, lit_then, lit, 0.002, 0.0001).unwrap(),
            "led220 carries 13.9 mA over 20ms and is lit. off needs the average at or below 100 uA. The average has held at that value for the last 40ms."
        );
        assert_eq!(
            steady_hint("led220", Light::Dim, lit_then, lit, 0.002, 0.0001).unwrap(),
            "led220 carries 13.9 mA over 20ms and is lit. dim needs the average above 100 uA and at or below 2 mA. The average has held at that value for the last 40ms."
        );
        let off_then = glow_at(3.76e-5, Light::Off, 20, 20);
        let off = glow_at(3.76e-5, Light::Off, 20, 40);
        assert_eq!(
            steady_hint("led1", Light::Lit, off_then, off, 0.002, 0.0001).unwrap(),
            "led1 carries 37.6 uA over 20ms and is off. lit needs the average above 2 mA. The average has held at that value for the last 40ms."
        );
    }

    #[test]
    fn a_moving_or_young_led_has_no_steady_hint() {
        let dim = glow_at(1e-3, Light::Dim, 20, 20);
        let later = glow_at(1.5e-3, Light::Dim, 20, 40);
        assert!(steady_hint("led1", Light::Lit, dim, later, 0.002, 0.0001).is_none());
        let young = glow_at(1e-3, Light::Dim, 5, 5);
        let young_then = glow_at(1e-3, Light::Dim, 0, 0);
        assert!(steady_hint("led1", Light::Lit, young_then, young, 0.002, 0.0001).is_none());
    }
}
