//! The engine's pin names for each part kind, from the engine's own tables.

use sim_core::netlist::{
    Atmega328p, Bjt, Capacitor, Cst816, CurrentSource, Inductor, Junction, LinearRegulator, Mosfet,
    Ne555, Potentiometer, PulsesReplay, Relay, Resistor, SpiReplay, St77916, Switch, VoltageSource,
    Ws2812,
};

/// Every kind whose pins are all required. An `esp32s3` lets pins go unwired and joins later.
pub(crate) const KINDS: &[(&str, &[&str])] = &[
    ("resistor", &Resistor::PINS),
    ("potentiometer", &Potentiometer::PINS),
    ("voltage_source", &VoltageSource::PINS),
    ("current_source", &CurrentSource::PINS),
    ("diode", &Junction::PINS),
    ("led", &Junction::PINS),
    ("zener", &Junction::PINS),
    ("linear_regulator", &LinearRegulator::PINS),
    ("npn", &Bjt::PINS),
    ("pnp", &Bjt::PINS),
    ("nmos", &Mosfet::PINS),
    ("capacitor", &Capacitor::PINS),
    ("inductor", &Inductor::PINS),
    ("relay", &Relay::PINS),
    ("switch", &Switch::PINS),
    ("ne555", &Ne555::PINS),
    ("atmega328p", &Atmega328p::PINS),
    ("spi_replay", &SpiReplay::PINS),
    ("st77916", &St77916::PINS),
    ("cst816", &Cst816::PINS),
    ("ws2812", &Ws2812::PINS),
    ("pulses_replay", &PulsesReplay::PINS),
];

pub(crate) fn pins(kind: &str) -> Option<&'static [&'static str]> {
    KINDS
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, pins)| *pins)
}
