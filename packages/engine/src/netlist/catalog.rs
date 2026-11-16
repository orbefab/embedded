//! The kind table. One row per kind generates its `Model` or `Chip` variant, its wire variant,
//! its resolve call and its entry in [`engine_kinds`]. An analog kind's answers are its [`Kind`]
//! impl, in its own file. `docs/engine/add-a-kind.md` says how to add one.

use std::sync::LazyLock;

use serde::Deserialize;

use orbe_model::Time;

use super::json::ParseCtx;
use super::{Chip, LevelField, NetId, RatingKind, Schedule, Serial, SerialField, Value};
use crate::element::{Device, Observed, OwnedCapacitor, OwnedInductor, Update};
use crate::mna::{Probe, Stamp, Unknowns};
use crate::name::Quantity;
use crate::newton::Side;

/// What an analog kind answers. One impl per kind, in the kind's own file.
///
/// Every method without a default is required, so a kind that leaves one out does not compile.
/// The defaults are the schedule answers, which only a source, a switch and a potentiometer have.
pub(crate) trait Kind {
    /// Its nets, in the order of its `PINS`.
    fn pins(&self) -> &[NetId];
    /// The internal nodes, branches and junction slots it adds to the system.
    fn unknowns(&self) -> Unknowns;
    /// The capacitors it owns, in index order. Each takes one of its first branches.
    fn charges(&self) -> Vec<OwnedCapacitor>;
    /// The inductors it owns, in index order. Each takes a branch after its capacitors.
    fn inductors(&self) -> Vec<OwnedInductor>;
    /// Adds its conductances and currents at the conditions and iterate `stamp` holds. It runs
    /// on every Newton iteration, so mark it `#[inline]`.
    fn stamp(&self, stamp: &mut Stamp<'_>);
    /// Whether its stamp reads the Newton iterate. A junction reads it through
    /// `Stamp::junction`. A linear regulator reads its last branch current and input voltage
    /// through `Stamp::prior`. Every other stamp reads only the call's conditions, so on a
    /// circuit where no part reads the iterate every iteration of one call stamps the same
    /// system.
    fn reads_iterate(&self) -> bool;
    /// Start voltages for its junction slots.
    fn initial(&self, out: &mut [f64]);
    /// After a solve, limits its junctions from `prior` into `next` and checks their currents.
    fn update(&self, probe: &Probe<'_>, prior: &[f64], next: &mut [f64]) -> Update;
    /// The pin pairs it joins at DC. A chain of them is a DC path for `net.floating`.
    fn dc_paths(&self) -> &'static [(usize, usize)];
    /// The quantities a run observes on it, in `Quantity` order. [`TWO_PIN`] for a two-pin
    /// part.
    ///
    /// [`TWO_PIN`]: crate::element::TWO_PIN
    fn quantities(&self) -> &'static [Quantity];
    /// Its current rating with the quantity that rating bounds, and its power rating, which
    /// bounds `p`.
    fn ratings(&self) -> (Option<(Quantity, &Value)>, Option<&Value>);
    /// Its `quantities` at `at`, in that order. `None` when it has none.
    fn observe(&self, probe: &Probe<'_>, at: Time, side: Side) -> Option<Observed>;
    /// How it carries a steady current between its pins.
    fn conduction(&self) -> Conduction;

    /// Its next source jump, contact edge or position move strictly after `at`.
    fn next_change_after(&self, _at: Time) -> Option<Time> {
        None
    }
    /// The settable level schedule, with the document field that holds it and that field's unit.
    fn level(&self) -> Option<LevelField<'_>> {
        None
    }
    /// [`level`](Self::level)'s schedule, for appending one event.
    fn level_mut(&mut self) -> Option<&mut Schedule<Value>> {
        None
    }
    /// The serial waveform, with the document field that holds it.
    fn serial(&self) -> Option<SerialField<'_>> {
        None
    }
    /// [`serial`](Self::serial)'s waveform, for appending one send.
    fn serial_mut(&mut self) -> Option<&mut Serial> {
        None
    }
}

/// How a part carries a steady current between its pins. `orbe check` reads it to find an LED
/// with nothing in series to limit its current.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conduction {
    /// It holds each net it is on at a set voltage, as ground does. A voltage source or a
    /// linear regulator.
    Fixes,
    /// It carries the current on without limiting it. A junction or a switch.
    Passes,
    /// It joins its first pin to its second while it is on. A transistor's collector and
    /// emitter, or drain and source, by pin index.
    Switches(usize, usize),
    /// Its resistance limits the current. A resistor or a potentiometer.
    Limits,
    /// None of these. A current source, a capacitor, an inductor, a relay or a chip.
    Other,
}

