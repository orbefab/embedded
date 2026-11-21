//! The lint rules: what a scene that checks is likely to have wrong or left unclear. Each rule
//! reads the checked circuit and pushes warnings. None of them blocks.

use orbe_engine::netlist::{Chip, Conduction, Model, NetId, Netlist};
use orbe_engine::semiconductor::Junction;
use orbe_model::Time;

use std::collections::{BTreeMap, BTreeSet};

use orbe_engine::diagnostic::Diagnostic;
use orbe_engine::name::Ref;

use super::{Circuit, ROOT};
use crate::diagnostic::{Code, Findings, Lint};
use crate::model::{Instance, Part, TypeRef};
use crate::name::and_list;
use crate::syntax::{self, Node};

const RULES: [fn(&Scope, &mut Findings); 9] = [
    unplaced,
    shorted,
    unconnected,
    ground_unnamed,
    rotation,
    no_model,
    floating,
    led_no_resistor,
    led_overcurrent,
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
    /// Nets left out of `net.floating`. See [`Circuit::quiet_floating`].
    quiet: BTreeSet<String>,
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
            engine: orbe_engine::netlist::parse_with(netlist.json.as_bytes(), &netlist.blobs)
                .expect("the engine read this netlist when it checked"),
            ground: &netlist.ground,
            quiet: netlist.refs.quiet_floating.clone(),
        };
        for rule in RULES {
            rule(&scope, out);
        }
    }

    /// Net names `net.floating` leaves out. A part lists those pins in `open`. The net of one
    /// stays out while no scene wire reaches it and every other pin on it, other than a hole,
    /// belongs to that part or a part inside it. A hole that only seats that pin, with nothing
    /// else in its strip, does not reach the net. A scene wire, another part's pin, or a wire
    /// to a hole on the net still warns. The ground ref does not reach a net.
    pub(super) fn quiet_floating(&mut self, nets: &BTreeMap<usize, String>) -> BTreeSet<String> {
        let keys: Vec<usize> = self
            .project
            .scene
            .comp
            .wires
            .iter()
            .flat_map(|wire| &wire.ends)
            .filter_map(|end| self.lookup(ROOT, end).ok().flatten())
            .collect();
        let root_of: Vec<usize> = (0..self.pins.len()).map(|key| self.find(key)).collect();
        let reached: BTreeSet<usize> = keys.into_iter().map(|key| root_of[key]).collect();
        let mut quiet = BTreeSet::new();
        for inst in 1..self.insts.len() {
            if self.insts[inst].dropped.is_some() {
                continue;
            }
            let open = match self.part(inst) {
                Some(part) if !part.open.is_empty() => part.open.clone(),
                _ => continue,
            };
            for pin_name in open {
                let Some(&key) = self.keys.get(&(inst, pin_name)) else {
                    continue;
                };
                let root = root_of[key];
                if reached.contains(&root) {
                    continue;
                }
                let contained = (0..self.pins.len()).all(|other| {
                    if root_of[other] != root {
                        return true;
                    }
                    let pin = &self.pins[other];
                    pin.hole
                        || self.insts[pin.inst].dropped.is_some()
                        || inside(self, pin.inst, inst)
                });
                if contained && let Some(name) = nets.get(&root) {
                    quiet.insert(name.clone());
                }
            }
        }
        quiet
    }
}

