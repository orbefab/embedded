//! The lint rules: what a scene that checks is likely to have wrong or left unclear. Each rule
//! reads the checked circuit and pushes warnings. None of them blocks.

use orbe_engine::netlist::{Conduction, Model, Netlist};

use std::collections::{BTreeMap, BTreeSet};

use orbe_engine::diagnostic::Diagnostic;
use orbe_engine::name::Ref;

use super::{Circuit, ROOT};
use crate::diagnostic::{Code, Findings, Lint};
use crate::model::{Instance, Part, TypeRef};
use crate::name::and_list;
use crate::syntax::{self, Node};

const RULES: [fn(&Scope, &mut Findings); 8] = [
    unplaced,
    shorted,
    unconnected,
    ground_unnamed,
    rotation,
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

    /// A board holds its own supply and a chip, so it runs with nothing wired to it.
    fn runs_alone(&self, instance: &Instance) -> bool {
        let holds = |kind: fn(&Model) -> bool| {
            self.engine.parts().any(|(_, p)| {
                kind(&p.model) && self.top_named(p.name.as_str()).name == instance.name
            })
        };
        holds(|m| matches!(m, Model::VoltageSource(_))) && holds(|m| matches!(m, Model::Chip(_)))
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
        let contacts: Vec<usize> = self
            .seated
            .values()
            .flat_map(|s| &s.contacts)
            .flat_map(|c| [c.pin_key, c.hole_key])
            .collect();
        let wired = ends
            .into_iter()
            .filter_map(|r| self.resolve(ROOT, r))
            .chain(contacts)
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
            engine: orbe_engine::netlist::parse(netlist.json.as_bytes())
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
    let unplaced: Vec<&Instance> = s
        .tops
        .iter()
        .filter(|(_, id, _)| !s.circuit.seated.contains_key(id))
        .map(|(instance, _, _)| *instance)
        .filter(|i| layout.and_then(|l| l.get(&i.name)?.get("at")).is_none())
        .collect();
    let Some(first) = unplaced.first() else {
        return;
    };
    let names: Vec<String> = unplaced.iter().map(|i| format!("`{}`", i.name)).collect();
    let names = and_list(&names.iter().map(String::as_str).collect::<Vec<_>>());
    let message = if unplaced.len() == 1 {
        format!("{names} has no position, so the 3D page places it in a row for drawing only")
    } else {
        format!("{names} have no position, so the 3D page places them in a row for drawing only")
    };
    out.push(
        Code::Lint(Lint::Unplaced),
        first.name_span,
        message,
        format!(
            "place a part whose place matters in the scene's \"layout\", as \"{}\": {{ \"at\": [x, y, z] }} in mm",
            first.name
        ),
    );
}

/// Two pins of one plugged part in one strip: the strip joins them, as a part plugged in along
/// a column instead of across it.
fn shorted(s: &Scope, out: &mut Findings) {
    for &(instance, id, _) in &s.tops {
        let Some(seated) = s.circuit.seated.get(&id) else {
            continue;
        };
        let mut strips: BTreeMap<(usize, usize), Vec<&super::plug::Contact>> = BTreeMap::new();
        for contact in &seated.contacts {
            if let Some(strip) = contact.strip {
                strips
                    .entry((contact.board, strip))
                    .or_default()
                    .push(contact);
            }
        }
        for ((board, strip), contacts) in strips {
            let mut pins: Vec<&str> = contacts.iter().map(|c| c.shown.as_str()).collect();
            pins.dedup();
            if pins.len() < 2 {
                continue;
            }
            let board = s.circuit.insts[board].dotted();
            let holes = s
                .circuit
                .part(s.circuit.insts[ROOT].children[&board])
                .and_then(|p| p.holes.as_ref())
                .map(|h| &h.strips[strip]);
            let span = holes.map_or_else(
                || board.clone(),
                |h| crate::holes::strip_text(h, &format!("{board}.")),
            );
            let shown: Vec<String> = pins.iter().map(|p| format!("`{p}`")).collect();
            out.push(
                Code::Lint(Lint::Shorted),
                instance.name_span,
                format!(
                    "{} sit in one strip, {span}, so the strip shorts them",
                    and_list(&shown.iter().map(String::as_str).collect::<Vec<_>>())
                ),
                seated.parted.clone().unwrap_or_default(),
            );
        }
    }
}

/// A part with at most this many nets, such as a resistor, an LED or a transistor, uses each of
/// them, so an open one is likely a mistake. A board leaves most of its pins open.
const FEW_NETS: usize = 3;

fn unconnected(s: &Scope, out: &mut Findings) {
    for &(instance, id, part) in &s.tops {
        if s.circuit.insts[id].dropped.is_some() {
            continue;
        }
        let mut nets: Vec<(usize, &str)> = Vec::new();
        for pin in &part.pins {
            let key = s.circuit.keys[&(id, pin.name.clone())];
            let root = s.root[key];
            if !s.circuit.unsimulated.contains_key(&key) && nets.iter().all(|(r, _)| *r != root) {
                nets.push((root, &pin.name));
            }
        }
        let open: Vec<&str> = nets
            .iter()
            .filter(|(root, _)| !s.wired.contains(root))
            .map(|(_, pin)| *pin)
            .collect();
        let Some(first) = open.first() else {
            continue;
        };
        let name = &instance.name;
        let (message, hint) = if open.len() == nets.len() {
            if s.runs_alone(instance) {
                continue;
            }
            (
                format!("nothing is wired to `{name}`"),
                format!(
                    "wire it, as [\"{name}.{first}\", \"<part>.<pin>\"], or remove it from \"parts\""
                ),
            )
        } else if nets.len() <= FEW_NETS {
            (
                format!("`{name}` has no wire on {}", and_list(&open)),
                format!(
                    "wire it, as [\"{name}.{first}\", \"<part>.<pin>\"], or leave it open if the circuit means it, as a pot's third pin when the pot is a variable resistor"
                ),
            )
        } else {
            continue;
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
        .filter(|(_, _, part)| part.holes.is_none())
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

fn no_model(s: &Scope, out: &mut Findings) {
    for &(instance, id, part) in &s.tops {
        if s.circuit.insts[id].dropped.is_some()
            || part.joins_only
            || !part.not_simulated.is_empty()
            || s.simulates(id)
        {
            continue;
        }
        let name = &instance.name;
        let wired = part
            .pins
            .iter()
            .any(|pin| s.wired.contains(&s.pin_root(id, &pin.name)));
        let hint = match (wired, &instance.ty) {
            (false, _) => format!(
                "use a part with a model if `{name}` should act in the circuit, or add \"not_simulated\": \"<why>\" to `{name}` to keep it for drawing only"
            ),
            (true, Some((TypeRef::Path(path), _))) => format!(
                "use a part with a model if `{name}` should act in the circuit, or add \"joins_only\": true to its part file, {path}, if it only joins wires"
            ),
            (true, _) => format!("use a part with a model if `{name}` should act in the circuit"),
        };
        out.push(
            Code::Lint(Lint::NoModel),
            instance.name_span,
            format!(
                "`{name}` is a {}, which has no engine model and no `not_simulated` reason, so the simulation runs nothing for it and its pins only join the wires that reach them",
                part.label
            ),
            hint,
        );
    }
}

fn floating(s: &Scope, out: &mut Findings) {
    for found in orbe_engine::floating_nets(&s.engine) {
        let Diagnostic::NetFloating { net: Ref::Net(net) } = &found else {
            continue;
        };
        let net = net.as_str();
        let root = s.net_root(net).expect("the reader named every net");
        let first = (0..s.circuit.pins.len())
            .find(|&k| s.root[k] == root && !s.circuit.pins[k].hole)
            .unwrap_or(root);
        let span = s.circuit.insts[s.circuit.pins[first].inst].top_span;
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

/// An LED whose path, through other junctions, switches and transistors switched to a held
/// net, joins two driven nets with no resistor in series. A diode's cathode is driven when its
/// anode is a source's net. A resistor with both ends on that
/// path is in parallel, so it limits nothing. A chip pin drives its net unless a resistor feeds
/// that net from a held net off the path, since then the pin more likely reads the net.
fn led_no_resistor(s: &Scope, out: &mut Findings) {
    let engine = &s.engine;
    let mut fixed = vec![false; engine.nets().len()];
    let mut chip = vec![false; engine.nets().len()];
    let mut names = vec![""; engine.nets().len()];
    for (id, name) in engine.nets() {
        fixed[id.index()] = id.is_ground();
        names[id.index()] = name.as_str();
    }
    for (_, part) in engine.parts() {
        let marks = match part.model {
            Model::Chip(_) => &mut chip,
            ref model if model.conduction() == Conduction::Fixes => &mut fixed,
            _ => continue,
        };
        for net in part.model.pins() {
            marks[net.index()] = true;
        }
    }
    // A diode from a source's net holds its cathode about a drop under that source, as the
    // Nano's Schottky holds its 5V rail under USB. Ground holds nothing this way, since a diode
    // from ground is a clamp.
    let mut more = true;
    while more {
        more = false;
        for (_, part) in engine.parts() {
            if !matches!(part.model, Model::Diode(_)) {
                continue;
            }
            let [anode, cathode] = part.model.pins()[..] else {
                continue;
            };
            if fixed[anode.index()] && !anode.is_ground() && !fixed[cathode.index()] {
                fixed[cathode.index()] = true;
                more = true;
            }
        }
    }
    let held = |net: usize| fixed[net] || chip[net];
    for (_, led) in engine.parts() {
        if !matches!(led.model, Model::Led(_)) {
            continue;
        }
        let mut path: BTreeSet<usize> = led.model.pins().iter().map(|n| n.index()).collect();
        let mut next: Vec<usize> = path.iter().copied().collect();
        while let Some(net) = next.pop() {
            if held(net) {
                continue;
            }
            for (_, part) in engine.parts() {
                let pins = part.model.pins();
                let reached: Vec<usize> = match part.model.conduction() {
                    Conduction::Passes if pins.iter().any(|n| n.index() == net) => {
                        pins.iter().map(|n| n.index()).collect()
                    }
                    Conduction::Switches(from, to)
                        if pins[from].index() == net && held(pins[to].index()) =>
                    {
                        vec![pins[to].index()]
                    }
                    _ => Vec::new(),
                };
                for other in reached {
                    if path.insert(other) {
                        next.push(other);
                    }
                }
            }
        }
        let fed = |net: usize| {
            engine.parts().any(|(_, part)| {
                let pins = part.model.pins();
                part.model.conduction() == Conduction::Limits
                    && pins.iter().any(|n| n.index() == net)
                    && pins
                        .iter()
                        .any(|n| !path.contains(&n.index()) && held(n.index()))
            })
        };
        let limited = engine.parts().any(|(_, part)| {
            let pins = part.model.pins();
            part.model.conduction() == Conduction::Limits
                && pins
                    .iter()
                    .any(|n| path.contains(&n.index()) && !held(n.index()))
                && pins.iter().any(|n| !path.contains(&n.index()))
        });
        let ends: Vec<String> = path
            .iter()
            .filter(|&&net| fixed[net] || (chip[net] && !fed(net)))
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
