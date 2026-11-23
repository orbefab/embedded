//! A relay as its coil and an SPDT contact that follows the coil current with hysteresis. The
//! contact moves at the located crossing, with no armature travel and no bounce.

use crate::diagnostic::Diagnostic;
use crate::diagnostic::ValueRule;
use crate::netlist::Contact;
use crate::netlist::NetId;
use crate::netlist::Rating;
use crate::netlist::Value;
use crate::netlist::json::AMPERE;
use crate::netlist::json::PinMap;
use crate::netlist::json::WireRating;
use crate::netlist::json::WireValue;
use crate::netlist::json::resolve_pins;
use crate::netlist::json::resolve_rating;
use crate::netlist::json::resolve_value;
use serde::Deserialize;

use crate::logic::Level;

use orbe_model::Time;

use crate::element::{self, Observed, OwnedCapacitor, OwnedInductor, TWO_PIN, Update};
use crate::mna::{Node, Probe, Stamp, Unknowns};
use crate::name::Quantity;
use crate::netlist::json::ParseCtx;
use crate::netlist::{Conduction, Kind};
use crate::newton::Side;
use crate::watch::{Signal, Watch};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct State {
    pub(crate) contact: Contact,
}

/// The operating point closes it when the coil starts above pull-in.
pub(crate) const POWER_ON: State = State {
    contact: Contact::Open,
};

pub(crate) const SIGNALS: [&str; 1] = ["contact"];

/// `coil_a` through the coil resistance to internal node 0, then the inductance to `coil_b`.
pub(crate) fn coil(relay: &Relay) -> OwnedInductor {
    element::coil(
        relay.pins[Relay::COIL_B],
        relay.inductance.si.get(),
        relay.coil_resistance.si.get(),
    )
}

/// One watch per polarity of the coil current, read across the coil resistance from `coil_a`
/// to internal node `x`, which carries the inductor's current exactly. Each rises above
/// pull-in and falls at drop-out or below.
pub(crate) fn watches(relay: &Relay, x: Node) -> Vec<Watch> {
    let g = 1.0 / relay.coil_resistance.si.get();
    let a = Node::from(relay.pins[Relay::COIL_A]);
    let current = |sign: f64, offset: f64| Signal {
        terms: vec![(a, sign * g), (x, -sign * g)],
        offset,
    };
    let (pull_in, drop_out) = (
        relay.pull_in_current.si.get(),
        relay.drop_out_current.si.get(),
    );
    [1.0, -1.0]
        .map(|sign| Watch {
            rise: current(sign, pull_in),
            fall: current(sign, drop_out),
        })
        .to_vec()
}

pub(crate) fn stamp(relay: &Relay, state: State, stamp: &mut Stamp<'_>) {
    let pin = |k: usize| relay.pins[k];
    element::stamp_coil(stamp, pin(Relay::COIL_A), coil(relay));
    let on = 1.0 / relay.on_resistance.si.get();
    let off = 1.0 / relay.off_resistance.si.get();
    let (no, nc) = match state.contact {
        Contact::Closed => (on, off),
        Contact::Open => (off, on),
    };
    stamp.conductance(pin(Relay::COM), pin(Relay::NO), no);
    stamp.conductance(pin(Relay::COM), pin(Relay::NC), nc);
}

/// Closed while either polarity's watch is high. Level-sensitive.
pub(crate) fn react(levels: &[bool]) -> State {
    let contact = if levels.iter().any(|&high| high) {
        Contact::Closed
    } else {
        Contact::Open
    };
    State { contact }
}

pub(crate) fn levels(state: State) -> [Level; 1] {
    [Level::from_bool(state.contact == Contact::Closed)]
}

/// A coil and an SPDT contact. The coil is `coil_a` through `coil_resistance`, then
/// `inductance` to `coil_b`, as an `Inductor`. `com` meets `no` through `on_resistance` while
/// the contact is closed, and `nc` while it is open. Each other pair takes `off_resistance`. The
/// contact closes when the coil current's magnitude rises above `pull_in_current` and opens when
/// it falls to `drop_out_current` or below.
#[derive(Clone, Debug, PartialEq)]
pub struct Relay {
    /// Nets of pins `coil_a`, `coil_b`, `com`, `no`, `nc`.
    pub pins: [NetId; 5],
    /// Henry, greater than 0.
    pub inductance: Value,
    /// Ohm, greater than 0.
    pub coil_resistance: Value,
    /// Ampere, greater than 0 and above `drop_out_current`.
    pub pull_in_current: Value,
    /// Ampere, greater than 0.
    pub drop_out_current: Value,
    /// Ohm, greater than 0.
    pub on_resistance: Value,
    /// Ohm, greater than 0.
    pub off_resistance: Value,
    /// Ampere on the coil. `None` when the part states no coil-current rating.
    pub max_current: Option<Rating>,
    /// Watt on the coil. `None` when the part states no coil-power rating.
    pub max_power: Option<Rating>,
}