/// `unlisted` keeps a chip row out of [`engine_kinds`].
macro_rules! listed {
    () => {
        true
    };
    (unlisted) => {
        false
    };
}

macro_rules! define_kinds {
    (
        analog {
            $(
                $(#[$meta:meta])*
                $variant:ident($ty:ty), $name:literal, $wire:ty, $resolve:path, $pins:expr;
            )*
        }
        chip {
            $(
                $chip:ident $($unlisted:ident)?, $chip_name:literal, $chip_wire:ty,
                $chip_resolve:path, $chip_pins:expr;
            )*
        }
    ) => {
        /// The closed set of part kinds.
        /// Pin order is that kind's `PINS` const.
        #[derive(Clone, Debug, PartialEq)]
        pub enum Model {
            $(
                $(#[$meta])*
                $variant($ty),
            )*
            /// A package modelled as a behavioural black box. A new chip kind is a `Chip` variant,
            /// not a `Model` variant.
            Chip(Chip),
        }

        const KINDS: &[(&str, &[&str], bool)] = &[
            $(($name, $pins, true),)*
            $(($chip_name, $chip_pins, listed!($($unlisted)?)),)*
        ];

        /// The wire format of one part. The variant order is the order serde lists the kinds in
        /// when it meets an unknown one.
        #[derive(Deserialize)]
        #[serde(tag = "kind", deny_unknown_fields)]
        pub(crate) enum WirePart {
            $(
                #[serde(rename = $name)]
                $variant(Box<$wire>),
            )*
            $(
                #[serde(rename = $chip_name)]
                $chip(Box<$chip_wire>),
            )*
        }

        impl WirePart {
            pub(crate) fn name(&self) -> &String {
                match self {
                    $(WirePart::$variant(wire) => &wire.name,)*
                    $(WirePart::$chip(wire) => &wire.name,)*
                }
            }

            pub(crate) fn resolve(self, ctx: ParseCtx<'_>) -> Option<Model> {
                match self {
                    $(WirePart::$variant(wire) => {
                        let resolve: fn(ParseCtx<'_>, $wire) -> Option<$ty> = $resolve;
                        resolve(ctx, *wire).map(Model::$variant)
                    })*
                    $(WirePart::$chip(wire) => {
                        $chip_resolve(ctx, *wire).map(|part| Model::Chip(Chip::$chip(Box::new(part))))
                    })*
                }
            }
        }

        impl Model {
            /// Pin names in the order of `pins()`. An `esp32s3` lists only the pins it has nets on.
            pub fn pin_names(&self) -> &[&'static str] {
                match self {
                    $(Model::$variant(_) => $pins,)*
                    Model::Chip(chip) => chip_pin_names(chip),
                }
            }

            /// The quantities a run observes on this part, in `Quantity` order. A two-pin part has
            /// `i`, `v` and `p`, and so does a relay, of its coil. A BJT has `ib`, `ic` and `p`. An
            /// `nmos` has `id` and `p`. A linear regulator has `iin`, `iout` and `p`. A chip, an op-amp
            /// and a potentiometer have none.
            pub fn quantities(&self) -> &'static [Quantity] {
                match self {
                    $(Model::$variant(part) => <$ty as Kind>::quantities(part),)*
                    Model::Chip(_) => &[],
                }
            }

            pub fn pins(&self) -> &[NetId] {
                match self {
                    $(Model::$variant(part) => <$ty as Kind>::pins(part),)*
                    Model::Chip(chip) => chip_pins(chip),
                }
            }

            /// Rated quantities in `RatingKind` order, each with the quantity it bounds. A
            /// current rating bounds `iout` on a linear regulator, `ic` on a BJT, `id` on an
            /// `nmos`, and `i` on every other part. A `null` rating is left out.
            pub fn ratings(&self) -> Vec<(RatingKind, Quantity, &Value)> {
                let (current, power) = match self {
                    $(Model::$variant(part) => <$ty as Kind>::ratings(part),)*
                    Model::Chip(_) => (None, None),
                };
                let current = current.map(|(quantity, v)| (RatingKind::Current, quantity, v));
                let power = power.map(|v| (RatingKind::Power, Quantity::P, v));
                current.into_iter().chain(power).collect()
            }

            /// How this part carries a steady current between its pins.
            pub fn conduction(&self) -> Conduction {
                match self {
                    $(Model::$variant(part) => <$ty as Kind>::conduction(part),)*
                    Model::Chip(_) => Conduction::Other,
                }
            }

            /// The next contact edge, source jump, potentiometer move, or device event strictly
            /// after `at`.
            pub fn next_change_after(&self, at: Time) -> Option<Time> {
                if let Some((_, events)) = self.device() {
                    return events.next_after(at);
                }
                match self {
                    $(Model::$variant(part) => <$ty as Kind>::next_change_after(part, at),)*
                    Model::Chip(_) => None,
                }
            }

            /// The settable level schedule, with the document field that holds it and that field's
            /// unit. A `steps` voltage, a `steps` current, or a potentiometer position. A `step`,
            /// `pulse`, or `serial` source has none.
            pub fn level(&self) -> Option<LevelField<'_>> {
                match self {
                    $(Model::$variant(part) => <$ty as Kind>::level(part),)*
                    Model::Chip(_) => None,
                }
            }

            /// [`level`](Self::level)'s schedule, for appending one event.
            pub fn level_mut(&mut self) -> Option<&mut Schedule<Value>> {
                match self {
                    $(Model::$variant(part) => <$ty as Kind>::level_mut(part),)*
                    Model::Chip(_) => None,
                }
            }

            /// The serial waveform, with the document field that holds it. A `serial` voltage or
            /// current.
            pub fn serial(&self) -> Option<SerialField<'_>> {
                match self {
                    $(Model::$variant(part) => <$ty as Kind>::serial(part),)*
                    Model::Chip(_) => None,
                }
            }

            pub(super) fn serial_mut(&mut self) -> Option<&mut Serial> {
                match self {
                    $(Model::$variant(part) => <$ty as Kind>::serial_mut(part),)*
                    Model::Chip(_) => None,
                }
            }

            /// [`Kind::reads_iterate`]. A chip reads it when its passive stamp does.
            #[inline]
            pub(crate) fn reads_iterate(&self) -> bool {
                match self {
                    $(Model::$variant(part) => <$ty as Kind>::reads_iterate(part),)*
                    Model::Chip(chip) => crate::chip::reads_iterate(chip),
                }
            }

            pub(crate) fn initial(&self, out: &mut [f64]) {
                match self {
                    $(Model::$variant(part) => <$ty as Kind>::initial(part, out),)*
                    Model::Chip(_) => {}
                }
            }

            pub(crate) fn update(
                &self,
                probe: &Probe<'_>,
                prior: &[f64],
                next: &mut [f64],
            ) -> Update {
                match self {
                    $(Model::$variant(part) => <$ty as Kind>::update(part, probe, prior, next),)*
                    Model::Chip(_) => Update::default(),
                }
            }

            pub(crate) fn dc_paths(&self) -> &'static [(usize, usize)] {
                match self {
                    $(Model::$variant(part) => <$ty as Kind>::dc_paths(part),)*
                    Model::Chip(chip) => crate::chip::dc_paths(chip),
                }
            }

            pub(crate) fn observe(
                &self,
                probe: &Probe<'_>,
                at: Time,
                side: Side,
            ) -> Option<Observed> {
                match self {
                    $(Model::$variant(part) => <$ty as Kind>::observe(part, probe, at, side),)*
                    Model::Chip(_) => None,
                }
            }
        }

        impl Device for Model {
            #[inline]
            fn unknowns(&self) -> Unknowns {
                match self {
                    $(Model::$variant(part) => <$ty as Kind>::unknowns(part),)*
                    Model::Chip(chip) => crate::chip::unknowns(chip),
                }
            }

            #[inline]
            fn charges(&self) -> Vec<OwnedCapacitor> {
                match self {
                    $(Model::$variant(part) => <$ty as Kind>::charges(part),)*
                    Model::Chip(_) => Vec::new(),
                }
            }

            #[inline]
            fn inductors(&self) -> Vec<OwnedInductor> {
                match self {
                    $(Model::$variant(part) => <$ty as Kind>::inductors(part),)*
                    Model::Chip(_) => Vec::new(),
                }
            }

            #[inline]
            fn stamp(&self, stamp: &mut Stamp<'_>) {
                match self {
                    $(Model::$variant(part) => <$ty as Kind>::stamp(part, stamp),)*
                    Model::Chip(_) => {
                        if let Some(passive) = crate::chip::passive(self) {
                            crate::chip::stamp(passive, *stamp.chip(), stamp);
                        }
                    }
                }
            }
        }
    };
}