/// `inst` is `parent`, or a part inside it. The paths are segments, so `nano` does not
/// contain `nano2`.
fn inside(circuit: &Circuit, inst: usize, parent: usize) -> bool {
    let child = circuit.insts[inst].path.as_slice();
    let of = circuit.insts[parent].path.as_slice();
    child.len() >= of.len() && child[..of.len()] == of[..]
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
        if s.quiet.contains(net) {
            continue;
        }
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

/// A steady current over a rating, on a path that is only a supply or one port pin, one fixed
/// resistor and an LED to ground. The resistor may sit on either side of the LED. Anything else
/// on the LED's node, a second pin, a supply sharing the pin's net, or a source that changes,
/// stays quiet. The estimate uses the LED's own forward voltage at the rating and leaves out the
/// pin's output resistance, so it runs high: a path that is fine stays quiet.
fn led_overcurrent(s: &Scope, out: &mut Findings) {
    for (_, part) in s.engine.parts() {
        let Model::Led(led) = &part.model else {
            continue;
        };
        let Some(path) = series_path(&s.engine, part.name.as_str(), led.pins) else {
            continue;
        };
        let Some(drive) = drive_of(s, path.drive) else {
            continue;
        };
        let Some(found) = over_limits(led, path.ohms, &drive) else {
            continue;
        };
        let shown = part.name.as_str().replace("__", ".");
        let resistor = path.resistor.replace("__", ".");
        let top = s.top_named(part.name.as_str());
        let source = match &drive.pin {
            Some(pin) => format!("`{}` at {}", pin.label, volts_text(drive.volts)),
            None => volts_text(drive.volts),
        };
        out.push(
            Code::Lint(Lint::LedOvercurrent),
            top.name_span,
            format!(
                "`{shown}` would draw about {} through `{resistor}` ({}) from {source}, over {}; use at least {} ohm",
                about_current(found.current),
                ohm_text(path.ohms),
                and_list(&found.ratings.iter().map(String::as_str).collect::<Vec<_>>()),
                found.least
            ),
            format!("raise `{resistor}` to at least {} ohm", found.least),
        );
    }
}

struct SeriesPath {
    resistor: String,
    ohms: f64,
    drive: NetId,
}

/// The LED's free node holds only that LED and one fixed resistor. The resistor's other pin is
/// the drive when the LED's other pin is ground, or ground when the LED's other pin is the drive.
fn series_path(engine: &Netlist, led_name: &str, pins: [NetId; 2]) -> Option<SeriesPath> {
    let [anode, cathode] = pins;
    if cathode.is_ground() && !anode.is_ground() {
        let found = lone_resistor(engine, anode, led_name)?;
        if !found.other.is_ground() {
            return Some(SeriesPath {
                resistor: found.name,
                ohms: found.ohms,
                drive: found.other,
            });
        }
    }
    if !cathode.is_ground() && !anode.is_ground() {
        let found = lone_resistor(engine, cathode, led_name)?;
        if found.other.is_ground() {
            return Some(SeriesPath {
                resistor: found.name,
                ohms: found.ohms,
                drive: anode,
            });
        }
    }
    None
}

struct LoneResistor {
    name: String,
    ohms: f64,
    other: NetId,
}

/// The one resistor on `mid`, when every other part there is `led_name` and the resistor's
/// resistance is a fixed number.
fn lone_resistor(engine: &Netlist, mid: NetId, led_name: &str) -> Option<LoneResistor> {
    let mut found: Option<LoneResistor> = None;
    for (_, part) in engine.parts() {
        if !part.model.pins().contains(&mid) || part.name.as_str() == led_name {
            continue;
        }
        let Model::Resistor(resistor) = &part.model else {
            return None;
        };
        if found.is_some() {
            return None;
        }
        let ohms = resistor.resistance.fixed()?.si.get();
        if !(ohms.is_finite() && ohms > 0.0) {
            return None;
        }
        let other = resistor.pins.into_iter().find(|pin| *pin != mid)?;
        if other == mid {
            return None;
        }
        found = Some(LoneResistor {
            name: part.name.as_str().to_string(),
            ohms,
            other,
        });
    }
    found
}

struct Hold {
    volts: f64,
    /// The smallest `max_current` among the sources, when one of them states one.
    limit: Option<f64>,
}

/// A constant ground-referenced voltage on `net`, walking diode cathodes back to their anodes
/// and ignoring each diode's drop. `None` when no such source feeds the net, the sources
/// disagree, or any of them changes after t = 0.
fn held_voltage(engine: &Netlist, start: NetId) -> Option<Hold> {
    let mut found: Vec<(f64, Option<f64>)> = Vec::new();
    let mut stack = vec![(start, 0u8)];
    let mut seen = BTreeSet::new();
    while let Some((net, depth)) = stack.pop() {
        if !seen.insert(net.index()) {
            continue;
        }
        for (_, part) in engine.parts() {
            match &part.model {
                Model::VoltageSource(source) => {
                    let [positive, negative] = source.pins;
                    if positive != net || !negative.is_ground() {
                        continue;
                    }
                    if source.voltage.next_change_after(Time::from_ps(0)).is_some() {
                        return None;
                    }
                    let volts = source.voltage.after(Time::from_ps(0));
                    if !(volts.is_finite() && volts > 0.0) {
                        continue;
                    }
                    found.push((
                        volts,
                        source.max_current.as_ref().map(|value| value.si.get()),
                    ));
                }
                Model::Diode(diode) if depth < 4 => {
                    let [anode, cathode] = diode.pins;
                    if cathode == net {
                        stack.push((anode, depth + 1));
                    }
                }
                _ => {}
            }
        }
    }
    let (volts, _) = *found.first()?;
    if found.iter().any(|(other, _)| !same_volts(*other, volts)) {
        return None;
    }
    let limit = found
        .iter()
        .filter_map(|(_, limit)| *limit)
        .filter(|limit| limit.is_finite() && *limit > 0.0)
        .reduce(f64::min);
    Some(Hold { volts, limit })
}

fn same_volts(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * a.abs().max(1.0)
}

struct PortPin {
    part: String,
    pin: &'static str,
    vcc: Option<NetId>,
    rating: Option<f64>,
}

/// ATmega328P port pins. `PC6` is `RESET` and can drive. The crystal pins `PB6` and `PB7` cannot.
fn port_pin(name: &str) -> bool {
    let Some((port, index)) = name.split_at_checked(2) else {
        return false;
    };
    let Ok(index) = index.parse::<u8>() else {
        return false;
    };
    matches!((port, index), ("PD", 0..=7) | ("PB", 0..=5) | ("PC", 0..=6))
}

fn port_pins(engine: &Netlist, net: NetId) -> Vec<PortPin> {
    let mut found = Vec::new();
    for (_, part) in engine.parts() {
        let Model::Chip(Chip::Atmega328p(mcu)) = &part.model else {
            continue;
        };
        let names = part.model.pin_names();
        for (pin, name) in part.model.pins().iter().zip(names) {
            if *pin != net || !port_pin(name) {
                continue;
            }
            let vcc = part
                .model
                .pins()
                .iter()
                .zip(names)
                .find(|(_, pin_name)| **pin_name == "VCC")
                .map(|(id, _)| *id);
            let rating = mcu
                .ratings
                .as_ref()
                .and_then(|ratings| ratings.pin_current.as_ref())
                .map(|value| value.si.get())
                .filter(|amps| amps.is_finite() && *amps > 0.0);
            found.push(PortPin {
                part: part.name.as_str().to_string(),
                pin: name,
                vcc,
                rating,
            });
        }
    }
    found
}

struct PinDrive {
    label: String,
    rating: Option<f64>,
}

struct Drive {
    volts: f64,
    /// Set when exactly one port pin drives the net, so its rating applies and the supply's does not.
    pin: Option<PinDrive>,
    supply_limit: Option<f64>,
}

/// The voltage on `net` when one port pin drives it from its VCC, or when a supply holds the net
/// and no port pin does. A pin and a supply together, or two port pins, is not this path.
fn drive_of(s: &Scope, net: NetId) -> Option<Drive> {
    let supply = held_voltage(&s.engine, net);
    let pins = port_pins(&s.engine, net);
    if pins.len() > 1 || (pins.len() == 1 && supply.is_some()) {
        return None;
    }
    if let Some(pin) = pins.into_iter().next() {
        let held = held_voltage(&s.engine, pin.vcc?)?;
        let label = scene_label(s, net)
            .unwrap_or_else(|| format!("{}.{}", pin.part.replace("__", "."), pin.pin));
        return Some(Drive {
            volts: held.volts,
            pin: Some(PinDrive {
                label,
                rating: pin.rating,
            }),
            supply_limit: None,
        });
    }
    let supply = supply?;
    Some(Drive {
        volts: supply.volts,
        pin: None,
        supply_limit: supply.limit,
    })
}

/// The smallest label on `net`, dotted, such as `nano.D13`.
fn scene_label(s: &Scope, net: NetId) -> Option<String> {
    let name = s.engine.nets().find(|(id, _)| *id == net)?.1.as_str();
    let root = s.net_root(name)?;
    let mut best: Option<String> = None;
    for key in 0..s.circuit.pins.len() {
        if s.root[key] != root || !s.circuit.pins[key].label || s.circuit.pins[key].hole {
            continue;
        }
        let shown = s.circuit.flat_pin(key).replace("__", ".");
        if best.as_ref().is_none_or(|have| shown < *have) {
            best = Some(shown);
        }
    }
    best
}

struct Over {
    current: f64,
    ratings: Vec<String>,
    least: u64,
}

/// The current past each applicable rating, estimated as `(V - Vf) / R` with `Vf` at the LED's
/// own rating when it has one. The suggested resistance is the largest that puts every exceeded
/// rating strictly under.
fn over_limits(led: &Junction, ohms: f64, drive: &Drive) -> Option<Over> {
    let mut caps: Vec<(f64, String)> = Vec::new();
    if let Some(max) = led
        .max_current
        .as_ref()
        .map(|value| value.si.get())
        .filter(|amps| amps.is_finite() && *amps > 0.0)
    {
        caps.push((max, format!("its {} rating", about_current(max))));
    }
    if let Some(pin) = &drive.pin {
        if let Some(max) = pin.rating {
            caps.push((
                max,
                format!("`{}`'s {} rating", pin.label, about_current(max)),
            ));
        }
    } else if let Some(max) = drive.supply_limit {
        caps.push((max, format!("the supply's {} rating", about_current(max))));
    }
    let mut ratings = Vec::new();
    let mut least = 0u64;
    let mut tightest = f64::INFINITY;
    for (limit, phrase) in &caps {
        let vf = forward(led, *limit)?;
        if drive.volts <= vf + *limit * ohms {
            continue;
        }
        let n = ohms_at_least(drive.volts, vf, *limit)?;
        ratings.push(phrase.clone());
        least = least.max(n);
        tightest = tightest.min(*limit);
    }
    if ratings.is_empty() {
        return None;
    }
    let shown_at = led
        .max_current
        .as_ref()
        .map(|value| value.si.get())
        .filter(|amps| amps.is_finite() && *amps > 0.0)
        .unwrap_or(tightest);
    let vf = forward(led, shown_at)?;
    if vf >= drive.volts {
        return None;
    }
    let current = (drive.volts - vf) / ohms;
    if !(current.is_finite() && current > 0.0) {
        return None;
    }
    Some(Over {
        current,
        ratings,
        least,
    })
}

/// `vj + i * Rs` at `i`, from the junction's own Shockley parameters. `None` when they are not a
/// forward voltage.
fn forward(led: &Junction, current: f64) -> Option<f64> {
    let saturation = led.saturation_current.si.get();
    let emission = led.emission_coefficient.si.get();
    let series = led.series_resistance.si.get();
    if !(saturation > 0.0 && emission > 0.0 && series >= 0.0 && current > 0.0) {
        return None;
    }
    let vj = emission * Junction::VT * libm::log(1.0 + current / saturation);
    let volts = vj + current * series;
    volts.is_finite().then_some(volts)
}

/// The smallest whole ohm strictly under `limit` at this voltage.
fn ohms_at_least(volts: f64, vf: f64, limit: f64) -> Option<u64> {
    let need = (volts - vf) / limit;
    if !(need.is_finite() && need > 0.0 && need < u64::MAX as f64) {
        return None;
    }
    Some(need.floor() as u64 + 1)
}

fn about_current(amps: f64) -> String {
    let abs = amps.abs();
    let (scale, unit) = if abs >= 1.0 {
        (1.0, "A")
    } else if abs >= 1e-3 {
        (1e3, "mA")
    } else if abs >= 1e-6 {
        (1e6, "µA")
    } else {
        (1.0, "A")
    };
    format!("{} {unit}", sig3(amps * scale))
}

fn sig3(value: f64) -> String {
    let digits = if value.abs() >= 100.0 {
        0
    } else if value.abs() >= 10.0 {
        1
    } else {
        2
    };
    let mut text = format!("{value:.digits$}");
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    text
}

fn ohm_text(ohms: f64) -> String {
    let whole = ohms.round();
    if (ohms - whole).abs() < 1e-6 && (0.0..1e15).contains(&whole) {
        format!("{} ohm", whole as u64)
    } else {
        format!("{ohms} ohm")
    }
}

fn volts_text(volts: f64) -> String {
    let whole = volts.round();
    if (volts - whole).abs() < 1e-6 {
        format!("{} V", whole as i64)
    } else {
        format!("{volts:.3} V")
    }
}
