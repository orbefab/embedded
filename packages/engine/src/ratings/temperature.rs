//! A part's operating temperature against the world's temperature.
//!
//! The world's temperature is a schedule of levels, so the check reads the schedule itself
//! rather than each accepted row: the level before the first change holds from t = 0, and each
//! change holds from its time. A part outside its range keeps running. The finding merges every
//! visit outside into one warning per part: the first time out, the farthest level, when the
//! world reached it, and the total time out through the last accepted step.

use orbe_model::Time;

use super::{Ratings, Timing};
use crate::diagnostic::{Diagnostic, TemperatureExcursion, exceeds_rounded, significant};
use crate::name::Ref;
use crate::netlist::{Model, Netlist};
use crate::world::{Quantity, World};

/// One part's operating temperature range, in degC.
#[derive(Clone)]
pub(super) struct TemperatureWatch {
    part: Ref,
    min: f64,
    max: f64,
}

/// Each part of `netlist` that rates its operating temperature.
pub(super) fn watches(netlist: &Netlist) -> Vec<TemperatureWatch> {
    netlist
        .parts()
        .filter_map(|(_, part)| {
            let Model::Chip(chip) = &part.model else {
                return None;
            };
            let range = crate::chip::temperature_range(chip)?;
            Some(TemperatureWatch {
                part: Ref::Part(part.name.clone()),
                min: range.min.si.get(),
                max: range.max.si.get(),
            })
        })
        .collect()
}

impl TemperatureWatch {
    /// How far `level` sits outside the range, once each side is rounded to 3 significant
    /// digits as the message prints it. `None` inside, ends included.
    fn outside(&self, level: f64) -> Option<f64> {
        if exceeds_rounded(self.min, significant(level)) {
            Some(self.min - level)
        } else if exceeds_rounded(level, significant(self.max)) {
            Some(level - self.max)
        } else {
            None
        }
    }

    /// The finding through `through`, or at t = 0 alone for an operating point.
    fn finding(
        &self,
        world: &World,
        through: Time,
        timing: Timing,
    ) -> Option<TemperatureExcursion> {
        let schedule = world.schedule(Quantity::Temperature)?;
        // Each level and the time it holds from, in order.
        let levels = std::iter::once((Time::ZERO, schedule.initial().si.get())).chain(
            schedule
                .events()
                .iter()
                .map(|event| (event.at, event.to.si.get())),
        );
        let ends = schedule
            .events()
            .iter()
            .map(|event| Some(event.at))
            .chain(std::iter::once(None));
        let mut found: Option<(TemperatureExcursion, f64)> = None;
        for ((from, level), until) in levels.zip(ends) {
            if from > Time::ZERO && from >= through {
                break;
            }
            let Some(past) = self.outside(level) else {
                continue;
            };
            let until = until.map_or(through, |until| until.min(through));
            let held = until.as_ps().saturating_sub(from.as_ps());
            match &mut found {
                None => {
                    let excursion = TemperatureExcursion {
                        part: self.part.clone(),
                        low: self.min,
                        high: self.max,
                        peak: level,
                        peak_at: from,
                        first: from,
                        over_ps: timing.over_ps(),
                    };
                    found = Some((excursion, past));
                }
                Some((excursion, worst)) => {
                    if past > *worst {
                        *worst = past;
                        excursion.peak = level;
                        excursion.peak_at = from;
                    }
                }
            }
            if let Some((excursion, _)) = &mut found
                && let Some(over_ps) = &mut excursion.over_ps
            {
                *over_ps += held;
            }
        }
        found.map(|(excursion, _)| excursion)
    }
}

