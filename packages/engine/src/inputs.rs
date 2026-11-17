use orbe_model::{Drive, Time};

use crate::active;
use crate::diagnostic::Diagnostic;
use crate::name::{Name, Ref};
use crate::netlist::{Contact, Model, NetId, Netlist, PartId};
use crate::passive::Switch;
use crate::topology::{find, unite};

/// The digital inputs a firmware reads while nothing drives them.
///
/// A chip's port pin leaks to its supply and its ground, so its net always has a DC path to
/// ground and `net.floating` never names it. With that leakage left out, the level of an input
/// whose pull-up is off and whose net reaches no supply is whatever the leakage and the noise
/// leave there: the firmware reads either level. Whether a pin is an input with its pull-up off,
/// and whether the firmware reads it, are known only while it runs, so each accepted step
/// checks them.
#[derive(Clone)]
pub(crate) struct Inputs {
    candidates: Vec<Candidate>,
    /// The static graph's union-find: every part's DC paths, but an active chip's, which its
    /// drives stand in for, a capacitor's, and a switch's, which its contact stands in for.
    joined: Vec<usize>,
    /// Each switch's part and nets.
    switches: Vec<(PartId, [NetId; 2])>,
    found: Vec<Diagnostic>,
}

/// An output of an active chip whose net holds another part's pin and reaches ground only
/// through a switch or an active chip.
#[derive(Clone)]
struct Candidate {
    active: usize,
    output: usize,
    pin: Ref,
    net: NetId,
}

impl Inputs {
    pub(crate) fn new(netlist: &Netlist, actives: &[active::Chip]) -> Inputs {
        let mut joined: Vec<usize> = (0..netlist.nets().len()).collect();
        let mut switches = Vec::new();
        let is_active = |id: PartId| actives.iter().any(|chip| chip.part == Some(id));
        for (id, part) in netlist.parts() {
            let pins = part.model.pins();
            match &part.model {
                Model::Switch(Switch { pins, .. }) => switches.push((id, *pins)),
                Model::Capacitor(_) => {}
                _ if is_active(id) => {}
                model => {
                    for &(left, right) in model.dc_paths() {
                        unite(&mut joined, pins[left].index(), pins[right].index());
                    }
                }
            }
        }
        let ground = find(&mut joined, 0);
        let mut candidates = Vec::new();
        for (active, chip) in actives.iter().enumerate() {
            let Some(id) = chip.part else { continue };
            let part = netlist.part(id);
            for (output, out) in chip.outputs.iter().enumerate() {
                let net = out.pin;
                if find(&mut joined, net.index()) == ground {
                    continue;
                }
                let shared = netlist
                    .parts()
                    .any(|(other, part)| other != id && part.model.pins().contains(&net));
                if !shared {
                    continue;
                }
                let name = part
                    .model
                    .pin_names()
                    .iter()
                    .zip(part.model.pins())
                    .find(|&(_, &pin)| pin == net)
                    .map(|(&name, _)| name)
                    .expect("an output is on one of its chip's pins");
                candidates.push(Candidate {
                    active,
                    output,
                    pin: Ref::Pin(part.name.clone(), Name::new(name).expect("a pin name")),
                    net,
                });
            }
        }
        Inputs {
            candidates,
            joined,
            switches,
            found: Vec::new(),
        }
    }

    /// Checks the accepted step at `at`, solved with the drives `cores` hold. `read(k, output)`
    /// is when active chip `k`'s firmware first read that output's pin, from a copy of its core
    /// that has run at least to `at`. A candidate warns once, at the first step where its drive
    /// is off, its firmware has read it by then, and its net reaches ground through neither a
    /// closed switch nor another output's drive.
    pub(crate) fn sample(
        &mut self,
        at: Time,
        netlist: &Netlist,
        actives: &[active::Chip],
        cores: &[active::Core],
        read: impl Fn(usize, usize) -> Option<Time>,
    ) {
        let mut joined: Option<Vec<usize>> = None;
        let mut k = 0;
        while k < self.candidates.len() {
            let candidate = &self.candidates[k];
            let off = cores[candidate.active].drive()[candidate.output] == Drive::Off;
            let read_by_now =
                read(candidate.active, candidate.output).is_some_and(|first| first <= at);
            if !off || !read_by_now {
                k += 1;
                continue;
            }
            let joined = joined.get_or_insert_with(|| {
                let mut joined = self.joined.clone();
                for &(id, [a, b]) in &self.switches {
                    let Model::Switch(switch) = &netlist.part(id).model else {
                        unreachable!("a switch's part is a switch");
                    };
                    if *switch.schedule.before(at) == Contact::Closed {
                        unite(&mut joined, a.index(), b.index());
                    }
                }
                for (chip, core) in actives.iter().zip(cores) {
                    for (out, &drive) in chip.outputs.iter().zip(core.drive()) {
                        let rail = match drive {
                            Drive::High | Drive::PullUp => out.vcc,
                            Drive::Low => out.gnd,
                            Drive::Off => continue,
                        };
                        unite(&mut joined, out.pin.index(), rail.index());
                    }
                }
                joined
            });
            let net = candidate.net.index();
            if find(joined, net) == find(joined, 0) {
                k += 1;
                continue;
            }
            let candidate = self.candidates.remove(k);
            self.found.push(Diagnostic::ChipFloatingInput {
                pin: candidate.pin,
                net: Ref::Net(netlist.net_name(candidate.net).clone()),
                at,
            });
        }
    }

    /// One `chip.floating_input` per pin, through the last sample.
    pub(crate) fn findings(&self) -> &[Diagnostic] {
        &self.found
    }
}
