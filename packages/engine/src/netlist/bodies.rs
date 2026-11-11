//! [`Kind`](super::catalog::Kind) for each analog part, and the parse wrapper each catalog row names.
//! A new analog kind implements the trait in its own file.

use std::collections::BTreeMap;

use orbe_model::Time;

use super::catalog::{NpnKind, PnpKind};
use super::{Chip, Kind, LevelField, Model, NetId, Schedule, SerialField, Value};
use crate::blob::Blobs;
use crate::chip::regulator;
use crate::chip::{self, LinearRegulator, Relay};
use crate::diagnostic::Diagnostic;
use crate::element::{Observed, OwnedCapacitor, OwnedInductor, StoreEnd, Update};
use crate::mna::{Probe, Stamp, Unknowns};
use crate::name::{Name, Quantity, Ref};
use crate::netlist::json::{self, AMPERE, VOLT};
use crate::newton::Side;
use crate::passive::{
    Capacitor, CurrentSource, Inductor, Potentiometer, Resistor, Switch, VoltageSource, capacitor,
    current_source, inductor, potentiometer, resistor, switch, voltage_source,
};
use crate::semiconductor::{
    Bjt, Junction, Mosfet, Zener, junction, transistor, transistor::Polarity, zener,
};

const IVP: &[Quantity] = &[Quantity::I, Quantity::V, Quantity::P];
const REGULATOR: &[Quantity] = &[Quantity::Iin, Quantity::Iout, Quantity::P];
const BJT: &[Quantity] = &[Quantity::Ib, Quantity::Ic, Quantity::P];
const NMOS: &[Quantity] = &[Quantity::Id, Quantity::P];

pub(crate) struct ParseCtx<'a> {
    pub(crate) seed: u64,
    pub(crate) name: &'a Name,
    pub(crate) part: &'a Ref,
    pub(crate) nets: &'a BTreeMap<String, NetId>,
    pub(crate) blobs: &'a Blobs,
    pub(crate) out: &'a mut Vec<Diagnostic>,
}

/// The catalog passes each wire payload straight through. A boxed payload
/// becomes the wire. A plain payload already is the wire.
pub(crate) trait Unpack<T> {
    fn unpack(self) -> T;
}

impl<T> Unpack<T> for T {
    fn unpack(self) -> T {
        self
    }
}

impl<T> Unpack<T> for Box<T> {
    fn unpack(self) -> T {
        *self
    }
}

fn two_pin(pins: [NetId; 2], i: f64, probe: &Probe<'_>) -> Observed {
    let v = probe.v(pins[0]) - probe.v(pins[1]);
    Observed::ivp(i, v)
}

fn level_of<'a>(
    schedule: Option<&'a Schedule<Value>>,
    field: &'static str,
    unit: &'static str,
) -> Option<LevelField<'a>> {
    schedule.map(|schedule| LevelField {
        schedule,
        field,
        unit,
    })
}

fn serial_of<'a>(
    waveform: Option<&'a super::Serial>,
    field: &'static str,
) -> Option<SerialField<'a>> {
    waveform.map(|waveform| SerialField { waveform, field })
}

pub(super) fn parse_resistor(
    ctx: &mut ParseCtx<'_>,
    wire: crate::passive::resistor::WireResistor,
) -> Option<Model> {
    resistor::resolve_resistor(ctx.name, ctx.part, wire, ctx.nets, ctx.out)
}

pub(super) fn parse_potentiometer(
    ctx: &mut ParseCtx<'_>,
    wire: crate::passive::potentiometer::WirePotentiometer,
) -> Option<Model> {
    potentiometer::resolve_potentiometer(ctx.name, ctx.part, wire, ctx.nets, ctx.out)
}

pub(super) fn parse_voltage_source(
    ctx: &mut ParseCtx<'_>,
    wire: crate::passive::voltage_source::WireVoltageSource,
) -> Option<Model> {
    voltage_source::resolve_voltage_source(ctx.name, ctx.part, wire, ctx.nets, ctx.out)
}