/// Kind names whose pins are all required, in wire order, with their pin names. An `esp32s3`
/// lets pins go unwired and is not here.
pub fn engine_kinds() -> &'static [(&'static str, &'static [&'static str])] {
    static LISTED: LazyLock<Vec<(&str, &[&str])>> = LazyLock::new(|| {
        KINDS
            .iter()
            .filter(|(_, _, listed)| *listed)
            .map(|&(name, pins, _)| (name, pins))
            .collect()
    });
    &LISTED
}

fn chip_pin_names(chip: &Chip) -> &[&'static str] {
    use crate::chip::Ne555;
    use crate::device::{Cst816, PulsesReplay, SpiReplay, St77916, Ws2812};
    use crate::mcu::Atmega328p;

    match chip {
        Chip::Ne555(_) => &Ne555::PINS,
        Chip::Atmega328p(_) => &Atmega328p::PINS,
        Chip::SpiReplay(_) => &SpiReplay::PINS,
        Chip::St77916(_) => &St77916::PINS,
        Chip::Cst816(_) => &Cst816::PINS,
        Chip::Esp32s3(mcu) => &mcu.names,
        Chip::Ws2812(_) => &Ws2812::PINS,
        Chip::PulsesReplay(_) => &PulsesReplay::PINS,
    }
}

