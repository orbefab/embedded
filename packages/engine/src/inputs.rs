use orbe_model::{Drive, Time};

use crate::active;
use crate::device::max7219;
use crate::diagnostic::{Diagnostic, ProtocolRule};
use crate::finite::Finite;
use crate::name::{Name, Ref};
use crate::netlist::{Chip, Contact, Model, NetId, Netlist, PartId};
use crate::observe::Columns;
use crate::passive::Switch;
use crate::topology::{find, unite};

/// Margin inside `VIL`/`VIH` so exact boundary voltages count as valid low/high, not indeterminate.
const BAND_MARGIN_V: f64 = 1e-6;

/// The digital inputs a firmware reads while nothing drives them, and driven `max7219` logic
/// inputs held in the indeterminate band between `VIL` and `VIH`.
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
    /// `max7219` logic inputs monitored for driven voltages held in `(VIL, VIH)`.
    bands: Vec<InputBand>,
    /// The static graph's union-find: every part's DC paths, but an active chip's, which its
    /// drives stand in for, a capacitor's, and a switch's, which its contact stands in for.
    joined: Vec<usize>,
    /// Each switch's part and nets.
    switches: Vec<(PartId, [NetId; 2])>,
    found: Vec<Diagnostic>,
}

/// A `max7219`'s `[DIN, CLK, LOAD]` inputs checked for driven voltages sitting between `VIL` and
/// `VIH` longer than a normal edge transition while the chip is powered.
#[derive(Clone)]
struct InputBand {
    part: Ref,
    vcc: usize,
    gnd: usize,
    pins: [(NetId, usize); 3],
    vil: f64,
    vih: f64,
    since: [Option<Time>; 3],
    warned: bool,
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
    pub(crate) fn new(netlist: &Netlist, actives: &[active::Chip], columns: &Columns) -> Inputs {
        let mut joined: Vec<usize> = (0..netlist.nets().len()).collect();
        let mut switches = Vec::new();
        let mut bands = Vec::new();
        let is_active = |id: PartId| actives.iter().any(|chip| chip.part == Some(id));
        for (id, part) in netlist.parts() {
            let pins = part.model.pins();
            let unite_dc_paths = |joined: &mut [usize], model: &Model| {
                for &(left, right) in model.dc_paths() {
                    unite(joined, pins[left].index(), pins[right].index());
                }
            };
            match &part.model {
                Model::Switch(Switch { pins, .. }) => switches.push((id, *pins)),
                Model::Capacitor(_) => {}
                Model::Chip(chip) => match chip {
                    Chip::Max7219(dev) => {
                        bands.push(InputBand {
                            part: Ref::Part(part.name.clone()),
                            vcc: columns.net(dev.pins[0]),
                            gnd: columns.net(dev.pins[1]),
                            pins: [
                                (dev.pins[2], columns.net(dev.pins[2])),
                                (dev.pins[3], columns.net(dev.pins[3])),
                                (dev.pins[4], columns.net(dev.pins[4])),
                            ],
                            vil: dev.input_threshold.low.si.get(),
                            vih: dev.input_threshold.high.si.get(),
                            since: [None; 3],
                            warned: false,
                        });
                    }
                    Chip::Atmega328p(_)
                    | Chip::SpiReplay(_)
                    | Chip::St77916(_)
                    | Chip::Ssd1306(_)
                    | Chip::Hd44780I2c(_)
                    | Chip::Hd44780(_)
                    | Chip::Cst816(_)
                    | Chip::Dht22(_)
                    | Chip::Bmp280(_)
                    | Chip::Hc595(_)
                    | Chip::Ws2812(_)
                    | Chip::PulsesReplay(_)
                    | Chip::Esp32s3(_)
                    | Chip::Sg90(_) => {
                        debug_assert!(is_active(id));
                    }
                    Chip::Ne555(_) => {
                        debug_assert!(!is_active(id));
                        unite_dc_paths(&mut joined, &part.model);
                    }
                },
                model @ (Model::Resistor(_)
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
                | Model::Inductor(_)
                | Model::Relay(_)
                | Model::OpAmp(_)
                | Model::AnalogTempSensor(_)) => {
                    debug_assert!(!is_active(id));
                    unite_dc_paths(&mut joined, model);
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
            bands,
            joined,
            switches,
            found: Vec::new(),
        }
    }

    fn build_joined(
        &self,
        at: Time,
        netlist: &Netlist,
        actives: &[active::Chip],
        cores: &[active::Core],
    ) -> Vec<usize> {
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
            for (n, (out, &drive)) in chip.outputs.iter().zip(core.drive()).enumerate() {
                if out.pull_down.is_some() && core.pulls_down(n) {
                    unite(&mut joined, out.pin.index(), out.gnd.index());
                }
                let rail = match drive {
                    Drive::High | Drive::PullUp => out.vcc,
                    Drive::Low => out.gnd,
                    Drive::Off => continue,
                };
                unite(&mut joined, out.pin.index(), rail.index());
            }
        }
        joined
    }

    /// Checks the accepted step at `at`, solved with the drives `cores` hold. `read(k, output)`
    /// is when active chip `k`'s firmware first read that output's pin since its drive last
    /// turned off, from a copy of its core that has run at least to `at`. While the drive is
    /// off at `at`, that stretch holds `at`, so a read at or before `at` was taken with the pin
    /// floating as it is now: a read under the pull-up, or before a pull-up came and went,
    /// does not count. A candidate warns once, at the first step where its drive is off and it
    /// does not pull down, its firmware has read it in that stretch by then, and its net reaches
    /// ground through neither a closed switch nor another output's drive or pull-down.
    ///
    /// Also checks each powered `max7219`'s driven logic inputs (`DIN`, `CLK`, `LOAD`): if a
    /// driven input stays strictly inside `(VIL, VIH)` for at least
    /// [`max7219::model::INDETERMINATE_HOLD_PS`], warns `chip.protocol` rule `input_level` once.
    /// High-Z undriven pins (such as MCU GPIOs during power-on boot before `setup()` sets
    /// `pinMode(OUTPUT)`) have no DC path in `joined` and do not trigger this warning.
    pub(crate) fn sample(
        &mut self,
        at: Time,
        row: &[Finite],
        netlist: &Netlist,
        actives: &[active::Chip],
        cores: &[active::Core],
        read: impl Fn(usize, usize) -> Option<Time>,
    ) {
        let mut joined: Option<Vec<usize>> = None;
        let mut k = 0;
        while k < self.candidates.len() {
            let candidate = &self.candidates[k];
            let core = &cores[candidate.active];
            let pulled_down = actives[candidate.active].outputs[candidate.output]
                .pull_down
                .is_some()
                && core.pulls_down(candidate.output);
            let off = core.drive()[candidate.output] == Drive::Off && !pulled_down;
            let read_by_now =
                read(candidate.active, candidate.output).is_some_and(|first| first <= at);
            if !off || !read_by_now {
                k += 1;
                continue;
            }
            let joined =
                joined.get_or_insert_with(|| self.build_joined(at, netlist, actives, cores));
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
        for idx in 0..self.bands.len() {
            if self.bands[idx].warned {
                continue;
            }
            let v_gnd = row[self.bands[idx].gnd].get();
            let v_vcc = row[self.bands[idx].vcc].get() - v_gnd;
            if v_vcc < max7219::model::RETENTION_VCC_V - BAND_MARGIN_V {
                self.bands[idx].since = [None; 3];
                continue;
            }
            let lo = self.bands[idx].vil + BAND_MARGIN_V;
            let hi = self.bands[idx].vih - BAND_MARGIN_V;
            if lo >= hi {
                self.bands[idx].since = [None; 3];
                continue;
            }
            for pin_idx in 0..3 {
                let (net, col) = self.bands[idx].pins[pin_idx];
                let v = row[col].get() - v_gnd;
                if v > lo && v < hi {
                    let joined = joined
                        .get_or_insert_with(|| self.build_joined(at, netlist, actives, cores));
                    if find(joined, net.index()) == find(joined, 0) {
                        let since = *self.bands[idx].since[pin_idx].get_or_insert(at);
                        if at.as_ps().saturating_sub(since.as_ps())
                            >= max7219::model::INDETERMINATE_HOLD_PS
                        {
                            self.bands[idx].warned = true;
                            self.found.push(Diagnostic::ChipProtocol {
                                part: self.bands[idx].part.clone(),
                                at,
                                rule: ProtocolRule::InputLevel,
                            });
                            break;
                        }
                    } else {
                        self.bands[idx].since[pin_idx] = None;
                    }
                } else {
                    self.bands[idx].since[pin_idx] = None;
                }
            }
        }
    }

    /// One `chip.floating_input` per floating pin and one `chip.protocol` (`input_level`) per
    /// `max7219` with a driven indeterminate input, through the last sample.
    pub(crate) fn findings(&self) -> &[Diagnostic] {
        &self.found
    }
}