pub(super) fn parse_current_source(
    ctx: &mut ParseCtx<'_>,
    wire: crate::passive::current_source::WireCurrentSource,
) -> Option<Model> {
    current_source::resolve_current_source(ctx.name, ctx.part, wire, ctx.nets, ctx.out)
}

pub(super) fn parse_diode(
    ctx: &mut ParseCtx<'_>,
    wire: crate::semiconductor::junction::WireJunction,
) -> Option<Model> {
    junction::resolve_junction(ctx.name, ctx.part, wire, ctx.nets, ctx.out).map(Model::Diode)
}

pub(super) fn parse_led(
    ctx: &mut ParseCtx<'_>,
    wire: crate::semiconductor::junction::WireJunction,
) -> Option<Model> {
    junction::resolve_junction(ctx.name, ctx.part, wire, ctx.nets, ctx.out).map(Model::Led)
}

pub(super) fn parse_zener(
    ctx: &mut ParseCtx<'_>,
    wire: crate::semiconductor::zener::WireZener,
) -> Option<Model> {
    zener::resolve_zener(ctx.name, ctx.part, wire, ctx.nets, ctx.out).map(Model::Zener)
}

pub(super) fn parse_linear_regulator(
    ctx: &mut ParseCtx<'_>,
    wire: crate::chip::regulator::WireLinearRegulator,
) -> Option<Model> {
    regulator::resolve_regulator(ctx.name, ctx.part, wire, ctx.nets, ctx.out)
        .map(Model::LinearRegulator)
}

pub(super) fn parse_npn(
    ctx: &mut ParseCtx<'_>,
    wire: crate::semiconductor::transistor::WireBjt,
) -> Option<Model> {
    transistor::resolve_bjt(ctx.name, ctx.part, wire, ctx.nets, ctx.out).map(Model::Npn)
}

pub(super) fn parse_pnp(
    ctx: &mut ParseCtx<'_>,
    wire: crate::semiconductor::transistor::WireBjt,
) -> Option<Model> {
    transistor::resolve_bjt(ctx.name, ctx.part, wire, ctx.nets, ctx.out).map(Model::Pnp)
}

pub(super) fn parse_nmos(
    ctx: &mut ParseCtx<'_>,
    wire: crate::semiconductor::transistor::WireMosfet,
) -> Option<Model> {
    transistor::resolve_mosfet(ctx.name, ctx.part, wire, ctx.nets, ctx.out).map(Model::Nmos)
}

pub(super) fn parse_capacitor(
    ctx: &mut ParseCtx<'_>,
    wire: crate::passive::capacitor::WireCapacitor,
) -> Option<Model> {
    capacitor::resolve_capacitor(ctx.name, ctx.part, wire, ctx.nets, ctx.out)
}

pub(super) fn parse_inductor(
    ctx: &mut ParseCtx<'_>,
    wire: crate::passive::inductor::WireInductor,
) -> Option<Model> {
    inductor::resolve_inductor(ctx.name, ctx.part, wire, ctx.nets, ctx.out)
}

pub(super) fn parse_relay(
    ctx: &mut ParseCtx<'_>,
    wire: crate::chip::relay::WireRelay,
) -> Option<Model> {
    crate::chip::relay::resolve_relay(ctx.name, ctx.part, wire, ctx.nets, ctx.out).map(Model::Relay)
}

pub(super) fn parse_switch(
    ctx: &mut ParseCtx<'_>,
    wire: crate::passive::switch::WireSwitch,
) -> Option<Model> {
    switch::resolve_switch(ctx.seed, ctx.name, ctx.part, wire, ctx.nets, ctx.out).map(Model::Switch)
}

