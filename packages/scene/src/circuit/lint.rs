//! The lint rules: what a scene that checks is likely to have wrong or left unclear. Each rule
//! reads the checked circuit and pushes warnings. None of them blocks.

use orbe_engine::netlist::{Conduction, Model, Netlist};

use std::collections::{BTreeMap, BTreeSet};

use orbe_engine::diagnostic::Diagnostic;
use orbe_engine::name::Ref;

use super::plug::Join;
use super::{Circuit, ROOT};
use crate::diagnostic::{Code, Findings, Lint};
use crate::model::{Instance, Part, TypeRef};
use crate::name::and_list;
use crate::syntax::{self, Node};

const RULES: [fn(&Scope, &mut Findings); 6] = [
    unplaced,
    shorted,
    unconnected,
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
    /// Nets left out of `net.floating`. See [`Circuit::quiet_floating`].
    quiet: BTreeSet<String>,
    /// A part drawn in a board's hole while it is not seated: the board, the hole, the pin,
    /// and how to join that pin without claiming a hole something already holds.
    hovering: BTreeMap<String, (String, String, String, Join)>,
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
        let hovering = self.hovering(&root);
        let scope = Scope {
            circuit: &*self,
            root,
            nets,
            wired,
            tops,
            scene: tree,
            engine: orbe_engine::netlist::parse_with(netlist.json.as_bytes(), &netlist.blobs)
                .expect("the engine read this netlist when it checked"),
            quiet: netlist.refs.quiet_floating.clone(),
            hovering,
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
    if let Some(first) = unplaced.first() {
        let names: Vec<String> = unplaced.iter().map(|i| format!("`{}`", i.name)).collect();
        let names = and_list(&names.iter().map(String::as_str).collect::<Vec<_>>());
        let message = if unplaced.len() == 1 {
            format!("{names} has no position, so the 3D page places it in a row for drawing only")
        } else {
            format!(
                "{names} have no position, so the 3D page places them in a row for drawing only"
            )
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
    for &(instance, _, _) in &s.tops {
        let Some((board, hole, pin, join)) = s.hovering.get(&instance.name) else {
            continue;
        };
        let turn = layout
            .and_then(|node| node.get(&instance.name))
            .and_then(|place| place.get("rotate"))
            .and_then(turned);
        let leg = format!("{}.{pin}", instance.name);
        let seat = match &turn {
            Some(turn) => format!(
                "\"{}\": {{ \"on\": {{ \"{pin}\": \"{board}.{hole}\" }}, \"rotate\": {turn} }}",
                instance.name
            ),
            None => format!(
                "\"{}\": {{ \"on\": {{ \"{pin}\": \"{board}.{hole}\" }} }}",
                instance.name
            ),
        };
        let hint = match join {
            Join::Seat => format!("seat it with \"on\", as {seat}"),
            Join::Contact(target) => format!(
                "join it with a contact wire, as [\"{leg}\", \"{target}\", {{ \"kind\": \"contact\" }}]"
            ),
            Join::Full => super::shared_hole_hint(&[]),
        };
        out.push(
            Code::Lint(Lint::Unplaced),
            instance.name_span,
            format!("`{leg}` is drawn in `{board}.{hole}` and joins nothing there"),
            hint,
        );
    }
}

/// `rotate` as written, when it is three numbers and not all zero.
fn turned(node: &Node) -> Option<String> {
    let items = node.items()?;
    if items.len() != 3 {
        return None;
    }
    let numbers: Vec<&str> = items.iter().filter_map(Node::raw_number).collect();
    if numbers.len() != 3 {
        return None;
    }
    let zero = items.iter().all(|item| item.number() == Some(0.0));
    if zero {
        return None;
    }
    Some(format!("[{}, {}, {}]", numbers[0], numbers[1], numbers[2]))
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

fn find(parent: &mut [usize], mut net: usize) -> usize {
    let mut root = net;
    while parent[root] != root {
        root = parent[root];
    }
    while parent[net] != root {
        let next = parent[net];
        parent[net] = root;
        net = next;
    }
    root
}

fn unite(parent: &mut [usize], a: usize, b: usize) {
    let a = find(parent, a);
    let b = find(parent, b);
    if a != b {
        parent[b] = a;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Visit {
    Unvisited,
    /// Reached only by stepping across this relay; its other contacts stay closed from here.
    Blocked(usize),
    /// Reached directly or through two distinct relays; every contact may leave from here.
    Full,
}

#[derive(Clone, Copy)]
enum Target {
    Held(usize),
    Group(usize),
}

#[derive(Clone, Copy)]
struct Edge {
    to: Target,
    relay: Option<usize>,
}

enum Expand {
    All,
    Except(usize),
    Only(usize),
}

/// Group non-held nets joined by `Passes` parts other than `skip`, then build the edges leaving
/// each group through `Passes`, `Switches` and `Contacts`. Contracting `Passes` keeps a dangling
/// switch or diode on a relay's COM from bouncing a walk back to COM and out the other throw.
fn conduction_graph(
    engine: &Netlist,
    skip: usize,
    held: impl Fn(usize) -> bool,
) -> (Vec<usize>, Vec<Vec<Edge>>) {
    let n = engine.nets().len();
    let mut parent: Vec<usize> = (0..n).collect();
    for (id, part) in engine.parts() {
        if id.index() == skip || part.model.conduction() != Conduction::Passes {
            continue;
        }
        let mut first = None;
        for pin in part.model.pins() {
            let net = pin.index();
            if held(net) {
                continue;
            }
            match first {
                None => first = Some(net),
                Some(root) => unite(&mut parent, root, net),
            }
        }
    }
    let group: Vec<usize> = (0..n).map(|net| find(&mut parent, net)).collect();
    let target = |net: usize| {
        if held(net) {
            Target::Held(net)
        } else {
            Target::Group(group[net])
        }
    };
    let mut adj = vec![Vec::new(); n];
    for (id, part) in engine.parts() {
        if id.index() == skip {
            continue;
        }
        let pins = part.model.pins();
        match part.model.conduction() {
            Conduction::Passes => {
                for &pin in pins {
                    let u = pin.index();
                    if held(u) {
                        continue;
                    }
                    let from = group[u];
                    for &other in pins {
                        let v = other.index();
                        if held(v) {
                            adj[from].push(Edge {
                                to: Target::Held(v),
                                relay: None,
                            });
                        }
                    }
                }
            }
            Conduction::Switches(from_pin, to_pin) => {
                let u = pins[from_pin].index();
                let v = pins[to_pin].index();
                if !held(u) && held(v) {
                    adj[group[u]].push(Edge {
                        to: Target::Held(v),
                        relay: None,
                    });
                }
            }
            Conduction::Contacts(pairs) => {
                let relay = Some(id.index());
                for &(from_pin, to_pin) in pairs {
                    let u = pins[from_pin].index();
                    let v = pins[to_pin].index();
                    if !held(u) && !held(v) && group[u] == group[v] {
                        continue;
                    }
                    if !held(u) {
                        adj[group[u]].push(Edge {
                            to: target(v),
                            relay,
                        });
                    }
                    if !held(v) {
                        adj[group[v]].push(Edge {
                            to: target(u),
                            relay,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    (group, adj)
}

/// Held nets reachable from `start` without passing through a `held` net or crossing two throws
/// of the same relay. Each non-held group transitions at most twice (`Unvisited -> Blocked ->
/// Full`), so each outgoing edge is inspected at most once.
fn reach_held(
    start: usize,
    held: impl Fn(usize) -> bool,
    group: &[usize],
    adj: &[Vec<Edge>],
) -> BTreeSet<usize> {
    let mut reached = BTreeSet::new();
    if held(start) {
        reached.insert(start);
        return reached;
    }
    let mut state = vec![Visit::Unvisited; group.len()];
    let root = group[start];
    state[root] = Visit::Full;
    let mut stack = vec![(root, Expand::All)];
    while let Some((node, mode)) = stack.pop() {
        for &edge in &adj[node] {
            let allowed = match mode {
                Expand::All => true,
                Expand::Except(relay) => edge.relay != Some(relay),
                Expand::Only(relay) => edge.relay == Some(relay),
            };
            if !allowed {
                continue;
            }
            match edge.to {
                Target::Held(net) => {
                    reached.insert(net);
                }
                Target::Group(next) => match (state[next], edge.relay) {
                    (Visit::Unvisited, None) => {
                        state[next] = Visit::Full;
                        stack.push((next, Expand::All));
                    }
                    (Visit::Unvisited, Some(relay)) => {
                        state[next] = Visit::Blocked(relay);
                        stack.push((next, Expand::Except(relay)));
                    }
                    (Visit::Blocked(blocked), via) if via != Some(blocked) => {
                        state[next] = Visit::Full;
                        stack.push((next, Expand::Only(blocked)));
                    }
                    _ => {}
                },
            }
        }
    }
    reached
}

/// An LED whose path, through other junctions, switches, relay contacts and transistors
/// switched to a held net, joins two driven nets with no resistor in series. A relay's `com`
/// meets `no` or `nc`, one throw at a time, so a path never runs from `no` through `com` to
/// `nc`. A diode's cathode is driven when its
/// anode is a source's net. One side of the LED can reach a supply through a diode while the
/// other side reaches it only through a resistor. That resistor is in series. A resistor that
/// bridges two sides which each already reach a driver is in parallel and limits nothing. A
/// chip pin drives its net unless a resistor feeds that net from a held net off the path, since
/// then the pin more likely reads the net.
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
    for (led_id, led) in engine.parts() {
        if !matches!(led.model, Model::Led(_)) {
            continue;
        }
        let [a, b] = led.model.pins()[..] else {
            continue;
        };
        let (a, b) = (a.index(), b.index());
        let (held_a, held_b) = if held(a) && held(b) {
            (BTreeSet::from([a]), BTreeSet::from([b]))
        } else {
            let (group, adj) = conduction_graph(engine, led_id.index(), held);
            (
                reach_held(a, held, &group, &adj),
                reach_held(b, held, &group, &adj),
            )
        };
        let all_held: BTreeSet<usize> = held_a.union(&held_b).copied().collect();
        let fed = |net: usize| {
            engine.parts().any(|(_, part)| {
                let pins = part.model.pins();
                part.model.conduction() == Conduction::Limits
                    && pins.iter().any(|n| n.index() == net)
                    && pins
                        .iter()
                        .any(|n| !all_held.contains(&n.index()) && held(n.index()))
            })
        };
        let is_driver = |net: usize| fixed[net] || (chip[net] && !fed(net));
        if !held_a.iter().any(|&net| is_driver(net)) || !held_b.iter().any(|&net| is_driver(net)) {
            continue;
        }
        let ends: Vec<String> = all_held
            .iter()
            .copied()
            .filter(|&net| is_driver(net))
            .map(|net| format!("`{}`", names[net]))
            .collect();
        if ends.len() < 2 {
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
