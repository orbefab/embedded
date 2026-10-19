use std::ops::Range;

use sim_core::name::Ref;
use sim_core::netlist::{Model, NetId, Netlist, PartId};

use crate::chip;
use crate::dense::{Matrix, Singular};
use crate::element;
use crate::newton::{Conditions, Iterate};

pub(crate) const GMIN: f64 = 1e-12;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Unknowns {
    pub internal_nodes: usize,
    pub branches: usize,
    /// Junction voltages Newton limits. Slots in `Iterate::junctions`, not matrix rows.
    pub junctions: usize,
}

/// Where one part's unknowns start.
struct PartRows {
    internal: usize,
    branch: usize,
    junctions: Range<usize>,
}

pub(crate) struct Layout {
    net_rows: usize,
    part_rows: Vec<PartRows>,
    /// Ascending.
    node_rows: Vec<usize>,
    size: usize,
    junctions: usize,
    /// The chips, ascending. A chip's index here is its slot in `Conditions::chips`.
    chips: Vec<PartId>,
}

impl Layout {
    pub(crate) fn new(netlist: &Netlist) -> Layout {
        let mut row = 0;
        let mut junction = 0;
        let mut node_rows = Vec::new();
        let net_rows = netlist.nets().len().saturating_sub(1);
        for _ in 0..net_rows {
            node_rows.push(row);
            row += 1;
        }
        let mut part_rows = Vec::with_capacity(netlist.parts().len());
        let mut chips = Vec::new();
        for (id, part) in netlist.parts() {
            if let Model::Chip(_) = part.model {
                chips.push(id);
            }
            let extra = element::unknowns(&part.model);
            let internal = row;
            for _ in 0..extra.internal_nodes {
                node_rows.push(row);
                row += 1;
            }
            let branch = row;
            row += extra.branches;
            let junctions = junction..junction + extra.junctions;
            junction = junctions.end;
            part_rows.push(PartRows {
                internal,
                branch,
                junctions,
            });
        }
        Layout {
            net_rows,
            part_rows,
            node_rows,
            size: row,
            junctions: junction,
            chips,
        }
    }

    /// The chips in slot order.
    pub(crate) fn chips(&self) -> &[PartId] {
        &self.chips
    }

    /// `part`'s internal node `k`.
    pub(crate) fn internal(&self, part: PartId, k: usize) -> Node {
        Node::Row(self.part_rows[part.index()].internal + k)
    }

    pub(crate) fn size(&self) -> usize {
        self.size
    }

    pub(crate) fn junctions(&self) -> usize {
        self.junctions
    }

    /// The row of `part`'s branch `k`.
    pub(crate) fn branch_row(&self, part: PartId, k: usize) -> usize {
        self.part_rows[part.index()].branch + k
    }

    pub(crate) fn junction_slots(&self, part: PartId) -> Range<usize> {
        self.part_rows[part.index()].junctions.clone()
    }

    /// Net and internal rows hold volts. Branch rows hold amperes.
    pub(crate) fn is_node_row(&self, row: usize) -> bool {
        self.node_rows.binary_search(&row).is_ok()
    }

    pub(crate) fn describe(&self, netlist: &Netlist, row: usize) -> Ref {
        if row < self.net_rows {
            let (_, name) = netlist
                .nets()
                .filter(|(id, _)| !id.is_ground())
                .nth(row)
                .expect("row is a net voltage");
            return Ref::Net(name.clone());
        }
        for (id, part) in netlist.parts() {
            let rows = &self.part_rows[id.index()];
            let extra = element::unknowns(&part.model);
            let end = rows.branch + extra.branches;
            if row >= rows.internal && row < end {
                return Ref::Part(part.name.clone());
            }
        }
        panic!("row {row} is outside this layout");
    }
}

/// A matrix row, or ground, which has none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Node {
    Ground,
    Row(usize),
}

impl From<NetId> for Node {
    fn from(net: NetId) -> Node {
        if net.is_ground() {
            Node::Ground
        } else {
            Node::Row(net.index() - 1)
        }
    }
}

pub(crate) struct System {
    a: Matrix,
    b: Vec<f64>,
}

impl System {
    /// Empty storage. `reset` loads `gmin` before each stamp.
    pub(crate) fn new(layout: &Layout) -> System {
        System {
            a: Matrix::zeros(layout.size()),
            b: vec![0.0; layout.size()],
        }
    }

    /// Drop the last factorization and stamp `gmin` on every node row.
    pub(crate) fn reset(&mut self, layout: &Layout, gmin: f64) {
        self.a.clear();
        self.b.fill(0.0);
        for &row in &layout.node_rows {
            self.a.add(row, row, gmin);
        }
    }

    pub(crate) fn conductance(&mut self, a: impl Into<Node>, b: impl Into<Node>, g: f64) {
        let (a, b) = (a.into(), b.into());
        self.add_term(a, a, g);
        self.add_term(b, b, g);
        self.add_term(a, b, -g);
        self.add_term(b, a, -g);
    }