pub(super) fn parse_ne555(
    ctx: &mut ParseCtx<'_>,
    wire: crate::chip::ne555::WireNe555,
) -> Option<Model> {
    crate::chip::ne555::resolve_ne555(ctx.name, ctx.part, wire, ctx.nets, ctx.out)
        .map(|chip| Model::Chip(Chip::Ne555(Box::new(chip))))
}

pub(super) fn parse_atmega328p(
    ctx: &mut ParseCtx<'_>,
    wire: crate::mcu::avr::WireAtmega328p,
) -> Option<Model> {
    crate::mcu::avr::resolve_atmega328p(ctx.name, ctx.part, wire, ctx.nets, ctx.out)
        .map(|chip| Model::Chip(Chip::Atmega328p(Box::new(chip))))
}

pub(super) fn parse_spi_replay(
    ctx: &mut ParseCtx<'_>,
    wire: crate::device::spi_replay::WireSpiReplay,
) -> Option<Model> {
    crate::device::spi_replay::resolve_spi_replay(ctx.name, ctx.part, wire, ctx.nets, ctx.out)
        .map(|chip| Model::Chip(Chip::SpiReplay(Box::new(chip))))
}

pub(super) fn parse_st77916(
    ctx: &mut ParseCtx<'_>,
    wire: crate::device::st77916::WireSt77916,
) -> Option<Model> {
    crate::device::st77916::resolve_st77916(ctx.name, ctx.part, wire, ctx.nets, ctx.out)
        .map(|chip| Model::Chip(Chip::St77916(Box::new(chip))))
}

pub(super) fn parse_cst816(
    ctx: &mut ParseCtx<'_>,
    wire: crate::device::cst816::WireCst816,
) -> Option<Model> {
    crate::device::cst816::resolve_cst816(ctx.name, ctx.part, wire, ctx.nets, ctx.out)
        .map(|chip| Model::Chip(Chip::Cst816(Box::new(chip))))
}

pub(super) fn parse_esp32s3(
    ctx: &mut ParseCtx<'_>,
    wire: crate::mcu::esp32s3::WireEsp32s3,
) -> Option<Model> {
    crate::mcu::esp32s3::resolve_esp32s3(ctx.name, ctx.part, wire, ctx.nets, ctx.blobs, ctx.out)
        .map(|chip| Model::Chip(Chip::Esp32s3(Box::new(chip))))
}

pub(super) fn parse_ws2812(
    ctx: &mut ParseCtx<'_>,
    wire: crate::device::ws2812::WireWs2812,
) -> Option<Model> {
    crate::device::ws2812::resolve_ws2812(ctx.name, ctx.part, wire, ctx.nets, ctx.out)
        .map(|chip| Model::Chip(Chip::Ws2812(Box::new(chip))))
}

pub(super) fn parse_pulses_replay(
    ctx: &mut ParseCtx<'_>,
    wire: crate::device::pulses_replay::WirePulsesReplay,
) -> Option<Model> {
    crate::device::pulses_replay::resolve_pulses_replay(ctx.name, ctx.part, wire, ctx.nets, ctx.out)
        .map(|chip| Model::Chip(Chip::PulsesReplay(Box::new(chip))))
}

impl Kind for Resistor {
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
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        resistor::stamp(self, stamp);
    }
    fn reads_iterate(&self) -> bool {
        false
    }
    fn initial(&self, _out: &mut [f64]) {}
    fn update(&self, _probe: &Probe<'_>, _prior: &[f64], _next: &mut [f64]) -> Update {
        Update::default()
    }
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[(0, 1)]
    }
    fn quantities(&self) -> &'static [Quantity] {
        IVP
    }
    fn ratings(&self) -> (Option<&Value>, Option<&Value>) {
        (None, self.max_power.as_ref())
    }
    fn observe(&self, probe: &Probe<'_>, _at: Time, _side: Side) -> Option<Observed> {
        Some(two_pin(self.pins, resistor::current(self, probe), probe))
    }
}