fn chip_pins(chip: &Chip) -> &[NetId] {
    match chip {
        Chip::Ne555(part) => &part.pins,
        Chip::Atmega328p(part) => &part.pins,
        Chip::SpiReplay(part) => &part.pins,
        Chip::St77916(part) => &part.pins,
        Chip::Cst816(part) => &part.pins,
        Chip::Ws2812(part) => &part.pins,
        Chip::PulsesReplay(part) => &part.pins,
        Chip::Esp32s3(part) => &part.pins,
    }
}

define_kinds! {
    analog {
        Resistor(crate::passive::Resistor), "resistor",
            crate::passive::resistor::WireResistor, crate::passive::resistor::resolve_resistor,
            &crate::passive::Resistor::PINS;
        Potentiometer(crate::passive::Potentiometer), "potentiometer",
            crate::passive::potentiometer::WirePotentiometer,
            crate::passive::potentiometer::resolve_potentiometer,
            &crate::passive::Potentiometer::PINS;
        VoltageSource(crate::passive::VoltageSource), "voltage_source",
            crate::passive::voltage_source::WireVoltageSource,
            crate::passive::voltage_source::resolve_voltage_source,
            &crate::passive::VoltageSource::PINS;
        CurrentSource(crate::passive::CurrentSource), "current_source",
            crate::passive::current_source::WireCurrentSource,
            crate::passive::current_source::resolve_current_source,
            &crate::passive::CurrentSource::PINS;
        Diode(crate::semiconductor::Junction), "diode",
            crate::semiconductor::junction::WireJunction,
            crate::semiconductor::junction::resolve_junction,
            &crate::semiconductor::Junction::PINS;
        #[doc = "The same device equation as `Diode`. Its own kind, so an LED-only quantity needs no flag."]
        Led(crate::semiconductor::Junction), "led",
            crate::semiconductor::junction::WireJunction,
            crate::semiconductor::junction::resolve_junction,
            &crate::semiconductor::Junction::PINS;
        #[doc = "A forward junction and a breakdown junction. See [`Zener`](crate::semiconductor::Zener)."]
        Zener(crate::semiconductor::Zener), "zener",
            crate::semiconductor::zener::WireZener, crate::semiconductor::zener::resolve_zener,
            &crate::semiconductor::Junction::PINS;
        #[doc = "A static linear regulator. See [`LinearRegulator`](crate::chip::LinearRegulator)."]
        LinearRegulator(crate::chip::LinearRegulator), "linear_regulator",
            crate::chip::regulator::WireLinearRegulator, crate::chip::regulator::resolve_regulator,
            &crate::chip::LinearRegulator::PINS;
        Npn(crate::semiconductor::Npn), "npn",
            crate::semiconductor::transistor::WireBjt, crate::semiconductor::transistor::resolve_npn,
            &crate::semiconductor::Bjt::PINS;
        #[doc = "The same device equations as `Npn`, with every junction voltage and pin current negated."]
        Pnp(crate::semiconductor::Pnp), "pnp",
            crate::semiconductor::transistor::WireBjt, crate::semiconductor::transistor::resolve_pnp,
            &crate::semiconductor::Bjt::PINS;
        #[doc = "An N-channel MOSFET. Level 1, bulk tied to the source."]
        Nmos(crate::semiconductor::Mosfet), "nmos",
            crate::semiconductor::transistor::WireMosfet,
            crate::semiconductor::transistor::resolve_mosfet,
            &crate::semiconductor::Mosfet::PINS;
        Capacitor(crate::passive::Capacitor), "capacitor",
            crate::passive::capacitor::WireCapacitor, crate::passive::capacitor::resolve_capacitor,
            &crate::passive::Capacitor::PINS;
        Inductor(crate::passive::Inductor), "inductor",
            crate::passive::inductor::WireInductor, crate::passive::inductor::resolve_inductor,
            &crate::passive::Inductor::PINS;
        #[doc = "A coil and a contact that follows the coil current. Passive, with one state."]
        Relay(crate::chip::Relay), "relay",
            crate::chip::relay::WireRelay, crate::chip::relay::resolve_relay,
            &crate::chip::Relay::PINS;
        Switch(crate::passive::Switch), "switch",
            crate::passive::switch::WireSwitch, crate::passive::switch::resolve_switch,
            &crate::passive::Switch::PINS;
        #[doc = "One op-amp channel with one pole. See [`OpAmp`](crate::chip::OpAmp)."]
        OpAmp(crate::chip::OpAmp), "op_amp",
            crate::chip::op_amp::WireOpAmp, crate::chip::op_amp::resolve_op_amp,
            &crate::chip::OpAmp::PINS;
    }
    chip {
        Ne555, "ne555",
            crate::chip::ne555::WireNe555, crate::chip::ne555::resolve_ne555,
            &crate::chip::Ne555::PINS;
        Atmega328p, "atmega328p",
            crate::mcu::avr::WireAtmega328p, crate::mcu::avr::resolve_atmega328p,
            &crate::mcu::Atmega328p::PINS;
        SpiReplay, "spi_replay",
            crate::device::spi_replay::WireSpiReplay, crate::device::spi_replay::resolve_spi_replay,
            &crate::device::SpiReplay::PINS;
        St77916, "st77916",
            crate::device::st77916::WireSt77916, crate::device::st77916::resolve_st77916,
            &crate::device::St77916::PINS;
        Cst816, "cst816",
            crate::device::cst816::WireCst816, crate::device::cst816::resolve_cst816,
            &crate::device::Cst816::PINS;
        Esp32s3 unlisted, "esp32s3",
            crate::mcu::esp32s3::WireEsp32s3, crate::mcu::esp32s3::resolve_esp32s3,
            &crate::mcu::Esp32s3::PINS;
        Ws2812, "ws2812",
            crate::device::ws2812::WireWs2812, crate::device::ws2812::resolve_ws2812,
            &crate::device::Ws2812::PINS;
        PulsesReplay, "pulses_replay",
            crate::device::pulses_replay::WirePulsesReplay,
            crate::device::pulses_replay::resolve_pulses_replay,
            &crate::device::PulsesReplay::PINS;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{KINDS, WirePart, engine_kinds};

    /// The kinds serde lists when it meets an unknown one, which is every wire tag in order.
    fn wire_kinds() -> Vec<String> {
        let Err(err) = serde_json::from_str::<WirePart>(r#"{"kind": "?"}"#) else {
            panic!("`?` is a wire kind");
        };
        let text = err.to_string();
        let (_, list) = text.split_once("expected one of ").expect(&text);
        let (list, _) = list.rsplit_once(" at line").expect(&text);
        list.split(", ")
            .map(|name| name.trim_matches('`').to_string())
            .collect()
    }

    #[test]
    fn the_scene_lists_every_wire_kind_but_esp32s3() {
        let listed: Vec<&str> = engine_kinds().iter().map(|(name, _)| *name).collect();
        let wire = wire_kinds();
        let expected: Vec<&str> = wire
            .iter()
            .map(String::as_str)
            .filter(|name| *name != "esp32s3")
            .collect();
        assert_eq!(listed, expected);
        assert!(wire.iter().any(|name| name == "esp32s3"));
    }

    #[test]
    fn every_kind_has_a_unique_name_and_unique_pins() {
        let mut names = BTreeSet::new();
        for (name, pins, _) in KINDS {
            assert!(names.insert(name), "`{name}` is two rows");
            assert!(!pins.is_empty(), "`{name}` has no pins");
            let unique: BTreeSet<_> = pins.iter().collect();
            assert_eq!(unique.len(), pins.len(), "`{name}` repeats a pin");
        }
    }
}
