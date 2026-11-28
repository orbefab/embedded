//! Record-routing check: a `Pulses` or `Spi` record carries no pin edges, so any element on
//! an active net other than the sender's own pin, a part that takes that record kind on that
//! pin, or a pull-up or pull-down resistor to a fixed supply or ground rail would silently miss
//! the signal. The first such record ends the run with `chip.link_unsupported`.

use orbe_model::{Miso, Time};

use crate::diagnostic::{Diagnostic, LinkKind};
use crate::link::{Record, RecordKind};
use crate::name::{Name, Ref};
use crate::netlist::{Model, NetId, Netlist, PartId};
use crate::newton::Circuit;

/// Which SPI signal an active line carries.
#[derive(Clone, Copy)]
enum Line {
    Pulses,
    SpiCs,
    SpiSclk,
    SpiData,
}

/// Precomputed net incidence, fixed rails and record-taking pins for fast routing checks.
#[derive(Default)]
pub(crate) struct Routing {
    /// Indexed by `NetId::index()`. True for ground, DC voltage-source pins, and linear
    /// regulator output/ground pins.
    fixed_rails: Vec<bool>,
    /// Indexed by `NetId::index()`. Every `(part, pin_index)` wired to that net, in part order
    /// then pin order.
    net_pins: Vec<Vec<(PartId, usize)>>,
    /// Netlist parts that take `Spi` records, with package pin indices `[CS, SCLK, D0..D3]`.
    spi_targets: Vec<(PartId, [usize; 6])>,
    /// Netlist parts that take `Pulses` records, with their `Pulses` input package pin index.
    pulses_targets: Vec<(PartId, usize)>,
}

impl Routing {
    pub(crate) fn new(circuit: &Circuit, has_senders: bool) -> Routing {
        if !has_senders {
            return Routing::default();
        }
        let net_count = circuit.netlist.nets().len();
        let mut fixed_rails = vec![false; net_count];
        fixed_rails[NetId::GROUND.index()] = true;
        let mut net_pins = vec![Vec::new(); net_count];
        for (part_id, part) in circuit.netlist.parts() {
            match &part.model {
                Model::VoltageSource(vs)
                    if vs.voltage.as_steps().is_some_and(|s| s.events().is_empty()) =>
                {
                    fixed_rails[vs.pins[0].index()] = true;
                    fixed_rails[vs.pins[1].index()] = true;
                }
                Model::LinearRegulator(reg) => {
                    // `LinearRegulator::PINS` is `["in", "out", "gnd"]`.
                    fixed_rails[reg.pins[1].index()] = true;
                    fixed_rails[reg.pins[2].index()] = true;
                }
                _ => {}
            }
            for (pin_idx, &net) in part.model.pins().iter().enumerate() {
                net_pins[net.index()].push((part_id, pin_idx));
            }
        }
        let mut spi_targets = Vec::new();
        let mut pulses_targets = Vec::new();
        for chip in &circuit.actives {
            let Some(part) = chip.part else {
                continue;
            };
            if let Some(target) = chip.takes.spi
                && let Some(pins) = target.pins
            {
                spi_targets.push((part, pins));
            }
            if let Some(pin) = chip.takes.pulses_pin {
                pulses_targets.push((part, pin));
            }
        }
        Routing {
            fixed_rails,
            net_pins,
            spi_targets,
            pulses_targets,
        }
    }

    /// Returns `Some(Diagnostic::ChipLinkUnsupported)` if any active net of `record` reaches a
    /// pin that will not receive the record faithfully. `Duty` records also drive real pin
    /// edges and are not restricted here.
    pub(crate) fn check(
        &self,
        netlist: &Netlist,
        sender: Option<PartId>,
        record: &Record,
        at: Time,
    ) -> Option<Diagnostic> {
        let sender = sender?;
        match &record.kind {
            RecordKind::Duty { .. } => None,
            RecordKind::Pulses { net, .. } => self.check_line(
                netlist,
                sender,
                record.pin,
                *net,
                record.peripheral,
                Line::Pulses,
                at,
            ),
            RecordKind::Spi { spi, nets, pins } => {
                let read = if spi.miso == Miso::None { 1 } else { 2 };
                let used = spi
                    .phases
                    .iter()
                    .map(|phase| phase.lanes.count())
                    .chain([spi.lanes.count(), read])
                    .max()
                    .unwrap_or(1);
                if let (Some(net), Some(pin)) = (nets.cs, pins.cs)
                    && let Some(d) = self.check_line(
                        netlist,
                        sender,
                        pin,
                        net,
                        record.peripheral,
                        Line::SpiCs,
                        at,
                    )
                {
                    return Some(d);
                }
                if let (Some(net), Some(pin)) = (nets.sclk, pins.sclk)
                    && let Some(d) = self.check_line(
                        netlist,
                        sender,
                        pin,
                        net,
                        record.peripheral,
                        Line::SpiSclk,
                        at,
                    )
                {
                    return Some(d);
                }
                for lane in 0..usize::from(used) {
                    if let (Some(net), Some(pin)) = (nets.data[lane], pins.data[lane])
                        && let Some(d) = self.check_line(
                            netlist,
                            sender,
                            pin,
                            net,
                            record.peripheral,
                            Line::SpiData,
                            at,
                        )
                    {
                        return Some(d);
                    }
                }
                None
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn check_line(
        &self,
        netlist: &Netlist,
        sender: PartId,
        sender_pin: &'static str,
        net: NetId,
        peripheral: &'static str,
        line: Line,
        at: Time,
    ) -> Option<Diagnostic> {
        for &(other_part, other_pin) in &self.net_pins[net.index()] {
            let part = netlist.part(other_part);
            let other_pin_name = part.model.pin_names()[other_pin];
            if other_part == sender && other_pin_name == sender_pin {
                continue;
            }
            if self.takes(other_part, other_pin, line) {
                continue;
            }
            if self.is_pull_resistor(netlist, other_part, net) {
                continue;
            }
            let sender_name = netlist.part(sender).name.clone();
            let kind = match line {
                Line::Pulses => LinkKind::Pulses,
                Line::SpiCs | Line::SpiSclk | Line::SpiData => LinkKind::Spi,
            };
            return Some(Diagnostic::ChipLinkUnsupported {
                pin: Ref::Pin(
                    sender_name,
                    Name::new(sender_pin).expect("a part pin name is a valid Name"),
                ),
                other: Ref::Pin(
                    part.name.clone(),
                    Name::new(other_pin_name).expect("a part pin name is a valid Name"),
                ),
                at,
                peripheral,
                kind,
            });
        }
        None
    }

    fn takes(&self, part: PartId, pin: usize, line: Line) -> bool {
        match line {
            Line::Pulses => self.pulses_targets.contains(&(part, pin)),
            Line::SpiCs => self
                .spi_targets
                .iter()
                .any(|&(target, pins)| target == part && pins[0] == pin),
            Line::SpiSclk => self
                .spi_targets
                .iter()
                .any(|&(target, pins)| target == part && pins[1] == pin),
            Line::SpiData => self
                .spi_targets
                .iter()
                .any(|&(target, pins)| target == part && pins[2..6].contains(&pin)),
        }
    }

    /// True for a 2-pin resistor whose other end sits on a fixed supply or ground rail.
    fn is_pull_resistor(&self, netlist: &Netlist, part: PartId, on_net: NetId) -> bool {
        let Model::Resistor(resistor) = &netlist.part(part).model else {
            return false;
        };
        let other = if resistor.pins[0] == on_net {
            resistor.pins[1]
        } else {
            resistor.pins[0]
        };
        other != on_net && self.fixed_rails[other.index()]
    }
}
