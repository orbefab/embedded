use crate::netlist::Rating;
use orbe_model::Time;

use crate::diagnostic::Diagnostic;
use crate::diagnostic::ValueRule;
use crate::name::Ref;
use crate::netlist::Bounce;
use crate::netlist::Contact;
use crate::netlist::Edge;
use crate::netlist::MIN_GAP_PS;
use crate::netlist::NetId;
use crate::netlist::Schedule;
use crate::netlist::Value;
use crate::netlist::bounce;
use crate::netlist::json::PinMap;
use crate::netlist::json::WireInt;
use crate::netlist::json::WireRating;
use crate::netlist::json::WireValue;
use crate::netlist::json::check_events;
use crate::netlist::json::resolve_int;
use crate::netlist::json::resolve_limit;
use crate::netlist::json::resolve_pins;
use crate::netlist::json::resolve_rating;
use crate::netlist::json::resolve_value;
use serde::Deserialize;

use crate::element::{Observed, OwnedCapacitor, OwnedInductor, TWO_PIN, Update};
use crate::mna::{Probe, Stamp, Unknowns};
use crate::name::Quantity;
use crate::netlist::json::ParseCtx;
use crate::netlist::{Conduction, Kind};
use crate::newton::Side;

fn conductance(switch: &Switch, contact: Contact) -> f64 {
    1.0 / switch.resistance(contact).si.get()
}

/// A two-terminal contact: `on_resistance` closed, `off_resistance` open.
#[derive(Clone, Debug, PartialEq)]
pub struct Switch {
    /// Nets of pins `a`, `b`.
    pub pins: [NetId; 2],
    /// Ohm, greater than 0.
    pub on_resistance: Value,
    /// Ohm, greater than 0.
    pub off_resistance: Value,
    /// The netlist's own events, without bounce.
    pub events: Schedule<Contact>,
    pub bounce: Option<Bounce>,
    /// `events` with their bounce expanded.
    pub schedule: Schedule<Contact>,
    /// Ampere, greater than 0: contact current limit, bounding `i`. `None` when unrated.
    pub max_current: Option<Rating>,
    /// Maximum DC voltage across `a` and `b`, in volt. `|V(a) - V(b)|` above this warns
    /// `rating.voltage` on every sample. `None` when unrated.
    pub max_voltage: Option<Value>,
}