impl Ratings {
    /// One `rating.temperature` per part whose range the world's temperature left by
    /// `through`, the last accepted step. An operating point reads the level at t = 0 alone.
    pub(crate) fn temperatures(&self, world: &World, through: Time) -> Vec<Diagnostic> {
        let through = match self.timing {
            Timing::Point => Time::ZERO,
            Timing::Run => through,
        };
        self.temperatures
            .iter()
            .filter_map(|watch| watch.finding(world, through, self.timing))
            .map(Diagnostic::RatingTemperature)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    const MS: u64 = 1_000_000_000;

    /// A DHT22 rated `min` to `max` degC on a 5 V supply, in a world whose temperature is
    /// `temperature`, under `analysis`.
    fn netlist(analysis: &str, temperature: &str, (min, max): (f64, f64)) -> String {
        format!(
            r#"{{
              "version": 1, "seed": 0,
              "analysis": {analysis},
              "ground": "gnd", "nets": ["gnd", "vcc", "data"],
              "world": {{ "temperature": {temperature}, "humidity": {{ "value": 50.0, "unit": "%RH" }} }},
              "parts": [
                {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "vcc", "n": "gnd" }},
                  "voltage": {{ "value": 5.0, "unit": "V" }} }},
                {{ "name": "rp", "kind": "resistor", "pins": {{ "a": "vcc", "b": "data" }},
                  "resistance": {{ "value": 10000.0, "unit": "ohm" }}, "ratings": {{ "power": null }} }},
                {{ "name": "u1", "kind": "dht22", "pins": {{ "VCC": "vcc", "DATA": "data", "GND": "gnd" }},
                  "low_resistance": {{ "value": 100.0, "unit": "ohm" }},
                  "input_threshold": {{ "low": {{ "value": 0.3, "unit": "1" }}, "high": {{ "value": 0.7, "unit": "1" }} }},
                  "ratings": {{ "temperature": {{
                    "min": {{ "value": {min:?}, "unit": "degC" }},
                    "max": {{ "value": {max:?}, "unit": "degC" }} }} }} }}
              ]
            }}"#
        )
    }

    fn transient(stop_ms: u64) -> String {
        format!(
            r#"{{ "transient": {{ "stop_ps": {}, "max_step_ps": {MS}, "grid_ps": {MS} }} }}"#,
            stop_ms * MS
        )
    }

    fn level(value: f64) -> String {
        format!(r#"{{ "value": {value:?}, "unit": "degC" }}"#)
    }

    /// `initial`, then each `(ms, degC)`.
    fn steps(initial: f64, events: &[(u64, f64)]) -> String {
        let events: Vec<String> = events
            .iter()
            .map(|&(ms, to)| format!(r#"{{ "t_ps": {}, "level": {} }}"#, ms * MS, level(to)))
            .collect();
        format!(
            r#"{{ "kind": "steps", "initial": {}, "events": [{}] }}"#,
            level(initial),
            events.join(", ")
        )
    }

    fn temperatures(json: &str) -> Vec<Value> {
        let netlist = crate::netlist::parse(json.as_bytes()).unwrap();
        crate::simulate(&netlist)
            .diagnostics()
            .iter()
            .map(|d| serde_json::to_value(d).unwrap())
            .filter(|d| d["code"] == "rating.temperature")
            .collect()
    }

    #[test]
    fn a_world_below_the_minimum_warns_and_the_ends_are_inside() {
        let cold = temperatures(&netlist(&transient(3), &level(-25.0), (-20.0, 50.0)));
        assert_eq!(cold.len(), 1, "{cold:?}");
        assert_eq!(
            cold[0]["message"],
            "#u1 sits at -25 degC, outside its -20 degC to 50 degC operating temperature for 3000000000 ps"
        );
        assert_eq!(cold[0]["severity"], "warning");
        assert_eq!(cold[0]["t_ps"], 0);
        for end in [-20.0, 50.0] {
            let found = temperatures(&netlist(&transient(3), &level(end), (-20.0, 50.0)));
            assert!(found.is_empty(), "{end} degC: {found:?}");
        }
    }

    /// Out cold at 1 ms, back at 2 ms, out hot and farther at 3 ms, back at 4 ms: one warning
    /// from 1 ms, naming the hot level, for the 2 ms spent out.
    #[test]
    fn visits_outside_merge_into_one_warning_with_the_farthest_level() {
        let world = steps(25.0, &[(1, -30.0), (2, 25.0), (3, 70.0), (4, 25.0)]);
        let found = temperatures(&netlist(&transient(6), &world, (-20.0, 50.0)));
        assert_eq!(found.len(), 1, "{found:?}");
        let d = &found[0];
        assert_eq!(d["t_ps"], MS);
        assert_eq!(d["refs"], serde_json::json!(["#u1"]));
        assert_eq!(
            d["data"],
            serde_json::json!({ "high": 50.0, "low": -20.0, "over_ps": 2 * MS, "peak": 70.0, "peak_ps": 3 * MS })
        );
    }

    /// A step out at the run's stop is after the last accepted step, so nothing reads it.
    #[test]
    fn a_step_out_at_the_stop_is_not_reached() {
        let world = steps(25.0, &[(3, 70.0)]);
        assert!(temperatures(&netlist(&transient(3), &world, (-20.0, 50.0))).is_empty());
        let found = temperatures(&netlist(&transient(4), &world, (-20.0, 50.0)));
        assert_eq!(found[0]["data"]["over_ps"], MS);
    }

    #[test]
    fn an_operating_point_reads_the_level_at_t_0_with_no_duration() {
        let world = steps(70.0, &[(1, 25.0)]);
        let found = temperatures(&netlist(r#"{ "op": {} }"#, &world, (-20.0, 50.0)));
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(
            found[0]["message"],
            "#u1 sits at 70 degC, outside its -20 degC to 50 degC operating temperature"
        );
        assert_eq!(found[0]["data"]["over_ps"], Value::Null);
    }

    #[test]
    fn a_range_whose_minimum_is_above_its_maximum_or_not_in_degc_is_refused() {
        let codes = |json: String| -> Vec<(String, String)> {
            match crate::netlist::parse(json.as_bytes()) {
                Ok(_) => Vec::new(),
                Err(found) => found
                    .iter()
                    .map(|d| serde_json::to_value(d).unwrap())
                    .map(|d| {
                        (
                            d["code"].as_str().unwrap().to_string(),
                            d["data"]["field"].as_str().unwrap_or_default().to_string(),
                        )
                    })
                    .collect(),
            }
        };
        assert_eq!(
            codes(netlist(&transient(1), &level(25.0), (60.0, 50.0))),
            [(
                "netlist.value".to_string(),
                "ratings.temperature.min".to_string()
            )]
        );
        let kelvin = netlist(&transient(1), &level(25.0), (-20.0, 50.0)).replacen(
            r#"-20.0, "unit": "degC""#,
            r#"-20.0, "unit": "K""#,
            1,
        );
        assert_eq!(
            codes(kelvin),
            [(
                "netlist.unit".to_string(),
                "ratings.temperature.min".to_string()
            )]
        );
    }
}
