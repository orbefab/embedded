//! A catalog board as the circuit reader sees it. The parts come from a `parts/` directory
//! the caller names. Nothing here is built into the binary.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::diagnostic::Findings;
use crate::files::Files;
use crate::model::ParamKind;
use crate::params::{self, Arg};
use crate::project::{self, BLOCK};
use crate::source::{FileId, Sources};
use crate::syntax::{self, Node};

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

/// Files of a Block whose scene holds one catalog part `id`. The scene file is `scene.jsonc`.
/// `parts` is the directory of part folders. The Block reads `id` and the parts it uses.
pub fn one_part(parts: &Path, id: &str) -> Files {
    let name = id.replace('-', "_");
    let block = format!(
        r#"{{ "format": 1, "block": "{id}", "version": "0.0.0", "title": "{id}", "board": "{id}", "license": "MIT" }}"#
    );
    let scene = format!(r#"{{ "format": 1, "parts": {{ "{name}": {{ "type": "{id}" }} }} }}"#);
    Files::memory([
        (BLOCK, block.into_bytes()),
        ("scene.jsonc", scene.into_bytes()),
    ])
    .with_catalog(catalog_of(parts, id))
}

/// The part `id` and every part its `uses` names, flat, as a Block's catalog.
fn catalog_of(parts: &Path, root_id: &str) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut pending = vec![root_id.to_string()];
    let mut seen = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let dir = parts.join(&id);
        let Ok(paths) = crate::published::published_paths(&dir) else {
            continue;
        };
        let mut files = BTreeMap::new();
        for path in paths {
            let Ok(rel) = path.strip_prefix(&dir) else {
                continue;
            };
            let rel = rel
                .components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if rel.is_empty() {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            files.insert(rel, bytes);
        }
        if let Some(text) = files
            .get("part.jsonc")
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
        {
            pending.extend(use_ids(text));
        }
        for (rel, bytes) in files {
            out.push((format!("{id}/{rel}"), bytes));
        }
    }
    out
}

fn use_ids(text: &str) -> Vec<String> {
    let mut sources = Sources::new(Path::new(""));
    let file = sources.add(Path::new("part.jsonc"), text.to_string());
    let mut findings = Findings::default();
    let Some(tree) = syntax::parse(&sources, file, &mut findings) else {
        return Vec::new();
    };
    tree.get("uses")
        .and_then(Node::props)
        .into_iter()
        .flatten()
        .map(|prop| prop.key.clone())
        .collect()
}

/// The facts of catalog board `id` in `parts`, from the same read a scene of that board gets.
/// The error is a board that does not read, or one that has no direct `atmega328p` child
/// with a clock.
pub fn board_facts(parts: &Path, id: &str) -> Result<BoardFacts, String> {
    let files = one_part(parts, id);
    let mut findings = Findings::default();
    let project = match project::load(&files, Path::new("scene.jsonc"), &mut findings) {
        Ok(project) if findings.is_empty() => project,
        Ok(project) => return Err(named(id, reported(findings, &project.sources))),
        Err(sources) => return Err(named(id, reported(findings, &sources))),
    };
    let mut out = Findings::default();
    let mut circuit = super::read_far(&project, &mut out);
    circuit.facts()
}

fn part_string(project: &project::Project, file: FileId, path: &[&str]) -> Option<String> {
    let mut findings = Findings::default();
    let tree = syntax::parse(&project.sources, file, &mut findings)?;
    let mut node = &tree;
    for key in path {
        node = node.get(key)?;
    }
    let text = node.str()?;
    (!text.trim().is_empty()).then(|| text.to_string())
}

fn named(id: &str, error: String) -> String {
    if error.contains(id) {
        error
    } else {
        format!("`{id}`: {error}")
    }
}

fn reported(findings: Findings, sources: &crate::source::Sources) -> String {
    let text = findings
        .into_diagnostics(sources)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        "the board did not read".to_string()
    } else {
        text
    }
}

