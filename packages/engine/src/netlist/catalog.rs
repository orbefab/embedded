//! One row per kind. An analog row is a [`Model`] variant, its wire type, and its [`Kind`]
//! answers. A chip row is pins and parse only. The part stays a [`Chip`](super::Chip) variant.
//!
//! The list is four groups in wire order: analogs, the chips before the optional group, that
//! group, and the chips after it. [`engine_kinds`] skips the optional group, so `esp32s3` stays
//! in the wire format and out of the scene list.

use std::collections::BTreeMap;

use serde::Deserialize;

use orbe_model::Time;

use super::bodies::Unpack;
use super::{Chip, LevelField, NetId, RatingKind, Schedule, Serial, SerialField, Value};
use crate::blob::Blobs;
use crate::diagnostic::Diagnostic;
use crate::element::{Observed, OwnedCapacitor, OwnedInductor, Update};
use crate::mna::{Probe, Stamp, Unknowns};
use crate::name::{Name, Quantity, Ref};
use crate::newton::Side;

/// Marker for [`crate::semiconductor::Bjt`] as an `npn`. The polarity is not stored on the part.
pub(crate) struct NpnKind;

/// Marker for [`crate::semiconductor::Bjt`] as a `pnp`.
pub(crate) struct PnpKind;

/// What an analog part answers. Diode and led share one impl. `npn` and `pnp` share the body
/// and differ by `M`.
///
/// The Newton answers, `charges` and `inductors` are required. A forgotten one does not compile.
/// A schedule answer defaults to none. A source, a switch and a potentiometer override the ones
/// they have.
pub(crate) trait Kind<M = ()> {
    fn pins(&self) -> &[NetId];
    fn unknowns(&self) -> Unknowns;
    fn charges(&self) -> Vec<OwnedCapacitor>;
    fn inductors(&self) -> Vec<OwnedInductor>;
    fn stamp(&self, stamp: &mut Stamp<'_>);
    fn reads_iterate(&self) -> bool;
    fn initial(&self, out: &mut [f64]);
    fn update(&self, probe: &Probe<'_>, prior: &[f64], next: &mut [f64]) -> Update;
    fn dc_paths(&self) -> &'static [(usize, usize)];
    fn quantities(&self) -> &'static [Quantity];
    fn ratings(&self) -> (Option<&Value>, Option<&Value>);
    fn observe(&self, probe: &Probe<'_>, at: Time, side: Side) -> Option<Observed>;

    fn next_change_after(&self, _at: Time) -> Option<Time> {
        None
    }
    fn level(&self) -> Option<LevelField<'_>> {
        None
    }
    fn level_mut(&mut self) -> Option<&mut Schedule<Value>> {
        None
    }
    fn serial(&self) -> Option<SerialField<'_>> {
        None
    }
    fn serial_mut(&mut self) -> Option<&mut Serial> {
        None
    }
}

macro_rules! define_kinds {
    (
        analog {
            $(
                $(#[$ameta:meta])*
                $avariant:ident,
                $aty:ty,
                $amarker:ty,
                $aname:literal,
                $awire:ty,
                $aparse:path,
                $apins:expr;
            )*
        }
        chip {
            $(
                $(#[$cmeta:meta])*
                $cvariant:ident,
                $cname:literal,
                $cwire:ty,
                $cparse:path,
                $cpins:expr;
            )*
        }
        optional {
            $(
                $(#[$ometa:meta])*
                $ovariant:ident,
                $oname:literal,
                $owire:ty,
                $oparse:path,
                $opins:expr;
            )*
        }
        chip {
            $(
                $(#[$dmeta:meta])*
                $dvariant:ident,
                $dname:literal,
                $dwire:ty,
                $dparse:path,
                $dpins:expr;
            )*
        }
    ) => {
        /// The closed set of part kinds.
        /// Pin order is that kind's `PINS` const.
        #[derive(Clone, Debug, PartialEq)]
        pub enum Model {
            $(
                $(#[$ameta])*
                $avariant($aty),
            )*
            /// A package modelled as a behavioural black box. A new chip kind is a `Chip` variant,
            /// not a `Model` variant.
            Chip(Chip),
        }

        const _: () = {
            $(
                let _: &'static [&'static str] = $apins;
            )*
            $(
                let _: &'static [&'static str] = $cpins;
            )*
            $(
                let _: &'static [&'static str] = $opins;
                // Not listed by `engine_kinds`. The literal still has to be a name.
                let _: &'static str = $oname;
            )*
            $(
                let _: &'static [&'static str] = $dpins;
            )*
        };

        /// Kind names whose pins are all required, in the order a reader lists them.
        /// An `esp32s3` lets pins go unwired and is not here.
        pub fn engine_kinds() -> &'static [(&'static str, &'static [&'static str])] {
            &[
                $(($aname, $apins),)*
                $(($cname, $cpins),)*
                $(($dname, $dpins),)*
            ]
        }

        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        pub(crate) enum WirePart {
            $(
                $avariant($awire),
            )*
            $(
                $(#[$cmeta])*
                $cvariant($cwire),
            )*
            $(
                $(#[$ometa])*
                $ovariant($owire),
            )*
            $(
                $(#[$dmeta])*
                $dvariant($dwire),
            )*
        }

        pub(crate) fn wire_name(part: &WirePart) -> &String {
            match part {
                $(WirePart::$avariant(wire) => &wire.name,)*
                $(WirePart::$cvariant(wire) => &wire.name,)*
                $(WirePart::$ovariant(wire) => &wire.name,)*
                $(WirePart::$dvariant(wire) => &wire.name,)*
            }
        }

        pub(crate) fn parse_wire(
            seed: u64,
            name: &Name,
            part_ref: &Ref,
            part: WirePart,
            nets: &BTreeMap<String, NetId>,
            blobs: &Blobs,
            out: &mut Vec<Diagnostic>,
        ) -> Option<Model> {
            let mut ctx = super::ParseCtx {
                seed,
                name,
                part: part_ref,
                nets,
                blobs,
                out,
            };
            match part {
                $(WirePart::$avariant(wire) => $aparse(&mut ctx, wire.unpack()),)*
                $(WirePart::$cvariant(wire) => $cparse(&mut ctx, wire.unpack()),)*
                $(WirePart::$ovariant(wire) => $oparse(&mut ctx, wire.unpack()),)*
                $(WirePart::$dvariant(wire) => $dparse(&mut ctx, wire.unpack()),)*
            }
        }

        impl Model {
            /// Pin names in the order of `pins()`. An `esp32s3` lists only the pins it has nets on.
            pub fn pin_names(&self) -> &[&'static str] {
                match self {
                    $(Model::$avariant(_) => $apins,)*
                    Model::Chip(chip) => chip_pin_names(chip),
                }
            }

            /// The quantities a run observes on this part, in `Quantity` order. A two-pin part has
            /// `i`, `v` and `p`, and so does a relay, of its coil. A BJT has `ib`, `ic` and `p`. An
            /// `nmos` has `id` and `p`. A linear regulator has `iin`, `iout` and `p`. A chip and a
            /// potentiometer have none.
            pub fn quantities(&self) -> &'static [Quantity] {
                match self {
                    $(Model::$avariant(part) => <$aty as Kind<$amarker>>::quantities(part),)*
                    Model::Chip(_) => &[],
                }
            }

            pub fn pins(&self) -> &[NetId] {
                match self {
                    $(Model::$avariant(part) => <$aty as Kind<$amarker>>::pins(part),)*
                    Model::Chip(chip) => chip_pins(chip),
                }
            }

            /// Rated quantities in `RatingKind` order. A `null` rating is left out.
            pub fn ratings(&self) -> Vec<(RatingKind, &Value)> {
                let (current, power) = match self {
                    $(Model::$avariant(part) => <$aty as Kind<$amarker>>::ratings(part),)*
                    Model::Chip(_) => (None, None),
                };
                let current = current.map(|v| (RatingKind::Current, v));
                let power = power.map(|v| (RatingKind::Power, v));
                current.into_iter().chain(power).collect()
            }

            /// The next contact edge, source jump, potentiometer move, or device event strictly
            /// after `at`.
            pub fn next_change_after(&self, at: Time) -> Option<Time> {
                if let Some((_, events)) = self.device() {
                    return events.next_after(at);
                }
                match self {
                    $(Model::$avariant(part) => {
                        <$aty as Kind<$amarker>>::next_change_after(part, at)
                    })*
                    Model::Chip(_) => None,
                }
            }

            /// The settable level schedule, with the document field that holds it and that field's
            /// unit. A `steps` voltage, a `steps` current, or a potentiometer position. A `step`,
            /// `pulse`, or `serial` source has none.
            pub fn level(&self) -> Option<LevelField<'_>> {
                match self {
                    $(Model::$avariant(part) => <$aty as Kind<$amarker>>::level(part),)*
                    Model::Chip(_) => None,
                }
            }

            /// [`level`](Self::level)'s schedule, for appending one event.
            pub fn level_mut(&mut self) -> Option<&mut Schedule<Value>> {
                match self {
                    $(Model::$avariant(part) => <$aty as Kind<$amarker>>::level_mut(part),)*
                    Model::Chip(_) => None,
                }
            }

            /// The serial waveform, with the document field that holds it. A `serial` voltage or
            /// current.
            pub fn serial(&self) -> Option<SerialField<'_>> {
                match self {
                    $(Model::$avariant(part) => <$aty as Kind<$amarker>>::serial(part),)*
                    Model::Chip(_) => None,
                }
            }

            pub(super) fn serial_mut(&mut self) -> Option<&mut Serial> {
                match self {
                    $(Model::$avariant(part) => <$aty as Kind<$amarker>>::serial_mut(part),)*
                    Model::Chip(_) => None,
                }
            }

            #[inline]
            pub(crate) fn unknowns(&self) -> Unknowns {
                match self {
                    $(Model::$avariant(part) => <$aty as Kind<$amarker>>::unknowns(part),)*
                    Model::Chip(chip) => crate::chip::unknowns(chip),
                }
            }

            #[inline]
            pub(crate) fn charges(&self) -> Vec<OwnedCapacitor> {
                match self {
                    $(Model::$avariant(part) => <$aty as Kind<$amarker>>::charges(part),)*
                    Model::Chip(_) => Vec::new(),
                }
            }

            #[inline]
            pub(crate) fn inductors(&self) -> Vec<OwnedInductor> {
                match self {
                    $(Model::$avariant(part) => <$aty as Kind<$amarker>>::inductors(part),)*
                    Model::Chip(_) => Vec::new(),
                }
            }

            #[inline]
            pub(crate) fn stamp(&self, stamp: &mut Stamp<'_>) {
                match self {
                    $(Model::$avariant(part) => <$aty as Kind<$amarker>>::stamp(part, stamp),)*
                    Model::Chip(_) => {
                        if let Some(passive) = crate::chip::passive(self) {
                            crate::chip::stamp(passive, *stamp.chip(), stamp);
                        }
                    }
                }
            }

            #[inline]
            pub(crate) fn reads_iterate(&self) -> bool {
                match self {
                    $(Model::$avariant(part) => <$aty as Kind<$amarker>>::reads_iterate(part),)*
                    Model::Chip(chip) => crate::chip::reads_iterate(chip),
                }
            }

            pub(crate) fn initial(&self, out: &mut [f64]) {
                match self {
                    $(Model::$avariant(part) => <$aty as Kind<$amarker>>::initial(part, out),)*
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
                    $(Model::$avariant(part) => {
                        <$aty as Kind<$amarker>>::update(part, probe, prior, next)
                    })*
                    Model::Chip(_) => Update::default(),
                }
            }

            pub(crate) fn dc_paths(&self) -> &'static [(usize, usize)] {
                match self {
                    $(Model::$avariant(part) => <$aty as Kind<$amarker>>::dc_paths(part),)*
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
                    $(Model::$avariant(part) => {
                        <$aty as Kind<$amarker>>::observe(part, probe, at, side)
                    })*
                    Model::Chip(_) => None,
                }
            }
        }
    };
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
        Resistor, crate::passive::Resistor, (), "resistor",
            crate::passive::resistor::WireResistor, super::bodies::parse_resistor,
            &crate::passive::Resistor::PINS;
        Potentiometer, crate::passive::Potentiometer, (), "potentiometer",
            crate::passive::potentiometer::WirePotentiometer, super::bodies::parse_potentiometer,
            &crate::passive::Potentiometer::PINS;
        VoltageSource, crate::passive::VoltageSource, (), "voltage_source",
            crate::passive::voltage_source::WireVoltageSource, super::bodies::parse_voltage_source,
            &crate::passive::VoltageSource::PINS;
        CurrentSource, crate::passive::CurrentSource, (), "current_source",
            crate::passive::current_source::WireCurrentSource, super::bodies::parse_current_source,
            &crate::passive::CurrentSource::PINS;
        Diode, crate::semiconductor::Junction, (), "diode",
            crate::semiconductor::junction::WireJunction, super::bodies::parse_diode,
            &crate::semiconductor::Junction::PINS;
        #[doc = "The same device equation as `Diode`. Its own kind, so an LED-only quantity needs no flag."]
        Led, crate::semiconductor::Junction, (), "led",
            crate::semiconductor::junction::WireJunction, super::bodies::parse_led,
            &crate::semiconductor::Junction::PINS;
        #[doc = "A forward junction and a breakdown junction. See [`Zener`](crate::semiconductor::Zener)."]
        Zener, crate::semiconductor::Zener, (), "zener",
            Box<crate::semiconductor::zener::WireZener>, super::bodies::parse_zener,
            &crate::semiconductor::Junction::PINS;
        #[doc = "A static linear regulator. See [`LinearRegulator`](crate::chip::LinearRegulator)."]
        LinearRegulator, crate::chip::LinearRegulator, (), "linear_regulator",
            Box<crate::chip::regulator::WireLinearRegulator>, super::bodies::parse_linear_regulator,
            &crate::chip::LinearRegulator::PINS;
        Npn, crate::semiconductor::Bjt, NpnKind, "npn",
            Box<crate::semiconductor::transistor::WireBjt>, super::bodies::parse_npn,
            &crate::semiconductor::Bjt::PINS;
        #[doc = "The same device equations as `Npn`, with every junction voltage and pin current negated."]
        Pnp, crate::semiconductor::Bjt, PnpKind, "pnp",
            Box<crate::semiconductor::transistor::WireBjt>, super::bodies::parse_pnp,
            &crate::semiconductor::Bjt::PINS;
        #[doc = "An N-channel MOSFET. Level 1, bulk tied to the source."]
        Nmos, crate::semiconductor::Mosfet, (), "nmos",
            Box<crate::semiconductor::transistor::WireMosfet>, super::bodies::parse_nmos,
            &crate::semiconductor::Mosfet::PINS;
        Capacitor, crate::passive::Capacitor, (), "capacitor",
            crate::passive::capacitor::WireCapacitor, super::bodies::parse_capacitor,
            &crate::passive::Capacitor::PINS;
        Inductor, crate::passive::Inductor, (), "inductor",
            crate::passive::inductor::WireInductor, super::bodies::parse_inductor,
            &crate::passive::Inductor::PINS;
        #[doc = "A coil and a contact that follows the coil current. Passive, with one state."]
        Relay, crate::chip::Relay, (), "relay",
            Box<crate::chip::relay::WireRelay>, super::bodies::parse_relay,
            &crate::chip::Relay::PINS;
        Switch, crate::passive::Switch, (), "switch",
            crate::passive::switch::WireSwitch, super::bodies::parse_switch,
            &crate::passive::Switch::PINS;
    }
    chip {
        Ne555, "ne555",
            Box<crate::chip::ne555::WireNe555>, super::bodies::parse_ne555,
            &crate::chip::Ne555::PINS;
        Atmega328p, "atmega328p",
            Box<crate::mcu::avr::WireAtmega328p>, super::bodies::parse_atmega328p,
            &crate::mcu::Atmega328p::PINS;
        SpiReplay, "spi_replay",
            Box<crate::device::spi_replay::WireSpiReplay>, super::bodies::parse_spi_replay,
            &crate::device::SpiReplay::PINS;
        St77916, "st77916",
            Box<crate::device::st77916::WireSt77916>, super::bodies::parse_st77916,
            &crate::device::St77916::PINS;
        Cst816, "cst816",
            Box<crate::device::cst816::WireCst816>, super::bodies::parse_cst816,
            &crate::device::Cst816::PINS;
    }
    optional {
        Esp32s3, "esp32s3",
            Box<crate::mcu::esp32s3::WireEsp32s3>, super::bodies::parse_esp32s3,
            &crate::mcu::Esp32s3::PINS;
    }
    chip {
        Ws2812, "ws2812",
            Box<crate::device::ws2812::WireWs2812>, super::bodies::parse_ws2812,
            &crate::device::Ws2812::PINS;
        PulsesReplay, "pulses_replay",
            Box<crate::device::pulses_replay::WirePulsesReplay>, super::bodies::parse_pulses_replay,
            &crate::device::PulsesReplay::PINS;
    }
}

