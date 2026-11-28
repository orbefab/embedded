//! A part's operating temperature against the world's temperature.
//!
//! The world's temperature is a schedule of levels, so the check reads the schedule itself
//! rather than each accepted row: the level before the first change holds from t = 0, and each
//! change holds from its time. An operating range applies only to a part that is operating, so
//! the check counts only the time the part is powered, each supply pin above
//! [`SUPPLY_UNPOWERED_V`] over its ground, the floor at or below which `rating.supply_low` calls
//! a rail off. Each row's power holds until the next row, and the operating point's from t = 0.
//! After a restart at an edge the next row's holds back to the edge, as a filter's sample does. A part
//! outside its range keeps running. The finding merges every powered visit outside into one
//! warning per part: the first time out, the farthest level, when the world reached it, and the
//! total time out through the last sample.

use orbe_model::Time;

use super::{Ratings, SUPPLY_UNPOWERED_V, Timing};
use crate::diagnostic::{Diagnostic, TemperatureExcursion, exceeds_rounded, significant};
use crate::finite::Finite;
use crate::name::Ref;
use crate::netlist::{Model, Netlist};
use crate::observe::Columns;
use crate::world::{Quantity, World};

/// One part's operating temperature range, in degC, and what it has found so far.
#[derive(Clone)]
pub(super) struct TemperatureWatch {
    part: Ref,
    min: f64,
    max: f64,
    /// Each supply pin's column and its ground's column.
    supplies: Vec<(usize, usize)>,
    /// The last sample's time and whether the part was powered there, which holds until the
    /// next sample.
    last: Option<(Time, bool)>,
    /// The merged excursion, and how far outside the range its peak sits.
    found: Option<(TemperatureExcursion, f64)>,
}