impl Circuit<'_> {
    fn facts(&mut self) -> Result<BoardFacts, String> {
        let mut root_of = vec![0; self.pins.len()];
        for (key, root) in root_of.iter_mut().enumerate() {
            *root = self.find(key);
        }
        let Some(board) = self.insts[ROOT].children.values().copied().next() else {
            return Err("the scene names no board".into());
        };
        let Some(part) = self.part(board) else {
            return Err("the board did not resolve".into());
        };
        let id = part.label.clone();
        let title = part_string(self.project, part.file, &["title"]).unwrap_or_else(|| id.clone());
        let (chip, clock_hz) = self.chip(board, &id)?;
        let pins = self.board_pins(board, &root_of);
        let (works, gaps, same) = split_pins(&pins);
        Ok(BoardFacts {
            id,
            title,
            chip,
            clock_hz,
            works,
            gaps,
            same,
            supplies: self.supplies(&pins, &root_of),
            leds: self.leds(board, &root_of),
        })
    }

    /// The direct `atmega328p` child that names a clock, and that clock.
    fn chip(&self, board: usize, id: &str) -> Result<(String, f64), String> {
        for &child in self.insts[board].children.values() {
            if self.insts[child].dropped.is_some() {
                continue;
            }
            let Some(part) = self.part(child) else {
                continue;
            };
            if !part.label.starts_with("atmega328p") {
                continue;
            }
            let Some(hz) = part
                .model
                .as_ref()
                .and_then(|model| model.get("clock_hz"))
                .and_then(|clock| clock.get("value"))
                .and_then(Node::number)
            else {
                continue;
            };
            let name = part_string(self.project, part.file, &["real", "number"])
                .or_else(|| part_string(self.project, part.file, &["title"]))
                .unwrap_or_else(|| part.label.clone());
            return Ok((name, hz));
        }
        Err(format!(
            "`{id}` is not an AVR board with a direct atmega328p child and a clock"
        ))
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
            let Some(part) = self.part(info.inst) else {
                continue;
            };
            if kind(part) != Some("resistor") {
                continue;
            }
            let other = match info.name.as_str() {
                "a" => "b",
                "b" => "a",
                _ => continue,
            };
            let Some(&other_key) = self.keys.get(&(info.inst, other.to_string())) else {
                continue;
            };
            let Some(pin) = self.digital_pin(board, root_of[other_key], root_of) else {
                continue;
            };
            let Some(ohms) = ohms(self, info.inst) else {
                continue;
            };
            let Some(resistor) = self.insts[info.inst].path.last() else {
                continue;
            };
            return Some((resistor.clone(), ohms, pin));
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn an_earlier_resistor_that_reaches_no_digital_pin_does_not_drop_the_led() {
        let files = Files::memory([
            (BLOCK, BLOCK_JSON.as_bytes().to_vec()),
            ("scene.jsonc", SCENE_JSON.as_bytes().to_vec()),
        ])
        .with_catalog([
            (
                "resistor-0603/part.jsonc".into(),
                RESISTOR.as_bytes().to_vec(),
            ),
            ("led-0603/part.jsonc".into(), LED.as_bytes().to_vec()),
            ("two-r-board/part.jsonc".into(), BOARD.as_bytes().to_vec()),
        ]);
        let mut findings = Findings::default();
        let project = match project::load(&files, Path::new("scene.jsonc"), &mut findings) {
            Ok(project) if findings.is_empty() => project,
            Ok(project) => panic!("{}", reported(findings, &project.sources)),
            Err(sources) => panic!("{}", reported(findings, &sources)),
        };
        let mut out = Findings::default();
        let mut circuit = super::super::read_far(&project, &mut out);
        let board = circuit.insts[super::super::ROOT]
            .children
            .values()
            .copied()
            .next()
            .unwrap();
        let mut root_of = vec![0; circuit.pins.len()];
        for (key, root) in root_of.iter_mut().enumerate() {
            *root = circuit.find(key);
        }
        let leds = circuit.leds(board, &root_of);
        assert_eq!(
            leds.len(),
            1,
            "resistors {}",
            leds.iter()
                .map(|led| led.resistor.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert_eq!(leds[0].name, "led_l");
        assert_eq!(leds[0].resistor, "r_l");
        assert_eq!(leds[0].pin, "D13");
        assert_eq!(leds[0].ohms, 1000.0);
    }

    const BLOCK_JSON: &str = r#"{ "format": 1, "block": "two-r-board", "version": "0.0.0", "title": "two-r-board", "license": "MIT" }"#;
    const SCENE_JSON: &str =
        r#"{ "format": 1, "parts": { "two_r_board": { "type": "two-r-board" } } }"#;
    const RESISTOR: &str = r#"{
  "format": 1, "part": "resistor-0603", "version": "0.1.0", "title": "R", "license": "MIT",
  "pins": { "a": [1], "b": [2] },
  "params": { "resistance": { "unit": "ohm", "required": true, "min_exclusive": 0, "sets": "/resistance" } },
  "model": { "kind": "resistor" }
}"#;
    const LED: &str = r#"{
  "format": 1, "part": "led-0603", "version": "0.1.0", "title": "L", "license": "MIT",
  "pins": { "anode": [1], "cathode": [2] },
  "model": { "kind": "led" }
}"#;
    const BOARD: &str = r#"{
  "format": 1, "part": "two-r-board", "version": "0.1.0", "title": "Two resistors", "license": "MIT",
  "parts": {
    "r_a": { "type": "resistor-0603", "resistance": "1k" },
    "r_l": { "type": "resistor-0603", "resistance": "1k" },
    "led_l": { "type": "led-0603" }
  },
  "wires": [
    ["r_a.a", "led_l.anode"],
    ["r_l.a", "led_l.anode"],
    ["r_l.b", "led_l.cathode"]
  ],
  "pins": { "D13": "r_l.b" }
}"#;
}