impl Switch {
    pub const PINS: [&'static str; 2] = ["a", "b"];

    pub fn resistance(&self, contact: Contact) -> &Value {
        match contact {
            Contact::Open => &self.off_resistance,
            Contact::Closed => &self.on_resistance,
        }
    }

    /// The first time at or after `from` that one more event passes the schedule rules.
    /// The bounce window is this switch's addition to [`Schedule::earliest`].
    pub fn earliest(&self, from: Time) -> Option<Time> {
        let window = self.bounce.as_ref().map_or(0, |bounce| bounce.window.value);
        self.events.earliest(from, window)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSwitchRatings {
    #[serde(deserialize_with = "Option::deserialize")]
    current: Option<WireRating>,
    #[serde(deserialize_with = "Option::deserialize")]
    voltage: Option<WireValue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireSwitch {
    pub(crate) name: String,
    pins: PinMap,
    on_resistance: WireValue,
    off_resistance: WireValue,
    initial: Contact,
    events: Vec<WireEvent>,
    #[serde(deserialize_with = "Option::deserialize")]
    bounce: Option<WireBounce>,
    /// Absent on a switch that rates no current or voltage.
    #[serde(default)]
    ratings: Option<WireSwitchRatings>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireEvent {
    t_ps: u64,
    state: Contact,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireBounce {
    window: WireInt<u64>,
    max_bounces: WireInt<u8>,
}

pub(crate) fn resolve_switch(ctx: ParseCtx<'_>, wire: WireSwitch) -> Option<Switch> {
    let ParseCtx {
        seed,
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let pins = resolve_pins(name, Switch::PINS, &wire.pins.0, nets, out);
    let on = resolve_value(
        part_ref,
        "on_resistance",
        "ohm",
        true,
        wire.on_resistance,
        out,
    );
    let off = resolve_value(
        part_ref,
        "off_resistance",
        "ohm",
        true,
        wire.off_resistance,
        out,
    );
    let bounce = match wire.bounce {
        None => Some(None),
        Some(bounce) => resolve_bounce(part_ref, bounce, out).map(Some),
    };
    let window = match &bounce {
        Some(Some(bounce)) => bounce.window.value,
        _ => 0,
    };
    let events: Vec<Edge> = wire
        .events
        .iter()
        .map(|event| Edge {
            at: Time::from_ps(event.t_ps),
            to: event.state,
        })
        .collect();
    let events_ok = check_events(
        part_ref,
        "events",
        wire.initial,
        &events,
        window,
        |left, right| left == right,
        out,
    );
    let (max_current, max_voltage) = match wire.ratings {
        None => (Some(None), Some(None)),
        Some(ratings) => (
            resolve_rating(part_ref, "ratings.current", "A", ratings.current, out),
            resolve_limit(part_ref, "ratings.voltage", "V", ratings.voltage, out),
        ),
    };
    let bounce = bounce?;
    if !events_ok {
        return None;
    }
    let edges = bounce::expand(seed, name, &events, bounce.as_ref());
    Some(Switch {
        pins: pins?,
        on_resistance: on?,
        off_resistance: off?,
        events: Schedule {
            initial: wire.initial,
            events,
        },
        bounce,
        schedule: Schedule {
            initial: wire.initial,
            events: edges,
        },
        max_current: max_current?,
        max_voltage: max_voltage?,
    })
}

pub(crate) fn resolve_bounce(
    part: &Ref,
    wire: WireBounce,
    out: &mut Vec<Diagnostic>,
) -> Option<Bounce> {
    let window = resolve_int(part, "bounce.window", "ps", wire.window, out);
    let max_bounces = resolve_int(part, "bounce.max_bounces", "1", wire.max_bounces, out);
    let (window, max_bounces) = (window?, max_bounces?);
    if window.value < 2 * max_bounces.value * MIN_GAP_PS {
        out.push(Diagnostic::NetlistValue {
            part: part.clone(),
            field: "bounce.window".into(),
            value: window.value as f64,
            rule: ValueRule::BounceRoom,
        });
        return None;
    }
    Some(Bounce {
        window,
        max_bounces,
    })
}

impl Kind for Switch {
    fn pins(&self) -> &[NetId] {
        &self.pins
    }
    fn unknowns(&self) -> Unknowns {
        Unknowns::default()
    }
    fn charges(&self) -> Vec<OwnedCapacitor> {
        Vec::new()
    }
    fn inductors(&self) -> Vec<OwnedInductor> {
        Vec::new()
    }
    /// A restart takes the contact after the edge at `at`. Every other solve takes the one
    /// before it.
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        let contact = stamp
            .conditions
            .side()
            .contact(&self.schedule, stamp.conditions.at);
        stamp.conductance(self.pins[0], self.pins[1], conductance(self, contact));
    }
    fn reads_iterate(&self) -> bool {
        false
    }
    fn initial(&self, _out: &mut [f64]) {}
    fn update(&self, _probe: &Probe<'_>, _prior: &[f64], _next: &mut [f64]) -> Update {
        Update::default()
    }
    /// A resistor in either state.
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[(0, 1)]
    }
    fn quantities(&self) -> &'static [Quantity] {
        TWO_PIN
    }
    fn ratings(&self) -> (Option<(Quantity, &Rating)>, Option<&Rating>) {
        (
            self.max_current
                .as_ref()
                .map(|rating| (Quantity::I, rating)),
            None,
        )
    }
    fn observe(&self, probe: &Probe<'_>, at: Time, side: Side) -> Option<Observed> {
        let g = conductance(self, side.contact(&self.schedule, at));
        let i = (probe.v(self.pins[0]) - probe.v(self.pins[1])) * g;
        Some(Observed::two_pin(self.pins, i, probe))
    }
    fn conduction(&self) -> Conduction {
        Conduction::Passes
    }
    fn next_change_after(&self, at: Time) -> Option<Time> {
        self.schedule.next_change_after(at)
    }
}

#[cfg(test)]
mod ratings {
    fn circuit(volts: f64, state: &str, ratings: &str) -> String {
        format!(
            r#"{{
              "version": 1,
              "seed": 0,
              "analysis": {{ "op": {{}} }},
              "ground": "gnd",
              "nets": ["gnd", "n"],
              "parts": [
                {{ "name": "v1", "kind": "voltage_source", "pins": {{ "p": "n", "n": "gnd" }},
                  "voltage": {{ "value": {volts}, "unit": "V" }} }},
                {{ "name": "sw1", "kind": "switch",
                  "pins": {{ "a": "n", "b": "gnd" }},
                  "on_resistance": {{ "value": 100.0, "unit": "ohm" }},
                  "off_resistance": {{ "value": 100000000.0, "unit": "ohm" }},
                  "initial": "{state}",
                  "events": [],
                  "bounce": null{ratings} }}
              ]
            }}"#
        )
    }

    fn warnings(json: &str) -> String {
        crate::simulate(&crate::netlist::parse(json.as_bytes()).unwrap())
            .diagnostics()
            .iter()
            .map(|d| serde_json::to_string(d).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn current_and_voltage_ratings_warn_past_the_limit_and_stay_quiet_inside() {
        let rated = r#", "ratings": { "current": { "value": 0.03, "unit": "A" }, "voltage": { "value": 24.0, "unit": "V" } }"#;
        let null_rated = r#", "ratings": { "current": null, "voltage": null }"#;
        // 3 V across closed 100 ohm contact = 30 mA (at limit, quiet).
        assert!(!warnings(&circuit(3.0, "closed", rated)).contains("rating."));
        // 5 V across closed 100 ohm contact = 50 mA > 30 mA (warns rating.current).
        let over_i = warnings(&circuit(5.0, "closed", rated));
        assert!(over_i.contains("rating.current"), "{over_i}");
        assert!(!over_i.contains("rating.voltage"), "{over_i}");
        // 24 V across open contact (at limit, quiet).
        assert!(!warnings(&circuit(24.0, "open", rated)).contains("rating."));
        assert!(!warnings(&circuit(-24.0, "open", rated)).contains("rating."));
        // 30 V or -30 V across open contact warns rating.voltage.
        let over_v = warnings(&circuit(30.0, "open", rated));
        assert!(over_v.contains("rating.voltage"), "{over_v}");
        assert!(
            over_v.contains("#sw1.v reaches 30 V, outside its -24 V to 24 V rating"),
            "{over_v}"
        );
        let over_neg_v = warnings(&circuit(-30.0, "open", rated));
        assert!(over_neg_v.contains("rating.voltage"), "{over_neg_v}");
        // Unrated or explicit null stays quiet.
        assert!(!warnings(&circuit(30.0, "closed", "")).contains("rating."));
        assert!(!warnings(&circuit(30.0, "closed", null_rated)).contains("rating."));
    }
}
