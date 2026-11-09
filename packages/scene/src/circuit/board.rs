//! A catalog board as the circuit reader sees it.

use crate::diagnostic::Findings;
use crate::files::Files;
use crate::model::ParamKind;
use crate::params::{self, Arg};
use crate::project::{self, BLOCK, Owner};
use crate::summary::card;
use crate::syntax::Node;

use super::{Circuit, ROOT};

/// One pin the reader leaves unsimulated, with the reason a wire to it gets.
pub struct Gap {
    pub pin: String,
    pub why: String,
}

/// A board pin fed by a regulator output.
pub struct Supply {
    pub pin: String,
    pub from: String,
    pub volts: f64,
}

/// The LED a digital pin drives through one resistor.
pub struct BoardLed {
    pub name: String,
    pub resistor: String,
    pub ohms: f64,
    /// The board pin on the far side of `resistor`.
    pub pin: String,
}

/// The chip, the pins, and the L LED of one catalog board.
pub struct BoardFacts {
    pub id: String,
    pub title: String,
    pub chip: String,
    pub clock_hz: f64,
    pub works: Vec<String>,
    pub gaps: Vec<Gap>,
    /// Working pins that share a net, each group in pin-file order.
    pub same: Vec<Vec<String>>,
    pub supplies: Vec<Supply>,
    pub leds: Vec<BoardLed>,
}

/// The facts of catalog board `id`, from the same read a scene of that board gets.
pub fn board_facts(id: &str) -> BoardFacts {
    let name = id.replace('-', "_");
    let block = format!(
        r#"{{ "format": 1, "block": "{id}", "version": "0.0.0", "title": "{id}", "board": "{id}", "license": "MIT" }}"#
    );
    let scene = format!(r#"{{ "format": 1, "parts": {{ "{name}": {{ "type": "{id}" }} }} }}"#);
    let files = Files::memory([
        (BLOCK, block.into_bytes()),
        ("scene.jsonc", scene.into_bytes()),
    ]);
    let mut findings = Findings::default();
    let project = project::load(&files, std::path::Path::new("scene.jsonc"), &mut findings)
        .unwrap_or_else(|_| panic!("{id} did not load"));
    let mut out = Findings::default();
    let mut circuit = Circuit::new(&project, &mut out);
    circuit.instantiate(ROOT, Owner::Scene);
    circuit.make_pins();
    circuit.join_inside();
    circuit.plug();
    circuit.check_scene_wires(&project.scene);
    circuit.facts()
}

impl Circuit<'_> {
    fn facts(&mut self) -> BoardFacts {
        let mut root_of = vec![0; self.pins.len()];
        for (key, root) in root_of.iter_mut().enumerate() {
            *root = self.find(key);
        }
        let board = self.insts[ROOT]
            .children
            .values()
            .copied()
            .next()
            .expect("the scene names the board");
        let id = self.part(board).expect("the board resolved").label.clone();
        let title = card(&id).expect("the board is a catalog part").title;
        let (chip, clock_hz) = self.chip(board);
        let pins = self.board_pins(board, &root_of);
        let (works, gaps, same) = split_pins(&pins);
        BoardFacts {
            id,
            title,
            chip,
            clock_hz,
            works,
            gaps,
            same,
            supplies: self.supplies(&pins, &root_of),
            leds: self.leds(board, &root_of),
        }
    }

    fn chip(&self, board: usize) -> (String, f64) {
        for &child in self.insts[board].children.values() {
            if self.insts[child].dropped.is_some() {
                continue;
            }
            let Some(part) = self.part(child) else {
                continue;
            };
            let Some(hz) = part
                .model
                .as_ref()
                .and_then(|model| model.get("clock_hz"))
                .and_then(|clock| clock.get("value"))
                .and_then(Node::number)
            else {
                continue;
            };
            let listed = card(&part.label).expect("the chip is a catalog part");
            let name = listed.real.number.unwrap_or(listed.title);
            return (name, hz);
        }
        panic!("the board's chip names no clock")
    }

    fn board_pins(&self, board: usize, root_of: &[usize]) -> Vec<PinRow> {
        self.part(board)
            .expect("the board resolved")
            .pins
            .iter()
            .map(|pin| {
                let key = self.keys[&(board, pin.name.clone())];
                PinRow {
                    name: pin.name.clone(),
                    root: root_of[key],
                    why: self.unsimulated.get(&key).cloned(),
                }
            })
            .collect()
    }

    fn supplies(&self, pins: &[PinRow], root_of: &[usize]) -> Vec<Supply> {
        pins.iter()
            .filter(|pin| pin.why.is_none())
            .filter_map(|pin| {
                let (from, volts) = self.regulator_out(pin.root, root_of)?;
                Some(Supply {
                    pin: pin.name.clone(),
                    from,
                    volts,
                })
            })
            .collect()
    }

    /// The child whose `OUT` shares `root`, when that child is a linear regulator.
    fn regulator_out(&self, root: usize, root_of: &[usize]) -> Option<(String, f64)> {
        for (key, info) in self.pins.iter().enumerate() {
            if root_of[key] != root || info.name != "OUT" {
                continue;
            }
            if self.insts[info.inst].dropped.is_some() {
                continue;
            }
            let part = self.part(info.inst)?;
            if kind(part) != Some("linear_regulator") {
                continue;
            }
            let volts = part
                .model
                .as_ref()?
                .get("output_voltage")?
                .get("value")?
                .number()?;
            let name = self.insts[info.inst].path.last()?.clone();
            return Some((name, volts));
        }
        None
    }

    /// Each simulated LED whose anode reaches a `D` pin through one resistor. The resistor
    /// splits the pin's net from the anode, so the pin is the resistor's other pin.
    fn leds(&self, board: usize, root_of: &[usize]) -> Vec<BoardLed> {
        let mut leds = Vec::new();
        let children: Vec<(String, usize)> = self.insts[board]
            .children
            .iter()
            .map(|(name, id)| (name.clone(), *id))
            .collect();
        for (name, child) in children {
            if self.insts[child].dropped.is_some() {
                continue;
            }
            let Some(part) = self.part(child) else {
                continue;
            };
            if kind(part) != Some("led") {
                continue;
            }
            let Some(&anode) = self.keys.get(&(child, "anode".to_string())) else {
                continue;
            };
            let Some(drive) = self.drive(anode, board, root_of) else {
                continue;
            };
            leds.push(BoardLed {
                name,
                resistor: drive.0,
                ohms: drive.1,
                pin: drive.2,
            });
        }
        leds
    }

    /// The resistor on the anode's net, the ohms it is set to, and the `D` pin on its other side.
    fn drive(
        &self,
        anode: usize,
        board: usize,
        root_of: &[usize],
    ) -> Option<(String, f64, String)> {
        let root = root_of[anode];
        for (key, info) in self.pins.iter().enumerate() {
            if root_of[key] != root || self.insts[info.inst].dropped.is_some() {
                continue;
            }
            let part = self.part(info.inst)?;
            if kind(part) != Some("resistor") {
                continue;
            }
            let other = match info.name.as_str() {
                "a" => "b",
                "b" => "a",
                _ => continue,
            };
            let other_key = self.keys.get(&(info.inst, other.to_string()))?;
            let pin = self.digital_pin(board, root_of[*other_key], root_of)?;
            let ohms = ohms(self, info.inst)?;
            let resistor = self.insts[info.inst].path.last()?.clone();
            return Some((resistor, ohms, pin));
        }
        None
    }

    fn digital_pin(&self, board: usize, root: usize, root_of: &[usize]) -> Option<String> {
        self.part(board)?
            .pins
            .iter()
            .find(|pin| {
                digital(&pin.name)
                    && self
                        .keys
                        .get(&(board, pin.name.clone()))
                        .is_some_and(|key| root_of[*key] == root)
            })
            .map(|pin| pin.name.clone())
    }
}

