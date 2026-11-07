//! The lint rules: what a scene that checks is likely to have wrong or left unclear. Each rule
//! reads the checked circuit and pushes warnings. None of them blocks.

use std::collections::{BTreeMap, BTreeSet};

use sim_core::diagnostic::Diagnostic;
use sim_core::name::Ref;
use sim_core::netlist::{Model, Netlist};

use super::{Circuit, ROOT};
use crate::diagnostic::{Code, Findings, Lint};
use crate::model::{Instance, Part};
use crate::name::and_list;
use crate::syntax::{self, Node};

const RULES: [fn(&Scope, &mut Findings); 8] = [
    unplaced,
    unconnected,
    ground_unnamed,
    rotation,
    unsimulated_pins,
    no_model,
    floating,
    led_no_resistor,
];

struct Scope<'c, 'p> {
    circuit: &'c Circuit<'p>,
    /// Each pin's net, as its root pin.
    root: Vec<usize>,
    /// Each net's name, by its root pin.
    nets: &'c BTreeMap<usize, String>,
    /// The nets a scene wire or the scene's `ground` reaches, by root pin.
    wired: BTreeSet<usize>,
    /// The scene's own instances, in its order, with their ids and parts.
    tops: Vec<(&'p Instance, usize, &'p Part)>,
    scene: Node,
    engine: Netlist,
    ground: &'c str,
}

impl Scope<'_, '_> {
    fn pin_root(&self, inst: usize, pin: &str) -> usize {
        self.root[self.circuit.keys[&(inst, pin.to_string())]]
    }

    fn net_root(&self, name: &str) -> Option<usize> {
        self.nets
            .iter()
            .find(|(_, net)| *net == name)
            .map(|(root, _)| *root)
    }

    /// Whether `inst` or a part inside it is simulated.
    fn simulates(&self, inst: usize) -> bool {
        let at = &self.circuit.insts[inst];
        at.dropped.is_none()
            && (self.circuit.part(inst).is_some_and(|p| p.model.is_some())
                || at.children.values().any(|&child| self.simulates(child)))
    }

    /// The scene instance an engine part sits in, by the first segment of its flat name.
    fn top_named(&self, flat: &str) -> &Instance {
        let top = flat.split("__").next().unwrap_or(flat);
        self.tops
            .iter()
            .map(|(i, _, _)| *i)
            .find(|i| i.name == top)
            .expect("an engine part sits in a scene part")
    }
}

impl Circuit<'_> {
    pub(super) fn lint(
        &mut self,
        nets: &BTreeMap<usize, String>,
        netlist: &super::Netlist,
        out: &mut Findings,
    ) {
        let root: Vec<usize> = (0..self.pins.len()).map(|k| self.find(k)).collect();
        let project = self.project;
        let scene = &project.scene;
        let reached = scene.comp.wires.iter().flat_map(|w| &w.ends);
        let ends: Vec<_> = reached.chain(&scene.comp.ground).collect();
        let wired = ends
            .into_iter()
            .filter_map(|r| self.resolve(ROOT, r))
            .map(|key| root[key])
            .collect();
        let tops = scene
            .comp
            .parts
            .iter()
            .map(|i| {
                let id = self.insts[ROOT].children[&i.name];
                (
                    i,
                    id,
                    self.part(id).expect("every part resolved when it checked"),
                )
            })
            .collect();
        let tree = syntax::parse(&project.sources, scene.file, &mut Findings::default())
            .expect("the scene parsed when it checked");
        let scope = Scope {
            circuit: &*self,
            root,
            nets,
            wired,
            tops,
            scene: tree,
            engine: sim_core::netlist::parse(netlist.json.as_bytes())
                .expect("the engine read this netlist when it checked"),
            ground: &netlist.ground,
        };
        for rule in RULES {
            rule(&scope, out);
        }
    }
}

fn unplaced(s: &Scope, out: &mut Findings) {
    let layout = s.scene.get("layout");
    for (instance, _, _) in &s.tops {
        let name = &instance.name;
        if layout.and_then(|l| l.get(name)?.get("at")).is_none() {
            out.push(
                Code::Lint(Lint::Unplaced),
                instance.name_span,
                format!(
                    "`{name}` has no position, so the viewer places it for drawing only and writes nothing"
                ),
                format!("place it in the scene's \"layout\", as \"{name}\": {{ \"at\": [x, y, z] }} in mm"),
            );
        }
    }
}