    /// `amps` flows out of `from`, through the element, into `to`.
    pub(crate) fn current(&mut self, from: impl Into<Node>, to: impl Into<Node>, amps: f64) {
        if let Node::Row(row) = from.into() {
            self.b[row] -= amps;
        }
        if let Node::Row(row) = to.into() {
            self.b[row] += amps;
        }
    }

    fn add_term(&mut self, row: Node, col: Node, value: f64) {
        let (Node::Row(row), Node::Row(col)) = (row, col) else {
            return;
        };
        self.a.add(row, col, value);
    }

    pub(crate) fn solve(&mut self) -> Result<Solution, Singular> {
        Ok(Solution::from(crate::dense::solve(
            &mut self.a,
            &mut self.b,
        )?))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Solution {
    x: Vec<f64>,
}

impl From<Vec<f64>> for Solution {
    fn from(x: Vec<f64>) -> Solution {
        Solution { x }
    }
}

impl Solution {
    pub(crate) fn zeros(n: usize) -> Solution {
        Solution { x: vec![0.0; n] }
    }

    pub(crate) fn get(&self, row: usize) -> f64 {
        self.x[row]
    }

    pub(crate) fn components(&self) -> &[f64] {
        &self.x
    }

    pub(crate) fn voltage(&self, net: NetId) -> f64 {
        self.node(net.into())
    }

    pub(crate) fn node(&self, node: Node) -> f64 {
        match node {
            Node::Ground => 0.0,
            Node::Row(row) => self.get(row),
        }
    }
}

pub(crate) struct Stamp<'a> {
    pub(crate) sys: &'a mut System,
    pub(crate) layout: &'a Layout,
    pub(crate) part: PartId,
    pub(crate) iterate: &'a Iterate,
    pub(crate) conditions: &'a Conditions<'a>,
}

impl Stamp<'_> {
    pub(crate) fn internal(&self, k: usize) -> Node {
        Node::Row(self.rows().internal + k)
    }

    /// This chip's state in the solve's conditions.
    pub(crate) fn chip(&self) -> &chip::State {
        let slot = self.layout.chips.binary_search(&self.part);
        &self.conditions.chips[slot.expect("only a chip reads its state")]
    }

    /// This part in another solution, such as the last accepted one.
    pub(crate) fn probe<'s>(&'s self, x: &'s Solution) -> Probe<'s> {
        Probe {
            layout: self.layout,
            part: self.part,
            x,
        }
    }

    /// The limited voltage of this part's junction `k`, from the iterate.
    pub(crate) fn junction(&self, k: usize) -> f64 {
        self.iterate.junctions[self.rows().junctions.start + k]
    }

    pub(crate) fn conductance(&mut self, a: impl Into<Node>, b: impl Into<Node>, g: f64) {
        self.sys.conductance(a, b, g);
    }

    /// `amps` flows out of `from`, through the part, into `to`.
    pub(crate) fn current(&mut self, from: impl Into<Node>, to: impl Into<Node>, amps: f64) {
        self.sys.current(from, to, amps);
    }

    /// `volts` is the netlist value. The stamp scales it by `conditions.source_scale`.
    pub(crate) fn voltage_source(&mut self, p: NetId, n: NetId, k: usize, volts: f64) {
        self.branch_current(p, n, k);
        self.branch_row(p, n, k, 0.0, 1.0, volts * self.conditions.source_scale);
    }

    /// Branch `k` carries a current that enters the part at `p` and leaves at `n`.
    pub(crate) fn branch_current(&mut self, p: NetId, n: NetId, k: usize) {
        let row = self.rows().branch + k;
        if let Node::Row(prow) = p.into() {
            self.sys.a.add(prow, row, 1.0);
        }
        if let Node::Row(nrow) = n.into() {
            self.sys.a.add(nrow, row, -1.0);
        }
    }

    /// Branch `k`'s own row: `ci * i_k + cv * (V(p) - V(n)) = rhs`.
    pub(crate) fn branch_row(&mut self, p: NetId, n: NetId, k: usize, ci: f64, cv: f64, rhs: f64) {
        let row = self.rows().branch + k;
        if ci != 0.0 {
            self.sys.a.add(row, row, ci);
        }
        if let Node::Row(prow) = p.into() {
            self.sys.a.add(row, prow, cv);
        }
        if let Node::Row(nrow) = n.into() {
            self.sys.a.add(row, nrow, -cv);
        }
        self.sys.b[row] += rhs;
    }

    fn rows(&self) -> &PartRows {
        &self.layout.part_rows[self.part.index()]
    }
}

pub(crate) struct Probe<'a> {
    pub(crate) layout: &'a Layout,
    pub(crate) part: PartId,
    pub(crate) x: &'a Solution,
}

impl Probe<'_> {
    pub(crate) fn v(&self, net: NetId) -> f64 {
        self.x.voltage(net)
    }

    pub(crate) fn internal(&self, k: usize) -> f64 {
        self.x.get(self.rows().internal + k)
    }

    pub(crate) fn branch(&self, k: usize) -> f64 {
        self.x.get(self.rows().branch + k)
    }

    fn rows(&self) -> &PartRows {
        &self.layout.part_rows[self.part.index()]
    }
}