struct PinRow {
    name: String,
    root: usize,
    why: Option<String>,
}

fn split_pins(pins: &[PinRow]) -> (Vec<String>, Vec<Gap>, Vec<Vec<String>>) {
    let mut works = Vec::new();
    let mut gaps = Vec::new();
    let mut groups: Vec<(usize, Vec<String>)> = Vec::new();
    for pin in pins {
        match &pin.why {
            Some(why) => gaps.push(Gap {
                pin: pin.name.clone(),
                why: why.clone(),
            }),
            None => {
                works.push(pin.name.clone());
                if let Some(group) = groups.iter_mut().find(|(root, _)| *root == pin.root) {
                    group.1.push(pin.name.clone());
                } else {
                    groups.push((pin.root, vec![pin.name.clone()]));
                }
            }
        }
    }
    let same = groups
        .into_iter()
        .filter_map(|(_, names)| (names.len() > 1).then_some(names))
        .collect();
    (works, gaps, same)
}

fn kind(part: &crate::model::Part) -> Option<&str> {
    part.model.as_ref()?.get("kind")?.str()
}

fn digital(name: &str) -> bool {
    name.strip_prefix('D')
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
}

fn ohms(circuit: &Circuit, inst: usize) -> Option<f64> {
    let part = circuit.part(inst)?;
    let decl = part.param("resistance")?;
    let ParamKind::Number { unit, range } = decl.kind else {
        return None;
    };
    let arg: &Arg = circuit.insts[inst].args.get("resistance")?;
    params::number(unit, range, &arg.node)
        .ok()
        .and_then(|value| value.get("value")?.as_f64())
}
