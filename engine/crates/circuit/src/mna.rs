use std::ops::Range;

use sim_core::name::Ref;
use sim_core::netlist::{NetId, Netlist, PartId};

use crate::chip;
use crate::dense::{Matrix, Singular};
use crate::element;
use crate::newton::{Conditions, Iterate};

pub(crate) const GMIN: f64 = 1e-12;

#[cfg(test)]
thread_local! {
    /// The systems this thread has solved, which a test reads to count Newton's solves.
    pub(crate) static SOLVES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

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
    /// The passive chips, ascending. A chip's index here is its slot in `Conditions::chips`.
    chips: Vec<PartId>,
}

impl Layout {
    /// `chips` are the passive chips in part order.
    pub(crate) fn new(netlist: &Netlist, chips: Vec<PartId>) -> Layout {
        let mut row = 0;
        let mut junction = 0;
        let mut node_rows = Vec::new();
        let net_rows = netlist.nets().len().saturating_sub(1);
        for _ in 0..net_rows {
            node_rows.push(row);
            row += 1;
        }
        let mut part_rows = Vec::with_capacity(netlist.parts().len());
        for (_, part) in netlist.parts() {
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

    /// The passive chips in slot order.
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
        #[cfg(test)]
        SOLVES.with(|solves| solves.set(solves.get() + 1));
        Ok(Solution::from(crate::dense::solve(
            &mut self.a,
            &mut self.b,
        )?))
    }
}

/// The most unknowns a system `Memo` keeps. An entry holds n² + 3n values, and a memo holds
/// its kept systems and the one it is filling, so at most 9 × (128² + 3 × 128) values, about
/// 1.15 MiB. At `dense::MAX_UNKNOWNS` it would be about 1.1 GiB.
const MEMO_UNKNOWNS: usize = 128;

/// How many systems a `Memo` keeps.
const MEMO_SYSTEMS: usize = 8;

/// The ceiling on the bytes a full `Memo` holds, 1.25 MiB. A full one holds about 1.15 MiB.
const MEMO_BYTES: usize = 5 << 18;

const _: () = assert!(
    (MEMO_SYSTEMS + 1) * (MEMO_UNKNOWNS * MEMO_UNKNOWNS + 3 * MEMO_UNKNOWNS) * size_of::<f64>()
        <= MEMO_BYTES
);

/// The latest systems solved, each with its solution. `dense::solve` reads nothing but the
/// matrix and the right-hand side, so a system equal to a kept one to the bit has the kept
/// solution. The key is the whole system, so nothing goes stale. A source step, a switch
/// toggle, a pin's new drive, a live `set`, another step length or another held voltage
/// changes a stamped value, which is a different key, or stamps the same system again, whose
/// solution has not changed.
#[derive(Default)]
pub(crate) struct Memo {
    /// Most recent first.
    systems: Vec<Kept>,
    /// A copy of a missed system, taken before the solve factors the system in place. It joins
    /// `systems` with its solution only once the solve succeeds, so a singular miss evicts nothing.
    next: Kept,
}

#[derive(Default)]
struct Kept {
    a: Vec<f64>,
    /// The diagonal of `a`. Systems that share `b` mostly differ there, in a step's companion
    /// conductance or a pin's drive, so it rejects them before the whole matrix is compared.
    diagonal: Vec<f64>,
    b: Vec<f64>,
    x: Vec<f64>,
}

impl Memo {
    /// `system`'s solution, the kept one when `system` is kept. Any other system up to
    /// `MEMO_UNKNOWNS` is solved and kept unless it is singular.
    pub(crate) fn solve(&mut self, system: &mut System) -> Result<Solution, Singular> {
        let n = system.b.len();
        if n > MEMO_UNKNOWNS {
            return system.solve();
        }
        let a = system.a.entries();
        if let Some(k) = self.systems.iter().position(|kept| kept.is(a, &system.b)) {
            let kept = self.systems.remove(k);
            let x = Solution::from(kept.x.clone());
            self.systems.insert(0, kept);
            return Ok(x);
        }
        let next = &mut self.next;
        next.a.clear();
        next.a.extend_from_slice(a);
        next.diagonal.clear();
        next.diagonal.extend((0..n).map(|k| a[k * (n + 1)]));
        next.b.clear();
        next.b.extend_from_slice(&system.b);
        let x = system.solve()?;
        next.x.clear();
        next.x.extend_from_slice(x.components());
        let spare = if self.systems.len() == MEMO_SYSTEMS {
            self.systems.pop().expect("the memo is full")
        } else {
            Kept::default()
        };
        self.systems
            .insert(0, std::mem::replace(&mut self.next, spare));
        Ok(x)
    }
}

impl Kept {
    /// Whether `a` and `b` are this system to the bit.
    fn is(&self, a: &[f64], b: &[f64]) -> bool {
        let n = b.len();
        same_bits(&self.b, b)
            && (self.diagonal.iter().enumerate())
                .all(|(k, value)| value.to_bits() == a[k * (n + 1)].to_bits())
            && same_bits(&self.a, a)
    }
}

/// Whether two slices hold the same bits. It folds each whole chunk of eight, so the compare
/// vectorizes.
fn same_bits(left: &[f64], right: &[f64]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let differ = |l: &[f64], r: &[f64]| {
        l.iter()
            .zip(r)
            .fold(0, |acc, (p, q)| acc | (p.to_bits() ^ q.to_bits()))
    };
    let ((left, left_rest), (right, right_rest)) = (left.as_chunks::<8>(), right.as_chunks::<8>());
    differ(left_rest, right_rest) == 0 && left.iter().zip(right).all(|(l, r)| differ(l, r) == 0)
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

    pub(crate) fn add(&mut self, row: usize, by: f64) {
        self.x[row] += by;
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
    /// Read only through `junction`, so a stamp that reads the iterate calls it by name.
    iterate: &'a Iterate,
    pub(crate) conditions: &'a Conditions<'a>,
}

impl<'a> Stamp<'a> {
    pub(crate) fn new(
        sys: &'a mut System,
        layout: &'a Layout,
        part: PartId,
        iterate: &'a Iterate,
        conditions: &'a Conditions<'a>,
    ) -> Stamp<'a> {
        Stamp {
            sys,
            layout,
            part,
            iterate,
            conditions,
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    /// `n` unknowns, `diagonal` on the diagonal, `above` just above it, and `rhs` in `b`.
    fn system(n: usize, diagonal: f64, above: f64, rhs: f64) -> System {
        let mut a = Matrix::zeros(n);
        for k in 0..n {
            a.add(k, k, diagonal);
            if k + 1 < n {
                a.add(k, k + 1, above);
            }
        }
        System { a, b: vec![rhs; n] }
    }

    fn bits(solution: &Solution) -> Vec<u64> {
        solution.components().iter().map(|v| v.to_bits()).collect()
    }

    /// The solution's bits, and whether the memo solved the system to get them.
    fn through(memo: &mut Memo, mut system: System) -> (Vec<u64>, bool) {
        let before = SOLVES.with(|solves| solves.get());
        let x = memo.solve(&mut system).unwrap();
        (bits(&x), SOLVES.with(|solves| solves.get()) > before)
    }

    fn fresh(mut system: System) -> Vec<u64> {
        bits(&system.solve().unwrap())
    }

    #[test]
    fn a_kept_system_returns_the_bits_of_a_fresh_solve_without_solving() {
        let mut memo = Memo::default();
        let solved = fresh(system(3, 4.0, 1.0, 1.0));
        assert_eq!(
            through(&mut memo, system(3, 4.0, 1.0, 1.0)),
            (solved.clone(), true)
        );
        assert_eq!(
            through(&mut memo, system(3, 4.0, 1.0, 1.0)),
            (solved, false)
        );
    }

    /// Each one shares the other two parts with the kept system: the right-hand side, the
    /// diagonal, and the entries off it.
    #[test]
    fn a_system_that_differs_in_any_bit_is_solved() {
        for other in [
            system(3, 4.0, 1.0, 2.0),
            system(3, 5.0, 1.0, 1.0),
            system(3, 4.0, 2.0, 1.0),
        ] {
            let mut memo = Memo::default();
            through(&mut memo, system(3, 4.0, 1.0, 1.0));
            let solved = fresh(system(3, 4.0, 1.0, 1.0));
            let (x, ran) = through(&mut memo, other);
            assert!(ran && x != solved, "{x:?}");
        }
    }

    #[test]
    fn a_singular_system_is_not_kept() {
        let mut memo = Memo::default();
        assert!(memo.solve(&mut system(3, 0.0, 1.0, 1.0)).is_err());
        assert!(memo.systems.is_empty());
    }

    #[test]
    fn a_singular_system_on_a_full_memo_evicts_nothing() {
        let mut memo = Memo::default();
        for rhs in 1..=MEMO_SYSTEMS {
            through(&mut memo, system(3, 4.0, 1.0, rhs as f64));
        }
        assert!(memo.solve(&mut system(3, 0.0, 1.0, 1.0)).is_err());
        for rhs in 1..=MEMO_SYSTEMS {
            let kept = system(3, 4.0, 1.0, rhs as f64);
            assert!(!through(&mut memo, kept).1, "{rhs} was evicted");
        }
    }

    #[test]
    fn a_system_above_the_cap_is_never_kept() {
        let mut memo = Memo::default();
        through(&mut memo, system(MEMO_UNKNOWNS, 4.0, 1.0, 1.0));
        assert_eq!(memo.systems.len(), 1);
        let mut memo = Memo::default();
        let solved = fresh(system(MEMO_UNKNOWNS + 1, 4.0, 1.0, 1.0));
        for _ in 0..2 {
            let x = through(&mut memo, system(MEMO_UNKNOWNS + 1, 4.0, 1.0, 1.0));
            assert_eq!(x, (solved.clone(), true));
            assert!(memo.systems.is_empty());
        }
    }

    #[test]
    fn the_memo_keeps_the_latest_systems() {
        let mut memo = Memo::default();
        for rhs in 1..=MEMO_SYSTEMS + 1 {
            through(&mut memo, system(3, 4.0, 1.0, rhs as f64));
        }
        assert_eq!(memo.systems.len(), MEMO_SYSTEMS);
        let latest = system(3, 4.0, 1.0, MEMO_SYSTEMS as f64 + 1.0);
        assert!(!through(&mut memo, latest).1, "the latest was not kept");
        assert!(
            through(&mut memo, system(3, 4.0, 1.0, 1.0)).1,
            "the oldest was kept"
        );
    }
}