impl Kind for Potentiometer {
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
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        potentiometer::stamp(self, stamp);
    }
    fn reads_iterate(&self) -> bool {
        false
    }
    fn initial(&self, _out: &mut [f64]) {}
    fn update(&self, _probe: &Probe<'_>, _prior: &[f64], _next: &mut [f64]) -> Update {
        Update::default()
    }
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[(0, 1), (1, 2)]
    }
    fn quantities(&self) -> &'static [Quantity] {
        &[]
    }
    fn ratings(&self) -> (Option<&Value>, Option<&Value>) {
        (None, None)
    }
    fn observe(&self, _probe: &Probe<'_>, _at: Time, _side: Side) -> Option<Observed> {
        None
    }
    fn next_change_after(&self, at: Time) -> Option<Time> {
        self.position.next_change_after(at)
    }
    fn level(&self) -> Option<LevelField<'_>> {
        level_of(Some(&self.position), "position", json::FRACTION_UNIT)
    }
    fn level_mut(&mut self) -> Option<&mut Schedule<Value>> {
        Some(&mut self.position)
    }
}

impl Kind for VoltageSource {
    fn pins(&self) -> &[NetId] {
        &self.pins
    }
    fn unknowns(&self) -> Unknowns {
        Unknowns {
            branches: 1,
            ..Unknowns::default()
        }
    }
    fn charges(&self) -> Vec<OwnedCapacitor> {
        Vec::new()
    }
    fn inductors(&self) -> Vec<OwnedInductor> {
        Vec::new()
    }
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        voltage_source::stamp(self, stamp);
    }
    fn reads_iterate(&self) -> bool {
        false
    }
    fn initial(&self, _out: &mut [f64]) {}
    fn update(&self, _probe: &Probe<'_>, _prior: &[f64], _next: &mut [f64]) -> Update {
        Update::default()
    }
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[(0, 1)]
    }
    fn quantities(&self) -> &'static [Quantity] {
        IVP
    }
    fn ratings(&self) -> (Option<&Value>, Option<&Value>) {
        (None, None)
    }
    fn observe(&self, probe: &Probe<'_>, _at: Time, _side: Side) -> Option<Observed> {
        Some(two_pin(self.pins, voltage_source::current(probe), probe))
    }
    fn next_change_after(&self, at: Time) -> Option<Time> {
        self.voltage.next_change_after(at)
    }
    fn level(&self) -> Option<LevelField<'_>> {
        level_of(self.voltage.as_steps(), "voltage", VOLT)
    }
    fn level_mut(&mut self) -> Option<&mut Schedule<Value>> {
        self.voltage.steps_mut()
    }
    fn serial(&self) -> Option<SerialField<'_>> {
        serial_of(self.voltage.as_serial(), "voltage")
    }
    fn serial_mut(&mut self) -> Option<&mut super::Serial> {
        self.voltage.serial_mut()
    }
}

impl Kind for CurrentSource {
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
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        current_source::stamp(self, stamp);
    }
    fn reads_iterate(&self) -> bool {
        false
    }
    fn initial(&self, _out: &mut [f64]) {}
    fn update(&self, _probe: &Probe<'_>, _prior: &[f64], _next: &mut [f64]) -> Update {
        Update::default()
    }
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[]
    }
    fn quantities(&self) -> &'static [Quantity] {
        IVP
    }
    fn ratings(&self) -> (Option<&Value>, Option<&Value>) {
        (None, None)
    }
    fn observe(&self, probe: &Probe<'_>, at: Time, side: Side) -> Option<Observed> {
        Some(two_pin(
            self.pins,
            current_source::current(self, at, side),
            probe,
        ))
    }
    fn next_change_after(&self, at: Time) -> Option<Time> {
        self.current.next_change_after(at)
    }
    fn level(&self) -> Option<LevelField<'_>> {
        level_of(self.current.as_steps(), "current", AMPERE)
    }
    fn level_mut(&mut self) -> Option<&mut Schedule<Value>> {
        self.current.steps_mut()
    }
    fn serial(&self) -> Option<SerialField<'_>> {
        serial_of(self.current.as_serial(), "current")
    }
    fn serial_mut(&mut self) -> Option<&mut super::Serial> {
        self.current.serial_mut()
    }
}

