use sim_core::name::Ref;
use sim_core::netlist::{NetId, Netlist, PartId};

use crate::dense::{Matrix, Singular};
use crate::element;

pub(crate) const GMIN: f64 = 1e-12;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Unknowns {
    pub internal_nodes: usize,
    pub branches: usize,
}

pub(crate) struct Layout {
    net_rows: usize,
    part_rows: Vec<(usize, usize)>,
    node_rows: Vec<usize>,
    size: usize,
}

impl Layout {
    pub(crate) fn new(netlist: &Netlist) -> Layout {
        let mut row = 0;
        let mut node_rows = Vec::new();
        let net_rows = netlist.nets().len().saturating_sub(1);
        for _ in 0..net_rows {
            node_rows.push(row);
            row += 1;
        }
        let mut part_rows = Vec::with_capacity(netlist.parts().len());
        for (_, part) in netlist.parts() {
            let extra = element::unknowns(&part.model);
            let internal_at = row;
            for _ in 0..extra.internal_nodes {
                node_rows.push(row);
                row += 1;
            }
            let branch_at = row;
            row += extra.branches;
            part_rows.push((internal_at, branch_at));
        }
        Layout {
            net_rows,
            part_rows,
            node_rows,
            size: row,
        }
    }

    pub(crate) fn size(&self) -> usize {
        self.size
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
            let (internal_at, branch_at) = self.part_rows[id.index()];
            let extra = element::unknowns(&part.model);
            let end = branch_at + extra.branches;
            if row >= internal_at && row < end {
                return Ref::Part(part.name.clone());
            }
        }
        panic!("row {row} is outside this layout");
    }
}

pub(crate) struct System {
    a: Matrix,
    b: Vec<f64>,
}

impl System {
    pub(crate) fn new(layout: &Layout, gmin: f64) -> System {
        let mut a = Matrix::zeros(layout.size());
        for &row in &layout.node_rows {
            a.add(row, row, gmin);
        }
        System {
            a,
            b: vec![0.0; layout.size()],
        }
    }

    pub(crate) fn solve(self) -> Result<Solution, Singular> {
        Ok(Solution {
            x: crate::dense::solve(self.a, self.b)?,
        })
    }
}

pub(crate) struct Solution {
    x: Vec<f64>,
}

impl Solution {
    pub(crate) fn get(&self, row: usize) -> f64 {
        self.x[row]
    }

    pub(crate) fn components(&self) -> &[f64] {
        &self.x
    }

    pub(crate) fn voltage(&self, net: NetId) -> f64 {
        if net.is_ground() {
            0.0
        } else {
            self.get(net.index() - 1)
        }
    }
}

pub(crate) struct Stamp<'a> {
    pub(crate) sys: &'a mut System,
    pub(crate) layout: &'a Layout,
    pub(crate) part: PartId,
}

impl Stamp<'_> {
    pub(crate) fn conductance(&mut self, a: NetId, b: NetId, g: f64) {
        self.add_term(a, a, g);
        self.add_term(b, b, g);
        self.add_term(a, b, -g);
        self.add_term(b, a, -g);
    }

    pub(crate) fn voltage_source(&mut self, p: NetId, n: NetId, k: usize, volts: f64) {
        let (_, branch_at) = self.layout.part_rows[self.part.index()];
        let row = branch_at + k;
        if let Some(prow) = self.net_row(p) {
            self.sys.a.add(prow, row, 1.0);
            self.sys.a.add(row, prow, 1.0);
        }
        if let Some(nrow) = self.net_row(n) {
            self.sys.a.add(nrow, row, -1.0);
            self.sys.a.add(row, nrow, -1.0);
        }
        self.sys.b[row] += volts;
    }

    fn add_term(&mut self, row: NetId, col: NetId, value: f64) {
        let (Some(row), Some(col)) = (self.net_row(row), self.net_row(col)) else {
            return;
        };
        self.sys.a.add(row, col, value);
    }

    fn net_row(&self, id: NetId) -> Option<usize> {
        if id.is_ground() {
            None
        } else {
            Some(id.index() - 1)
        }
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

    pub(crate) fn branch(&self, k: usize) -> f64 {
        let (_, branch_at) = self.layout.part_rows[self.part.index()];
        self.x.get(branch_at + k)
    }
}