/// Each part of `netlist` that rates its operating temperature.
pub(super) fn watches(netlist: &Netlist, columns: &Columns) -> Vec<TemperatureWatch> {
    netlist
        .parts()
        .filter_map(|(_, part)| {
            let chip = match &part.model {
                Model::Chip(chip) => chip,
                Model::Resistor(_)
                | Model::Potentiometer(_)
                | Model::VoltageSource(_)
                | Model::CurrentSource(_)
                | Model::Diode(_)
                | Model::Led(_)
                | Model::Zener(_)
                | Model::LinearRegulator(_)
                | Model::Npn(_)
                | Model::Pnp(_)
                | Model::Nmos(_)
                | Model::Pmos(_)
                | Model::Capacitor(_)
                | Model::Inductor(_)
                | Model::Relay(_)
                | Model::Switch(_)
                | Model::OpAmp(_)
                | Model::AnalogTempSensor(_) => return None,
            };
            let (range, supplies) = crate::chip::temperature_rating(chip)?;
            Some(TemperatureWatch {
                part: Ref::Part(part.name.clone()),
                min: range.min.si.get(),
                max: range.max.si.get(),
                supplies: supplies
                    .into_iter()
                    .map(|(pin, gnd)| (columns.net(pin), columns.net(gnd)))
                    .collect(),
                last: None,
                found: None,
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

    /// Every supply pin in `row` sits above the unpowered floor over its ground.
    fn powered(&self, row: &[Finite]) -> bool {
        self.supplies
            .iter()
            .all(|&(pin, gnd)| row[pin].get() - row[gnd].get() > SUPPLY_UNPOWERED_V)
    }

    /// The operating point's `row` at t = 0. A powered part checks the level there, with no
    /// time yet.
    fn seed(&mut self, row: &[Finite], world: &World, timing: Timing) {
        let on = self.powered(row);
        self.last = Some((Time::ZERO, on));
        if on {
            self.count(world, Time::ZERO, Time::ZERO, timing);
        }
    }

    /// The row at `at`. A part powered at the last sample counts the world from there to `at`.
    /// After a restart, `back`, the last sample's row held the state before its edge, so this
    /// row's power holds back to that edge instead.
    fn sample(&mut self, at: Time, row: &[Finite], world: &World, timing: Timing, back: bool) {
        let on = self.powered(row);
        if let Some((from, was)) = self.last {
            if at <= from {
                return;
            }
            let held = if back { on } else { was };
            if held {
                self.count(world, from, at, timing);
            }
        }
        self.last = Some((at, on));
    }

    /// Merges each level the world holds over `[from, until)`, or the level at `from` alone
    /// with no time when the two are equal.
    fn count(&mut self, world: &World, from: Time, until: Time, timing: Timing) {
        let Some(schedule) = world.schedule(Quantity::Temperature) else {
            return;
        };
        let events = schedule.events();
        let k = events.partition_point(|event| event.at <= from);
        let mut level = match k {
            0 => schedule.initial().si.get(),
            _ => events[k - 1].to.si.get(),
        };
        let mut start = from;
        for event in events[k..].iter().take_while(|event| event.at < until) {
            self.merge(start, event.at, level, timing);
            start = event.at;
            level = event.to.si.get();
        }
        self.merge(start, until, level, timing);
    }

    /// Merges `level`, held from `from` to `until`, into the finding when it is outside.
    fn merge(&mut self, from: Time, until: Time, level: f64, timing: Timing) {
        let Some(past) = self.outside(level) else {
            return;
        };
        let (excursion, worst) = self.found.get_or_insert_with(|| {
            let excursion = TemperatureExcursion {
                part: self.part.clone(),
                low: self.min,
                high: self.max,
                peak: level,
                peak_at: from,
                first: from,
                over_ps: timing.over_ps(),
            };
            (excursion, past)
        });
        if past > *worst {
            *worst = past;
            excursion.peak = level;
            excursion.peak_at = from;
        }
        if let Some(over_ps) = &mut excursion.over_ps {
            *over_ps += until.as_ps() - from.as_ps();
        }
    }
}

impl Ratings {
    /// Each temperature watch's power at the operating point, `row` at t = 0. A powered part
    /// whose range the world's level at t = 0 is outside warns with no time yet.
    pub(crate) fn seed_temperatures(&mut self, row: &[Finite], world: &World) {
        for watch in &mut self.temperatures {
            watch.seed(row, world, self.timing);
        }
    }

    /// Counts each powered part's time in the world from its last sample to `at`, and notes
    /// whether `row` powers it from `at` on. It reads whether the last sample restarted, so it
    /// runs before [`Self::sample`] of the same row.
    pub(crate) fn sample_temperatures(&mut self, at: Time, row: &[Finite], world: &World) {
        for watch in &mut self.temperatures {
            watch.sample(at, row, world, self.timing, self.restarted);
        }
    }

    /// One `rating.temperature` per part whose range the world's temperature left while the
    /// part was powered, through the last sample, or through `probe`, a time and its row, when
    /// it is after that sample. A probe is not kept.
    pub(crate) fn temperatures(
        &self,
        world: &World,
        probe: Option<(Time, &[Finite])>,
    ) -> Vec<Diagnostic> {
        self.temperatures
            .iter()
            .filter_map(|watch| match probe {
                Some((at, row)) if watch.last.is_none_or(|(last, _)| at > last) => {
                    let mut watch = watch.clone();
                    watch.sample(at, row, world, self.timing, self.restarted);
                    watch.found
                }
                _ => watch.found.clone(),
            })
            .map(|(excursion, _)| Diagnostic::RatingTemperature(excursion))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    const MS: u64 = 1_000_000_000;

    const FIVE_VOLTS: &str = r#"{ "value": 5.0, "unit": "V" }"#;

    /// A DHT22 rated `min` to `max` degC on a 5 V supply, in a world whose temperature is
    /// `temperature`, under `analysis`.
    fn netlist(analysis: &str, temperature: &str, range: (f64, f64)) -> String {
        supplied(FIVE_VOLTS, analysis, temperature, range)
    }

    /// [`netlist`] with `supply` as the supply's `voltage`.
    fn supplied(supply: &str, analysis: &str, temperature: &str, (min, max): (f64, f64)) -> String {
        format!(
            r#"{{
              "version": 1, "seed": 0,
              "analysis": {analysis},
              "ground": "gnd", "nets": ["gnd", "vcc", "data"],
              "world": {{ "temperature": {temperature}, "humidity": {{ "value": 50.0, "unit": "%RH" }} }},
              "parts": [
                {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "vcc", "n": "gnd" }},
                  "voltage": {supply} }},
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

    /// The supply is off until 1 ms, on until 3 ms and off again, so only those 2 ms count.
    #[test]
    fn only_the_time_the_part_is_powered_counts() {
        let pulse = format!(
            r#"{{ "kind": "pulse", "initial": {{ "value": 0.0, "unit": "V" }},
                 "pulsed": {{ "value": 5.0, "unit": "V" }},
                 "delay_ps": {MS}, "width_ps": {}, "period_ps": {} }}"#,
            2 * MS,
            100 * MS
        );
        let found = temperatures(&supplied(
            &pulse,
            &transient(6),
            &level(-25.0),
            (-20.0, 50.0),
        ));
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0]["t_ps"], MS);
        assert_eq!(found[0]["data"]["peak_ps"], MS);
        assert_eq!(found[0]["data"]["over_ps"], 2 * MS);
    }

    /// A supply at or below the 0.1 V floor is off, so a part on it never operates.
    #[test]
    fn a_part_that_is_never_powered_does_not_warn() {
        for volts in [0.0, 0.05, 0.1] {
            let supply = format!(r#"{{ "value": {volts:?}, "unit": "V" }}"#);
            for analysis in [transient(3), r#"{ "op": {} }"#.to_string()] {
                let found =
                    temperatures(&supplied(&supply, &analysis, &level(70.0), (-20.0, 50.0)));
                assert!(found.is_empty(), "{volts} V, {analysis}: {found:?}");
            }
        }
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