/// Diode and led. The equations match. The variant name does not.
impl Kind for Junction {
    fn pins(&self) -> &[NetId] {
        &self.pins
    }
    fn unknowns(&self) -> Unknowns {
        Unknowns {
            internal_nodes: 1,
            bare_internal: true,
            branches: 0,
            junctions: 1,
        }
    }
    fn charges(&self) -> Vec<OwnedCapacitor> {
        Vec::new()
    }
    fn inductors(&self) -> Vec<OwnedInductor> {
        Vec::new()
    }
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        junction::stamp(self, stamp);
    }
    fn reads_iterate(&self) -> bool {
        true
    }
    fn initial(&self, out: &mut [f64]) {
        out[0] = junction::initial(self);
    }
    fn update(&self, probe: &Probe<'_>, prior: &[f64], next: &mut [f64]) -> Update {
        let (limited, update) = junction::update(self, probe, prior[0]);
        next[0] = limited;
        update
    }
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[(0, 1)]
    }
    fn quantities(&self) -> &'static [Quantity] {
        IVP
    }
    fn ratings(&self) -> (Option<&Value>, Option<&Value>) {
        (self.max_current.as_ref(), self.max_power.as_ref())
    }
    fn observe(&self, probe: &Probe<'_>, _at: Time, _side: Side) -> Option<Observed> {
        Some(two_pin(self.pins, junction::current(self, probe), probe))
    }
}

impl Kind for Zener {
    fn pins(&self) -> &[NetId] {
        &self.pins
    }
    fn unknowns(&self) -> Unknowns {
        Unknowns {
            internal_nodes: 2,
            bare_internal: true,
            branches: 0,
            junctions: 2,
        }
    }
    fn charges(&self) -> Vec<OwnedCapacitor> {
        Vec::new()
    }
    fn inductors(&self) -> Vec<OwnedInductor> {
        Vec::new()
    }
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        zener::stamp(self, stamp);
    }
    fn reads_iterate(&self) -> bool {
        true
    }
    fn initial(&self, out: &mut [f64]) {
        out[..2].copy_from_slice(&zener::initial(self));
    }
    fn update(&self, probe: &Probe<'_>, prior: &[f64], next: &mut [f64]) -> Update {
        let (limited, update) = zener::update(self, probe, prior);
        next[..2].copy_from_slice(&limited);
        update
    }
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[(0, 1)]
    }
    fn quantities(&self) -> &'static [Quantity] {
        IVP
    }
    fn ratings(&self) -> (Option<&Value>, Option<&Value>) {
        (self.max_current.as_ref(), self.max_power.as_ref())
    }
    fn observe(&self, probe: &Probe<'_>, _at: Time, _side: Side) -> Option<Observed> {
        Some(two_pin(self.pins, zener::current(self, probe), probe))
    }
}

impl Kind for LinearRegulator {
    fn pins(&self) -> &[NetId] {
        &self.pins
    }
    fn unknowns(&self) -> Unknowns {
        Unknowns {
            branches: 1,
            ..Unknowns::default()
        }
    }
    fn charges(&self) -> Vec<OwnedCapacitor> {
        Vec::new()
    }
    fn inductors(&self) -> Vec<OwnedInductor> {
        Vec::new()
    }
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        regulator::stamp(self, stamp);
    }
    fn reads_iterate(&self) -> bool {
        true
    }
    fn initial(&self, _out: &mut [f64]) {}
    fn update(&self, probe: &Probe<'_>, _prior: &[f64], _next: &mut [f64]) -> Update {
        regulator::update(self, probe)
    }
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[(0, 2), (1, 2)]
    }
    fn quantities(&self) -> &'static [Quantity] {
        REGULATOR
    }
    fn ratings(&self) -> (Option<&Value>, Option<&Value>) {
        (self.max_current.as_ref(), self.max_power.as_ref())
    }
    fn observe(&self, probe: &Probe<'_>, _at: Time, _side: Side) -> Option<Observed> {
        Some(Observed::values(regulator::observe(self, probe), 3))
    }
}

