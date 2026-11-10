use crate::diagnostic::Diagnostic;
use crate::diagnostic::ValueRule;
use crate::name::Name;
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
use crate::netlist::json::WireValue;
use crate::netlist::json::check_events;
use crate::netlist::json::resolve_int;
use crate::netlist::json::resolve_pins;
use crate::netlist::json::resolve_value;
use serde::Deserialize;
use std::collections::BTreeMap;

use crate::time::Time;

use crate::mna::{Probe, Stamp};
use crate::newton::Side;

fn conductance(switch: &Switch, contact: Contact) -> f64 {
    1.0 / switch.resistance(contact).si.get()
}

/// A restart takes the contact after the edge at `at`. Every other solve takes the one before it.
pub(crate) fn stamp(switch: &Switch, stamp: &mut Stamp<'_>) {
    let contact = stamp
        .conditions
        .side()
        .contact(&switch.schedule, stamp.conditions.at);
    stamp.conductance(switch.pins[0], switch.pins[1], conductance(switch, contact));
}

pub(crate) fn current(switch: &Switch, probe: &Probe<'_>, at: Time, side: Side) -> f64 {
    let g = conductance(switch, side.contact(&switch.schedule, at));
    (probe.v(switch.pins[0]) - probe.v(switch.pins[1])) * g
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
pub(crate) struct WireSwitch {
    pub(crate) name: String,
    pins: PinMap,
    on_resistance: WireValue,
    off_resistance: WireValue,
    initial: Contact,
    events: Vec<WireEvent>,
    #[serde(deserialize_with = "Option::deserialize")]
    bounce: Option<WireBounce>,
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

pub(crate) fn resolve_switch(
    seed: u64,
    name: &Name,
    part_ref: &Ref,
    wire: WireSwitch,
    nets: &BTreeMap<String, NetId>,
    out: &mut Vec<Diagnostic>,
) -> Option<Switch> {
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
