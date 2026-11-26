use crate::netlist::{NetId, Netlist, PartId};

use std::ops::Range;
use std::sync::Arc;

use crate::name::Ref;

use crate::chip;
use crate::element;
use crate::element::Device;
use crate::newton::{Conditions, Iterate};
use crate::sparse::{self, Factors, Failure, Pattern, Scratch, Shape};

pub(crate) const GMIN: f64 = 1e-12;

#[cfg(test)]
thread_local! {
    /// The systems this thread has solved, which a test reads to count Newton's solves.
    pub(crate) static SOLVES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Unknowns {
    pub internal_nodes: usize,
    /// The internal nodes take no node gmin. The part stamps the leakage its ngspice model has.
    pub bare_internal: bool,
    pub branches: usize,
    /// Junction voltages Newton limits. Slots in `Iterate::junctions`, not matrix rows.
    pub junctions: usize,
}

/// Where one part's unknowns start.
struct PartRows {
    internal: usize,
    branch: usize,
    branches: usize,
    junctions: Range<usize>,
    charges: usize,
    fluxes: usize,
}

pub(crate) struct Layout {
    net_rows: usize,
    part_rows: Vec<PartRows>,
    /// Ascending.
    node_rows: Vec<usize>,
    /// The node rows that take gmin, ascending.
    gmin_rows: Vec<usize>,
    /// The branch rows that carry a capacitor's current, ascending.
    charge_rows: Vec<usize>,
    size: usize,
    junctions: usize,
    /// The passive chips, ascending. A chip's index here is its slot in `Conditions::chips`.
    chips: Vec<PartId>,
}

/// A part's unknowns, and how many capacitors and inductors it owns.
fn shape(model: &impl Device) -> (Unknowns, usize, usize) {
    let extra = element::Device::unknowns(model);
    (
        extra,
        element::Device::charges(model).len(),
        element::Device::inductors(model).len(),
    )
}

impl Layout {
    /// `chips` are the passive chips in part order.
    pub(crate) fn new(netlist: &Netlist, chips: Vec<PartId>) -> Layout {
        assemble(netlist, chips)
    }
}

fn assemble(netlist: &Netlist, chips: Vec<PartId>) -> Layout {
    let mut row = 0;
    let mut junction = 0;
    let mut node_rows = Vec::new();
    let mut gmin_rows = Vec::new();
    let mut charge_rows = Vec::new();
    let net_rows = netlist.nets().len().saturating_sub(1);
    for _ in 0..net_rows {
        node_rows.push(row);
        gmin_rows.push(row);
        row += 1;
    }
    let mut part_rows = Vec::with_capacity(netlist.parts().len());
    for (_, part) in netlist.parts() {
        let (extra, charges, fluxes) = shape(&part.model);
        debug_assert!(
            charges + fluxes <= extra.branches,
            "a part's capacitors and inductors exceed its branches"
        );
        let internal = row;
        for _ in 0..extra.internal_nodes {
            node_rows.push(row);
            if !extra.bare_internal {
                gmin_rows.push(row);
            }
            row += 1;
        }
        let branch = row;
        charge_rows.extend(branch..branch + charges);
        row += extra.branches;
        let junctions = junction..junction + extra.junctions;
        junction = junctions.end;
        part_rows.push(PartRows {
            internal,
            branch,
            branches: extra.branches,
            junctions,
            charges,
            fluxes,
        });
    }
    Layout {
        net_rows,
        part_rows,
        node_rows,
        gmin_rows,
        charge_rows,
        size: row,
        junctions: junction,
        chips,
    }
}

impl Layout {
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
        let rows = &self.part_rows[part.index()];
        debug_assert!(k < rows.branches, "branch {k} is past this part's branches");
        rows.branch + k
    }

    /// The branch of `part`'s inductor `k`, after the part's capacitors.
    pub(crate) fn flux_branch(&self, part: PartId, k: usize) -> usize {
        let rows = &self.part_rows[part.index()];
        debug_assert!(k < rows.fluxes, "flux {k} is past this part's inductors");
        rows.charges + k
    }

    pub(crate) fn junction_slots(&self, part: PartId) -> Range<usize> {
        self.part_rows[part.index()].junctions.clone()
    }

    pub(crate) fn is_charge_row(&self, row: usize) -> bool {
        self.charge_rows.binary_search(&row).is_ok()
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
            let end = rows.branch + rows.branches;
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

/// A linear system on a sparse pattern, in the layout's own rows. A stamp that adds an entry
/// off the pattern keeps it aside, and `widen` moves it onto a wider pattern.
pub(crate) struct System {
    pattern: Arc<Pattern>,
    /// One per entry of `pattern`.
    values: Vec<f64>,
    b: Vec<f64>,
    /// Entries a stamp added off `pattern`.
    off: Vec<(usize, usize, f64)>,
}

impl System {
    /// The stamped coefficient of column `col` on row `row`. Tests read the regulator's input column.
    #[cfg(test)]
    pub(crate) fn term(&self, row: usize, col: usize) -> f64 {
        let on = self
            .pattern
            .slot(row, col)
            .map_or(0.0, |slot| self.values[slot]);
        let off = self
            .off
            .iter()
            .filter(|e| (e.0, e.1) == (row, col))
            .map(|e| e.2);
        on + off.sum::<f64>()
    }

    /// No entry yet. `reset` loads `gmin` before each stamp.
    pub(crate) fn new(layout: &Layout) -> System {
        System::on(Arc::new(Pattern::empty(layout.size())))
    }

    fn on(pattern: Arc<Pattern>) -> System {
        System {
            values: vec![0.0; pattern.len()],
            b: vec![0.0; pattern.n()],
            pattern,
            off: Vec::new(),
        }
    }

    /// Adds `value` to the coefficient of column `col` on row `row`.
    fn add(&mut self, row: usize, col: usize, value: f64) {
        match self.pattern.slot(row, col) {
            Some(slot) => self.values[slot] += value,
            None => self.off.push((row, col, value)),
        }
    }

    /// Adds `value` to row `row` of the right-hand side.
    fn add_rhs(&mut self, row: usize, value: f64) {
        self.b[row] += value;
    }

    /// Clears every value and stamps `gmin` on every net row and on each internal row that is
    /// not bare.
    pub(crate) fn reset(&mut self, layout: &Layout, gmin: f64) {
        self.values.fill(0.0);
        self.b.fill(0.0);
        self.off.clear();
        for &row in &layout.gmin_rows {
            self.add(row, row, gmin);
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
            self.add_rhs(row, amps);
        }
    }

    fn add_term(&mut self, row: Node, col: Node, value: f64) {
        let (Node::Row(row), Node::Row(col)) = (row, col) else {
            return;
        };
        self.add(row, col, value);
    }

    /// Moves every entry added off the pattern onto a pattern that has it.
    fn widen(&mut self) {
        if self.off.is_empty() {
            return;
        }
        let wider = self
            .pattern
            .with(self.off.iter().map(|&(row, col, _)| (row, col)));
        let mut values = vec![0.0; wider.len()];
        for col in 0..wider.n() {
            for slot in self.pattern.column_slots(col) {
                let row = self.pattern.row(slot);
                values[wider.slot(row, col).expect("a wider pattern")] += self.values[slot];
            }
        }
        for &(row, col, value) in &self.off {
            values[wider.slot(row, col).expect("a wider pattern")] += value;
        }
        self.pattern = Arc::new(wider);
        self.values = values;
        self.off.clear();
    }

    /// The first row whose value or right-hand side is not finite.
    fn nonfinite(&self) -> Option<usize> {
        let row = (0..self.pattern.n()).find(|&row| !self.b[row].is_finite());
        let entry = (0..self.pattern.n()).find_map(|col| {
            self.pattern
                .column_slots(col)
                .find(|&slot| !self.values[slot].is_finite())
                .map(|slot| self.pattern.row(slot))
        });
        match (row, entry) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Solves this system alone, on a pattern and factors made for it. Tests only.
    #[cfg(test)]
    pub(crate) fn solve(&mut self) -> Result<Solution, Failure> {
        let n = self.pattern.n();
        let mut solver = Solver {
            system: std::mem::replace(self, System::on(Arc::new(Pattern::empty(0)))),
            nodes: vec![false; n].into(),
            lus: Vec::new(),
            scratch: Scratch::default(),
            memo: Memo::default(),
            work: 0,
            solves: Solves::default(),
        };
        let solved = solver.solve(false);
        *self = solver.system;
        solved
    }
}

/// How many factorizations a solver keeps: one for a step's systems and one for a restart's,
/// whose held capacitors make other unknowns late.
const KEEP: usize = 2;

/// The factors of a system a solver factored, and the pattern they are for.
#[derive(Clone)]
struct Lu {
    pattern: Arc<Pattern>,
    shape: Arc<Shape>,
    factors: Factors,
}

/// A circuit's system, and what one solve keeps for the next: the pattern its stamps have
/// filled, the column order and pivots of its factors, and the memo. A run steps on one
/// solver. A row's probe takes a [`Solver::scratch`] copy, so nothing a probe does reaches the
/// run's solves.
pub(crate) struct Solver {
    system: System,
    /// Whether each unknown is a node's voltage, which the column order reads.
    nodes: Arc<[bool]>,
    /// The latest factorizations, the newest first, each for its own late unknowns.
    lus: Vec<Lu>,
    scratch: Scratch,
    memo: Memo,
    /// The multiply-adds of every factorization and solve so far.
    pub(crate) work: u64,
    /// How each system so far was solved.
    pub(crate) solves: Solves,
}

/// How a solver's systems were solved. Each is one Newton iteration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Solves {
    /// The memo held the system and its solution.
    pub(crate) memo: u64,
    /// Refactored on the pivots of kept factors.
    pub(crate) refactored: u64,
    /// Factored again with pivoting on a kept column order, because a kept pivot no longer passed.
    pub(crate) repivoted: u64,
    /// Factored on a fresh column order: a new pattern or new late unknowns.
    pub(crate) reordered: u64,
}

impl Solves {
    pub(crate) fn total(&self) -> u64 {
        self.memo + self.refactored + self.repivoted + self.reordered
    }
}

impl Solver {
    pub(crate) fn new(layout: &Layout) -> Solver {
        Solver {
            system: System::new(layout),
            nodes: (0..layout.size())
                .map(|row| layout.is_node_row(row))
                .collect(),
            lus: Vec::new(),
            scratch: Scratch::default(),
            memo: Memo::default(),
            work: 0,
            solves: Solves::default(),
        }
    }

    /// This solver's pattern and factors, with an empty memo and no work or solves counted.
    pub(crate) fn scratch(&self) -> Solver {
        Solver {
            system: System::on(Arc::clone(&self.system.pattern)),
            nodes: Arc::clone(&self.nodes),
            lus: self.lus.clone(),
            scratch: Scratch::default(),
            memo: Memo::default(),
            work: 0,
            solves: Solves::default(),
        }
    }

    /// The system, cleared and loaded with `gmin` for the next stamp.
    pub(crate) fn reset(&mut self, layout: &Layout, gmin: f64) -> &mut System {
        self.system.reset(layout, gmin);
        &mut self.system
    }

    /// The stamped system's solution. A value that is not finite is `NonFinite` at its row.
    /// When `remember`, a system kept in the memo has the solution kept with it, and any other
    /// is kept. A system on the pattern and with the late unknowns of kept factors is
    /// refactored on their pivots. When a pivot no longer passes it is factored again with
    /// pivoting, and those factors replace them. Any other system is factored on a fresh order,
    /// and its factors are kept in place of the oldest.
    pub(crate) fn solve(&mut self, remember: bool) -> Result<Solution, Failure> {
        self.system.widen();
        if let Some(row) = self.system.nonfinite() {
            return Err(Failure::NonFinite { col: row });
        }
        if remember && let Some(x) = self.memo.find(&self.system) {
            self.solves.memo += 1;
            return Ok(x);
        }
        #[cfg(test)]
        SOLVES.with(|solves| solves.set(solves.get() + 1));
        let system = &self.system;
        let late = system.pattern.late(&system.values, &self.nodes);
        let kept = self
            .lus
            .iter()
            .position(|lu| Arc::ptr_eq(&lu.pattern, &system.pattern) && lu.shape.late() == late);
        let refactored = match kept {
            Some(k) => {
                let lu = &mut self.lus[k];
                sparse::refactor(
                    &lu.shape,
                    &system.pattern,
                    &system.values,
                    &mut lu.factors,
                    &mut self.scratch,
                    &mut self.work,
                )
            }
            None => Err(sparse::Refused::Pivot),
        };
        let k = match (refactored, kept) {
            (Ok(()), Some(k)) => {
                self.solves.refactored += 1;
                k
            }
            (Err(sparse::Refused::NonFinite { col }), _) => {
                self.solves.refactored += 1;
                return Err(Failure::NonFinite { col });
            }
            (_, kept) => {
                match kept {
                    Some(_) => self.solves.repivoted += 1,
                    None => self.solves.reordered += 1,
                }
                let order = match kept {
                    Some(k) => self.lus[k].shape.order(),
                    None => system.pattern.order(late),
                };
                let (shape, factors) = sparse::factor(
                    &system.pattern,
                    &system.values,
                    order,
                    &mut self.scratch,
                    &mut self.work,
                )?;
                let lu = Lu {
                    pattern: Arc::clone(&system.pattern),
                    shape: Arc::new(shape),
                    factors,
                };
                match kept {
                    Some(k) => {
                        self.lus[k] = lu;
                        k
                    }
                    None => {
                        self.lus.truncate(KEEP - 1);
                        self.lus.insert(0, lu);
                        0
                    }
                }
            }
        };
        let lu = &self.lus[k];
        let x = Solution::from(
            lu.factors
                .solve(&lu.shape, &self.system.b, &mut self.work)?,
        );
        if remember {
            self.memo.keep(&self.system, &x);
        }
        Ok(x)
    }
}

/// How many systems a `Memo` keeps.
const MEMO_SYSTEMS: usize = 8;

/// The most bytes the systems a `Memo` keeps may take, 4 MiB. A system larger than its share is
/// never kept.
const MEMO_BYTES: usize = 4 << 20;

/// The latest systems solved, each with its solution. The key is the whole system, its pattern,
/// values and right-hand side, so nothing goes stale. A source step, a switch toggle, a pin's
/// new drive, a live `set`, another step length or another held voltage changes a stamped
/// value, which is a different key, or stamps the same system again, whose solution has not
/// changed.
#[derive(Default)]
struct Memo {
    /// Most recent first.
    systems: Vec<Kept>,
}

struct Kept {
    pattern: Arc<Pattern>,
    values: Vec<f64>,
    b: Vec<f64>,
    x: Solution,
}

impl Memo {
    /// The solution kept with `system`, which then becomes the most recent.
    fn find(&mut self, system: &System) -> Option<Solution> {
        let k = self.systems.iter().position(|kept| {
            Arc::ptr_eq(&kept.pattern, &system.pattern)
                && same_bits(&kept.b, &system.b)
                && same_bits(&kept.values, &system.values)
        })?;
        let kept = self.systems.remove(k);
        let x = kept.x.clone();
        self.systems.insert(0, kept);
        Some(x)
    }

    /// Keeps `system` with its solution `x`, dropping the oldest past `MEMO_SYSTEMS`.
    fn keep(&mut self, system: &System, x: &Solution) {
        let bytes = (system.values.len() + 2 * system.b.len()) * size_of::<f64>();
        if bytes * MEMO_SYSTEMS > MEMO_BYTES {
            return;
        }
        if self.systems.len() == MEMO_SYSTEMS {
            self.systems.pop();
        }
        self.systems.insert(
            0,
            Kept {
                pattern: Arc::clone(&system.pattern),
                values: system.values.clone(),
                b: system.b.clone(),
                x: x.clone(),
            },
        );
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

    #[cfg(test)]
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
    /// Read through `junction` or `prior`. A stamp that linearises at the last solution
    /// calls `prior` by name.
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

    /// The last solution. A branch law linearised here is not the law at the zero start.
    pub(crate) fn prior(&self) -> Probe<'_> {
        self.probe(&self.iterate.solution)
    }

    /// The limited voltage of this part's junction `k`, from the iterate.
    pub(crate) fn junction(&self, k: usize) -> f64 {
        self.iterate.junctions[self.rows().junctions.start + k]
    }

    pub(crate) fn conductance(&mut self, a: impl Into<Node>, b: impl Into<Node>, g: f64) {
        self.sys.conductance(a, b, g);
    }

    /// One matrix term, the slope of the current leaving `row` on the voltage at `col`. A
    /// controlled source's current depends on nodes it does not flow between.
    pub(crate) fn add(&mut self, row: impl Into<Node>, col: impl Into<Node>, g: f64) {
        self.sys.add_term(row.into(), col.into(), g);
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
    pub(crate) fn branch_current(&mut self, p: impl Into<Node>, n: impl Into<Node>, k: usize) {
        let row = self.rows().branch + k;
        if let Node::Row(prow) = p.into() {
            self.sys.add(prow, row, 1.0);
        }
        if let Node::Row(nrow) = n.into() {
            self.sys.add(nrow, row, -1.0);
        }
    }

    /// Branch `k`'s own row: `ci * i_k + cv * (V(p) - V(n)) = rhs`.
    pub(crate) fn branch_row(
        &mut self,
        p: impl Into<Node>,
        n: impl Into<Node>,
        k: usize,
        ci: f64,
        cv: f64,
        rhs: f64,
    ) {
        let row = self.rows().branch + k;
        if ci != 0.0 {
            self.sys.add(row, row, ci);
        }
        if let Node::Row(prow) = p.into() {
            self.sys.add(row, prow, cv);
        }
        if let Node::Row(nrow) = n.into() {
            self.sys.add(row, nrow, -cv);
        }
        self.sys.add_rhs(row, rhs);
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
    pub(crate) fn v(&self, node: impl Into<Node>) -> f64 {
        self.x.node(node.into())
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
    fn stamped(solver: &mut Solver, n: usize, diagonal: f64, above: f64, rhs: f64) {
        let system = &mut solver.system;
        system.values.fill(0.0);
        system.off.clear();
        system.b = vec![rhs; n];
        for k in 0..n {
            system.add(k, k, diagonal);
            if k + 1 < n {
                system.add(k, k + 1, above);
            }
        }
    }

    fn solver(n: usize) -> Solver {
        Solver {
            system: System::on(Arc::new(Pattern::empty(n))),
            nodes: vec![false; n].into(),
            lus: Vec::new(),
            scratch: Scratch::default(),
            memo: Memo::default(),
            work: 0,
            solves: Solves::default(),
        }
    }

    fn bits(solution: &Solution) -> Vec<u64> {
        solution.components().iter().map(|v| v.to_bits()).collect()
    }

    /// The solution's bits, and whether the solver factored to get them.
    fn through(
        solver: &mut Solver,
        n: usize,
        diagonal: f64,
        above: f64,
        rhs: f64,
    ) -> (Vec<u64>, bool) {
        stamped(solver, n, diagonal, above, rhs);
        let before = solver.work;
        let x = solver.solve(true).unwrap();
        (bits(&x), solver.work > before)
    }

    #[test]
    fn a_kept_system_returns_its_solution_without_solving() {
        let mut solver = solver(3);
        let (first, ran) = through(&mut solver, 3, 4.0, 1.0, 1.0);
        assert!(ran);
        assert_eq!(through(&mut solver, 3, 4.0, 1.0, 1.0), (first, false));
    }

    /// Each one shares the other two parts with the kept system: the right-hand side, the
    /// diagonal, and the entries off it.
    #[test]
    fn a_system_that_differs_in_any_bit_is_solved() {
        for (diagonal, above, rhs) in [(4.0, 1.0, 2.0), (5.0, 1.0, 1.0), (4.0, 2.0, 1.0)] {
            let mut solver = solver(3);
            let (kept, _) = through(&mut solver, 3, 4.0, 1.0, 1.0);
            let (x, ran) = through(&mut solver, 3, diagonal, above, rhs);
            assert!(ran && x != kept, "{x:?}");
        }
    }

    #[test]
    fn a_singular_system_is_not_kept() {
        let mut solver = solver(3);
        stamped(&mut solver, 3, 0.0, 1.0, 1.0);
        assert!(solver.solve(true).is_err());
        assert!(solver.memo.systems.is_empty());
    }

    #[test]
    fn the_memo_keeps_the_latest_systems() {
        let mut solver = solver(3);
        for rhs in 1..=MEMO_SYSTEMS + 1 {
            through(&mut solver, 3, 4.0, 1.0, rhs as f64);
        }
        assert_eq!(solver.memo.systems.len(), MEMO_SYSTEMS);
        let latest = MEMO_SYSTEMS as f64 + 1.0;
        assert!(
            !through(&mut solver, 3, 4.0, 1.0, latest).1,
            "the latest was not kept"
        );
        assert!(
            through(&mut solver, 3, 4.0, 1.0, 1.0).1,
            "the oldest was kept"
        );
    }

    /// 1000 unknowns, far past what a dense memo held, are kept.
    #[test]
    fn a_large_sparse_system_is_kept() {
        let mut solver = solver(1000);
        assert!(through(&mut solver, 1000, 4.0, 1.0, 1.0).1);
        assert!(!through(&mut solver, 1000, 4.0, 1.0, 1.0).1);
    }

    /// A system whose pattern grew is factored on the wider pattern, and the next system on it
    /// refactors on the kept pivots, to the same bits as a solver that never had the narrower
    /// pattern.
    #[test]
    fn a_wider_pattern_is_factored_again_and_then_reused() {
        let mut grown = solver(3);
        grown.system.b = vec![1.0; 3];
        for k in 0..3 {
            grown.system.add(k, k, 4.0);
        }
        grown.solve(true).unwrap();
        let narrow = Arc::clone(&grown.system.pattern);
        let (wide, ran) = through(&mut grown, 3, 4.0, 1.0, 2.0);
        assert!(ran && !Arc::ptr_eq(&narrow, &grown.system.pattern));
        let mut fresh = solver(3);
        through(&mut fresh, 3, 4.0, 1.0, 5.0);
        assert_eq!(through(&mut fresh, 3, 4.0, 1.0, 2.0).0, wide);
    }

    #[test]
    fn a_value_that_is_not_finite_names_its_row() {
        let mut solver = solver(3);
        stamped(&mut solver, 3, 4.0, 1.0, 1.0);
        solver.system.b[2] = f64::INFINITY;
        assert_eq!(
            solver.solve(false).unwrap_err(),
            Failure::NonFinite { col: 2 }
        );
    }
}