fn unconnected(s: &Scope, out: &mut Findings) {
    for &(instance, id, part) in &s.tops {
        if s.circuit.insts[id].dropped.is_some() {
            continue;
        }
        let pins: Vec<&str> = part
            .pins
            .iter()
            .map(|p| p.name.as_str())
            .filter(|pin| {
                let key = s.circuit.keys[&(id, pin.to_string())];
                !s.circuit.unsimulated.contains_key(&key)
            })
            .collect();
        let open: Vec<&str> = pins
            .iter()
            .copied()
            .filter(|pin| !s.wired.contains(&s.pin_root(id, pin)))
            .collect();
        let Some(first) = open.first() else {
            continue;
        };
        let name = &instance.name;
        let (message, hint) = if open.len() == pins.len() {
            (
                format!("nothing is wired to `{name}`"),
                format!(
                    "wire it, as [\"{name}.{first}\", \"<part>.<pin>\"], or remove it from \"parts\""
                ),
            )
        } else {
            (
                format!("`{name}` has no wire on {}", and_list(&open)),
                format!(
                    "wire each pin the circuit uses, as [\"{name}.{first}\", \"<part>.<pin>\"]; a pin nothing reads can stay open"
                ),
            )
        };
        out.push(
            Code::Lint(Lint::Unconnected),
            instance.name_span,
            message,
            hint,
        );
    }
}

fn ground_unnamed(s: &Scope, out: &mut Findings) {
    let scene = &s.circuit.project.scene;
    if scene.nets.iter().any(|(name, _, _)| name == s.ground) {
        return;
    }
    let root = s.net_root(s.ground).expect("the ground is a net");
    let pin = s
        .tops
        .iter()
        .find_map(|&(instance, id, part)| {
            let pin = part.pins.iter().find(|p| s.pin_root(id, &p.name) == root)?;
            Some(format!("{}.{}", instance.name, pin.name))
        })
        .unwrap_or_else(|| {
            let info = &s.circuit.pins[root];
            format!("{}.{}", s.circuit.insts[info.inst].dotted(), info.name)
        });
    out.push(
        Code::Lint(Lint::GroundUnnamed),
        scene.span,
        format!(
            "the ground net is `{}`, a name taken from a pin, because \"nets\" does not name it",
            s.ground
        ),
        format!("add \"GND\": \"{pin}\" to the scene's \"nets\", so refs and messages say GND"),
    );
}

/// An angle as one turn writes it, above -180 and at most 180 degrees.
fn canonical(angle: f64) -> f64 {
    let turned = angle.rem_euclid(360.0);
    if turned > 180.0 {
        turned - 360.0
    } else {
        turned
    }
}

fn rotation(s: &Scope, out: &mut Findings) {
    let Some(layout) = s.scene.get("layout").and_then(Node::props) else {
        return;
    };
    let shown = |angles: &[f64]| {
        let each: Vec<String> = angles.iter().map(f64::to_string).collect();
        format!("[{}]", each.join(", "))
    };
    for placement in layout {
        let Some(rotate) = placement.value.get("rotate") else {
            continue;
        };
        let Some(angles) = rotate
            .items()
            .and_then(|items| items.iter().map(Node::number).collect::<Option<Vec<f64>>>())
        else {
            continue;
        };
        if angles.iter().all(|a| *a > -180.0 && *a <= 180.0) {
            continue;
        }
        let turned: Vec<f64> = angles.iter().copied().map(canonical).collect();
        out.push(
            Code::Lint(Lint::Rotation),
            rotate.span,
            format!(
                "`{}` turns by {}, which is {} with each angle above -180 and at most 180 degrees",
                placement.key,
                shown(&angles),
                shown(&turned)
            ),
            format!("write \"rotate\": {}", shown(&turned)),
        );
    }
}

fn unsimulated_pins(s: &Scope, out: &mut Findings) {
    let mut seen = BTreeSet::new();
    for &(instance, id, part) in &s.tops {
        if s.circuit.insts[id].dropped.is_some() || !seen.insert(&part.label) {
            continue;
        }
        let mut reasons: Vec<(&str, Vec<&str>)> = Vec::new();
        for pin in &part.pins {
            let key = s.circuit.keys[&(id, pin.name.clone())];
            let Some(why) = s.circuit.unsimulated.get(&key) else {
                continue;
            };
            match reasons.iter_mut().find(|(w, _)| w == why) {
                Some((_, pins)) => pins.push(&pin.name),
                None => reasons.push((why, vec![&pin.name])),
            }
        }
        if reasons.is_empty() {
            continue;
        }
        let all: Vec<&str> = reasons
            .iter()
            .flat_map(|(_, p)| p.iter().copied())
            .collect();
        let why: Vec<String> = reasons
            .iter()
            .map(|(why, pins)| format!("{}: {why}", pins.join(", ")))
            .collect();
        out.push(
            Code::Lint(Lint::UnsimulatedPins),
            instance.name_span,
            format!(
                "{} does not simulate {}. {}",
                part.label,
                and_list(&all),
                why.join(" ")
            ),
            format!(
                "leave these pins of `{}` unwired; a wire to one is a `not_simulated` error",
                instance.name
            ),
        );
    }
}