#[cfg(test)]
mod tests {
    use super::engine_kinds;

    #[test]
    fn engine_kinds_list_the_required_pins_in_order() {
        let expected: &[(&str, &[&str])] = &[
            ("resistor", &crate::passive::Resistor::PINS),
            ("potentiometer", &crate::passive::Potentiometer::PINS),
            ("voltage_source", &crate::passive::VoltageSource::PINS),
            ("current_source", &crate::passive::CurrentSource::PINS),
            ("diode", &crate::semiconductor::Junction::PINS),
            ("led", &crate::semiconductor::Junction::PINS),
            ("zener", &crate::semiconductor::Junction::PINS),
            ("linear_regulator", &crate::chip::LinearRegulator::PINS),
            ("npn", &crate::semiconductor::Bjt::PINS),
            ("pnp", &crate::semiconductor::Bjt::PINS),
            ("nmos", &crate::semiconductor::Mosfet::PINS),
            ("capacitor", &crate::passive::Capacitor::PINS),
            ("inductor", &crate::passive::Inductor::PINS),
            ("relay", &crate::chip::Relay::PINS),
            ("switch", &crate::passive::Switch::PINS),
            ("ne555", &crate::chip::Ne555::PINS),
            ("atmega328p", &crate::mcu::Atmega328p::PINS),
            ("spi_replay", &crate::device::SpiReplay::PINS),
            ("st77916", &crate::device::St77916::PINS),
            ("cst816", &crate::device::Cst816::PINS),
            ("ws2812", &crate::device::Ws2812::PINS),
            ("pulses_replay", &crate::device::PulsesReplay::PINS),
        ];
        assert_eq!(engine_kinds(), expected);
    }
}