macro_rules! impl_bjt {
    ($marker:ty, $polarity:expr) => {
        impl Kind<$marker> for Bjt {
            fn pins(&self) -> &[NetId] {
                &self.pins
            }
            fn unknowns(&self) -> Unknowns {
                Unknowns {
                    internal_nodes: 3,
                    bare_internal: true,
                    branches: 2,
                    junctions: 2,
                }
            }
            fn charges(&self) -> Vec<OwnedCapacitor> {
                transistor::charges(self)
            }
            fn inductors(&self) -> Vec<OwnedInductor> {
                Vec::new()
            }
            #[inline]
            fn stamp(&self, stamp: &mut Stamp<'_>) {
                transistor::stamp(self, $polarity, stamp);
            }
            fn reads_iterate(&self) -> bool {
                true
            }
            fn initial(&self, out: &mut [f64]) {
                transistor::initial(self, out);
            }
            fn update(&self, probe: &Probe<'_>, prior: &[f64], next: &mut [f64]) -> Update {
                transistor::update(self, $polarity, probe, prior, next)
            }
            fn dc_paths(&self) -> &'static [(usize, usize)] {
                &[(0, 1), (1, 2)]
            }
            fn quantities(&self) -> &'static [Quantity] {
                BJT
            }
            fn ratings(&self) -> (Option<&Value>, Option<&Value>) {
                (self.max_current.as_ref(), self.max_power.as_ref())
            }
            fn observe(&self, probe: &Probe<'_>, _at: Time, _side: Side) -> Option<Observed> {
                Some(Observed::values(transistor::observe(self, probe), 3))
            }
        }
    };
}

impl_bjt!(NpnKind, Polarity::Npn);
impl_bjt!(PnpKind, Polarity::Pnp);

impl Kind for Mosfet {
    fn pins(&self) -> &[NetId] {
        &self.pins
    }
    fn unknowns(&self) -> Unknowns {
        Unknowns {
            internal_nodes: 0,
            bare_internal: false,
            branches: 3,
            junctions: 2,
        }
    }
    fn charges(&self) -> Vec<OwnedCapacitor> {
        transistor::mos_charges(self)
    }
    fn inductors(&self) -> Vec<OwnedInductor> {
        Vec::new()
    }
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        transistor::mos_stamp(self, stamp);
    }
    fn reads_iterate(&self) -> bool {
        true
    }
    fn initial(&self, out: &mut [f64]) {
        transistor::mos_initial(out);
    }
    fn update(&self, probe: &Probe<'_>, prior: &[f64], next: &mut [f64]) -> Update {
        transistor::mos_update(self, probe, prior, next)
    }
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[(0, 2)]
    }
    fn quantities(&self) -> &'static [Quantity] {
        NMOS
    }
    fn ratings(&self) -> (Option<&Value>, Option<&Value>) {
        (self.max_current.as_ref(), self.max_power.as_ref())
    }
    fn observe(&self, probe: &Probe<'_>, _at: Time, _side: Side) -> Option<Observed> {
        let [id, p] = transistor::mos_observe(self, probe);
        let len = self.quantities().len();
        debug_assert_eq!(len, 2);
        Some(Observed::values([id, p, 0.0], len))
    }
}