fn no_model(s: &Scope, out: &mut Findings) {
    for &(instance, id, part) in &s.tops {
        if s.circuit.insts[id].dropped.is_some()
            || !part.not_simulated.is_empty()
            || s.simulates(id)
        {
            continue;
        }
        let name = &instance.name;
        out.push(
            Code::Lint(Lint::NoModel),
            instance.name_span,
            format!(
                "`{name}` is a {}, which has no engine model and no `not_simulated` reason, so the simulation runs nothing for it and its pins only join the wires that reach them",
                part.label
            ),
            format!(
                "use a part with a model if `{name}` should act in the circuit, or add \"not_simulated\": \"<why>\" to `{name}` to keep it for drawing only"
            ),
        );
    }
}

fn floating(s: &Scope, out: &mut Findings) {
    for found in sim_circuit::floating_nets(&s.engine) {
        let Diagnostic::NetFloating { net: Ref::Net(net) } = &found else {
            continue;
        };
        let net = net.as_str();
        let root = s.net_root(net).expect("the reader named every net");
        let span = s.circuit.insts[s.circuit.pins[root].inst].top_span;
        let value = serde_json::to_value(&found).unwrap_or_default();
        let pin = net.replace("__", ".");
        out.push(
            Code::Lint(Lint::Floating),
            span,
            value["message"].as_str().unwrap_or_default().to_string(),
            format!(
                "if a part reads `{pin}`, wire it to a source, or through a resistor to ground; a net nothing reads can stay as it is"
            ),
        );
    }
}

/// A net an LED's current can come from or go to: ground, or one a source, a regulator or a
/// chip pin holds.
fn holds(model: &Model) -> bool {
    matches!(
        model,
        Model::VoltageSource(_) | Model::LinearRegulator(_) | Model::Chip(_)
    )
}

/// A part that carries an LED's current on without limiting it.
fn passes(model: &Model) -> bool {
    matches!(
        model,
        Model::Led(_) | Model::Diode(_) | Model::Zener(_) | Model::Switch(_)
    )
}

fn limits(model: &Model) -> bool {
    matches!(model, Model::Resistor(_) | Model::Potentiometer(_))
}

/// An LED whose path, through other junctions and switches, joins two held nets with no
/// resistor in series. A resistor with both ends on that path is in parallel, so it limits
/// nothing.
fn led_no_resistor(s: &Scope, out: &mut Findings) {
    let engine = &s.engine;
    let mut held = vec![false; engine.nets().len()];
    let mut names = vec![""; engine.nets().len()];
    for (id, name) in engine.nets() {
        held[id.index()] = id.is_ground();
        names[id.index()] = name.as_str();
    }
    for (_, part) in engine.parts() {
        if holds(&part.model) {
            for net in part.model.pins() {
                held[net.index()] = true;
            }
        }
    }
    for (_, led) in engine.parts() {
        if !matches!(led.model, Model::Led(_)) {
            continue;
        }
        let mut path: BTreeSet<usize> = led.model.pins().iter().map(|n| n.index()).collect();
        let mut next: Vec<usize> = path.iter().copied().collect();
        while let Some(net) = next.pop() {
            if held[net] {
                continue;
            }
            for (_, part) in engine.parts() {
                let pins = part.model.pins();
                if passes(&part.model) && pins.iter().any(|n| n.index() == net) {
                    for other in pins {
                        if path.insert(other.index()) {
                            next.push(other.index());
                        }
                    }
                }
            }
        }
        let limited = engine.parts().any(|(_, part)| {
            let pins = part.model.pins();
            limits(&part.model)
                && pins
                    .iter()
                    .any(|n| path.contains(&n.index()) && !held[n.index()])
                && pins.iter().any(|n| !path.contains(&n.index()))
        });
        let ends: Vec<String> = path
            .iter()
            .filter(|&&net| held[net])
            .map(|&net| format!("`{}`", names[net]))
            .collect();
        if limited || ends.len() < 2 {
            continue;
        }
        let name = led.name.as_str();
        let top = s.top_named(name);
        let shown = name.replace("__", ".");
        out.push(
            Code::Lint(Lint::LedNoResistor),
            top.name_span,
            format!(
                "nothing limits the current through `{shown}`: it joins {} with no resistor in series, so it can draw far over its rating",
                and_list(&ends.iter().map(String::as_str).collect::<Vec<_>>())
            ),
            format!(
                "put a resistor in series with `{shown}`, such as 220 ohm between a 5 V pin and the LED"
            ),
        );
    }
}