impl Relay {
    pub const PINS: [&'static str; 5] = ["coil_a", "coil_b", "com", "no", "nc"];
    pub const COIL_A: usize = 0;
    pub const COIL_B: usize = 1;
    pub const COM: usize = 2;
    pub const NO: usize = 3;
    pub const NC: usize = 4;
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireRelay {
    pub(crate) name: String,
    pins: PinMap,
    inductance: WireValue,
    coil_resistance: WireValue,
    pull_in_current: WireValue,
    drop_out_current: WireValue,
    on_resistance: WireValue,
    off_resistance: WireValue,
    #[serde(default)]
    ratings: WireRelayRatings,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct WireRelayRatings {
    #[serde(default)]
    current: Option<WireRating>,
    #[serde(default)]
    power: Option<WireRating>,
}

pub(crate) fn resolve_relay(ctx: ParseCtx<'_>, wire: WireRelay) -> Option<Relay> {
    let ParseCtx {
        name,
        part: part_ref,
        nets,
        out,
        ..
    } = ctx;
    let pins = resolve_pins(name, Relay::PINS, &wire.pins.0, nets, out);
    let mut positive =
        |field: &'static str, unit, value| resolve_value(part_ref, field, unit, true, value, out);
    let inductance = positive("inductance", "H", wire.inductance);
    let coil_resistance = positive("coil_resistance", "ohm", wire.coil_resistance);
    let pull_in_current = positive("pull_in_current", AMPERE, wire.pull_in_current);
    let drop_out_current = positive("drop_out_current", AMPERE, wire.drop_out_current);
    let on_resistance = positive("on_resistance", "ohm", wire.on_resistance);
    let off_resistance = positive("off_resistance", "ohm", wire.off_resistance);
    let max_current = resolve_rating(
        part_ref,
        "ratings.current",
        AMPERE,
        wire.ratings.current,
        out,
    );
    let max_power = resolve_rating(part_ref, "ratings.power", "W", wire.ratings.power, out);
    if let (Some(pull), Some(drop)) = (&pull_in_current, &drop_out_current)
        && drop.si.get() >= pull.si.get()
    {
        out.push(Diagnostic::NetlistValue {
            part: part_ref.clone(),
            field: "drop_out_current".into(),
            value: drop.si.get(),
            rule: ValueRule::Hysteresis,
        });
        return None;
    }
    Some(Relay {
        pins: pins?,
        inductance: inductance?,
        coil_resistance: coil_resistance?,
        pull_in_current: pull_in_current?,
        drop_out_current: drop_out_current?,
        on_resistance: on_resistance?,
        off_resistance: off_resistance?,
        max_current: max_current?,
        max_power: max_power?,
    })
}

impl Kind for Relay {
    fn pins(&self) -> &[NetId] {
        &self.pins
    }
    fn unknowns(&self) -> Unknowns {
        Unknowns {
            internal_nodes: 1,
            branches: 1,
            ..Unknowns::default()
        }
    }
    fn charges(&self) -> Vec<OwnedCapacitor> {
        Vec::new()
    }
    fn inductors(&self) -> Vec<OwnedInductor> {
        vec![coil(self)]
    }
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        crate::chip::stamp(crate::chip::Passive::Relay(self), *stamp.chip(), stamp);
    }
    fn reads_iterate(&self) -> bool {
        false
    }
    fn initial(&self, _out: &mut [f64]) {}
    fn update(&self, _probe: &Probe<'_>, _prior: &[f64], _next: &mut [f64]) -> Update {
        Update::default()
    }
    /// Its coil's ends, and `com` to `no` and to `nc` in either state.
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[
            (Relay::COIL_A, Relay::COIL_B),
            (Relay::COM, Relay::NO),
            (Relay::COM, Relay::NC),
        ]
    }
    /// Its coil's.
    fn quantities(&self) -> &'static [Quantity] {
        TWO_PIN
    }
    fn ratings(&self) -> (Option<(Quantity, &Rating)>, Option<&Rating>) {
        (
            self.max_current
                .as_ref()
                .map(|rating| (Quantity::I, rating)),
            self.max_power.as_ref(),
        )
    }
    fn observe(&self, probe: &Probe<'_>, _at: Time, _side: Side) -> Option<Observed> {
        let coil = [self.pins[Relay::COIL_A], self.pins[Relay::COIL_B]];
        Some(Observed::two_pin(coil, probe.branch(0), probe))
    }
    fn conduction(&self) -> Conduction {
        Conduction::Other
    }
}