impl Kind for Capacitor {
    fn pins(&self) -> &[NetId] {
        &self.pins
    }
    fn unknowns(&self) -> Unknowns {
        Unknowns {
            branches: 1,
            ..Unknowns::default()
        }
    }
    fn charges(&self) -> Vec<OwnedCapacitor> {
        vec![OwnedCapacitor {
            k: 0,
            ends: self.pins.map(StoreEnd::Net),
            farad: self.capacitance.si.get(),
        }]
    }
    fn inductors(&self) -> Vec<OwnedInductor> {
        Vec::new()
    }
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        capacitor::stamp(self, stamp);
    }
    fn reads_iterate(&self) -> bool {
        false
    }
    fn initial(&self, _out: &mut [f64]) {}
    fn update(&self, _probe: &Probe<'_>, _prior: &[f64], _next: &mut [f64]) -> Update {
        Update::default()
    }
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        if self.initial.is_some() {
            &[(0, 1)]
        } else {
            &[]
        }
    }
    fn quantities(&self) -> &'static [Quantity] {
        IVP
    }
    fn ratings(&self) -> (Option<&Value>, Option<&Value>) {
        (None, None)
    }
    fn observe(&self, probe: &Probe<'_>, _at: Time, _side: Side) -> Option<Observed> {
        Some(two_pin(self.pins, capacitor::current(probe), probe))
    }
}

impl Kind for Inductor {
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
        vec![inductor::owned(self)]
    }
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        inductor::stamp(self, stamp);
    }
    fn reads_iterate(&self) -> bool {
        false
    }
    fn initial(&self, _out: &mut [f64]) {}
    fn update(&self, _probe: &Probe<'_>, _prior: &[f64], _next: &mut [f64]) -> Update {
        Update::default()
    }
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[(0, 1)]
    }
    fn quantities(&self) -> &'static [Quantity] {
        IVP
    }
    fn ratings(&self) -> (Option<&Value>, Option<&Value>) {
        (None, None)
    }
    fn observe(&self, probe: &Probe<'_>, _at: Time, _side: Side) -> Option<Observed> {
        Some(two_pin(self.pins, inductor::current(probe), probe))
    }
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
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        switch::stamp(self, stamp);
    }
    fn reads_iterate(&self) -> bool {
        false
    }
    fn initial(&self, _out: &mut [f64]) {}
    fn update(&self, _probe: &Probe<'_>, _prior: &[f64], _next: &mut [f64]) -> Update {
        Update::default()
    }
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[(0, 1)]
    }
    fn quantities(&self) -> &'static [Quantity] {
        IVP
    }
    fn ratings(&self) -> (Option<&Value>, Option<&Value>) {
        (None, None)
    }
    fn observe(&self, probe: &Probe<'_>, at: Time, side: Side) -> Option<Observed> {
        Some(two_pin(
            self.pins,
            switch::current(self, probe, at, side),
            probe,
        ))
    }
    fn next_change_after(&self, at: Time) -> Option<Time> {
        self.schedule.next_change_after(at)
    }
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
        vec![chip::relay_coil(self)]
    }
    #[inline]
    fn stamp(&self, stamp: &mut Stamp<'_>) {
        chip::stamp(chip::Passive::Relay(self), *stamp.chip(), stamp);
    }
    fn reads_iterate(&self) -> bool {
        false
    }
    fn initial(&self, _out: &mut [f64]) {}
    fn update(&self, _probe: &Probe<'_>, _prior: &[f64], _next: &mut [f64]) -> Update {
        Update::default()
    }
    fn dc_paths(&self) -> &'static [(usize, usize)] {
        &[
            (Relay::COIL_A, Relay::COIL_B),
            (Relay::COM, Relay::NO),
            (Relay::COM, Relay::NC),
        ]
    }
    fn quantities(&self) -> &'static [Quantity] {
        IVP
    }
    fn ratings(&self) -> (Option<&Value>, Option<&Value>) {
        (None, None)
    }
    fn observe(&self, probe: &Probe<'_>, _at: Time, _side: Side) -> Option<Observed> {
        Some(two_pin(
            [self.pins[Relay::COIL_A], self.pins[Relay::COIL_B]],
            inductor::current(probe),
            probe,
        ))
    }
}
