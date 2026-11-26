//! Sparse LU. A system's pattern is analysed once: a column order that keeps the factors
//! sparse, then a factorization that picks each pivot and fixes where L and U have entries.
//! Every later system on that pattern is refactored on those pivots, so a Newton iteration costs
//! the factors' entries, not n². Pure: the same pattern, pivots and values give the same bits
//! on every target.

use std::collections::BTreeSet;

/// The most unknowns a circuit may have. Above it the circuit is `solver.too_large` before
/// anything is allocated.
pub(crate) const MAX_UNKNOWNS: usize = 4096;

/// A pivot must be at least this share of the largest entry left in its column. The diagonal
/// is taken when it passes, which keeps the factors as sparse as the column order made them,
/// and each multiplier stays under 1 / `THRESHOLD`.
const THRESHOLD: f64 = 1e-3;

/// The most unknowns a pattern keeps a dense slot table for, 4 MiB of slots. A larger
/// pattern finds each slot by a binary search of its column.
const DENSE_UNKNOWNS: usize = 1024;

/// A dense table's mark for a row and column with no entry.
const NO_SLOT: u32 = u32::MAX;

/// Where a system has entries: for each column, its rows, ascending.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pattern {
    n: usize,
    /// Column `j`'s rows are `rows[start[j]..start[j + 1]]`.
    start: Vec<usize>,
    rows: Vec<usize>,
    /// For at most `DENSE_UNKNOWNS` unknowns, the slot of row `i` in column `j` at `j * n + i`,
    /// or `NO_SLOT`. Made from `start` and `rows`, so it finds the slot the search would.
    dense: Option<Box<[u32]>>,
}

impl Pattern {
    /// The pattern with `n` unknowns and these columns, with its dense table.
    fn new(n: usize, start: Vec<usize>, rows: Vec<usize>) -> Pattern {
        let dense = (n <= DENSE_UNKNOWNS).then(|| {
            let mut table = vec![NO_SLOT; n * n].into_boxed_slice();
            for col in 0..n {
                for slot in start[col]..start[col + 1] {
                    table[col * n + rows[slot]] =
                        u32::try_from(slot).expect("a dense pattern has under 2^32 entries");
                }
            }
            table
        });
        Pattern {
            n,
            start,
            rows,
            dense,
        }
    }

    /// Whether each row has one entry.
    fn single_rows(&self) -> Vec<bool> {
        let mut entries = vec![0usize; self.n];
        for &row in &self.rows {
            entries[row] += 1;
        }
        entries.into_iter().map(|count| count == 1).collect()
    }

    /// `n` unknowns and no entry.
    pub(crate) fn empty(n: usize) -> Pattern {
        Pattern::new(n, vec![0; n + 1], Vec::new())
    }

    pub(crate) fn n(&self) -> usize {
        self.n
    }

    /// How many entries it has.
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    /// The index of the entry at `row` and `col` in a value list on this pattern.
    #[inline]
    pub(crate) fn slot(&self, row: usize, col: usize) -> Option<usize> {
        if let Some(table) = &self.dense {
            let slot = table[col * self.n + row];
            return (slot != NO_SLOT).then_some(slot as usize);
        }
        self.search(row, col)
    }

    /// `slot`, by a binary search of the column.
    fn search(&self, row: usize, col: usize) -> Option<usize> {
        let (from, to) = (self.start[col], self.start[col + 1]);
        self.rows[from..to]
            .binary_search(&row)
            .ok()
            .map(|k| from + k)
    }

    /// This pattern with every entry of `more` added.
    pub(crate) fn with(&self, more: impl IntoIterator<Item = (usize, usize)>) -> Pattern {
        let mut columns: Vec<BTreeSet<usize>> = (0..self.n)
            .map(|col| self.column(col).iter().copied().collect())
            .collect();
        for (row, col) in more {
            columns[col].insert(row);
        }
        let mut start = Vec::with_capacity(self.n + 1);
        let mut rows = Vec::new();
        start.push(0);
        for column in columns {
            rows.extend(column);
            start.push(rows.len());
        }
        Pattern::new(self.n, start, rows)
    }

    /// The slots of column `col`'s entries.
    pub(crate) fn column_slots(&self, col: usize) -> std::ops::Range<usize> {
        self.start[col]..self.start[col + 1]
    }

    /// The row of the entry at `slot`.
    pub(crate) fn row(&self, slot: usize) -> usize {
        self.rows[slot]
    }

    fn column(&self, col: usize) -> &[usize] {
        &self.rows[self.start[col]..self.start[col + 1]]
    }

    /// The columns in minimum degree order, on the graph of the pattern plus its transpose.
    ///
    /// A column that a row with one entry pins comes first, in index order. That row is a
    /// source's constraint, and eliminating on it fills nothing, so its unknown is the source's
    /// value before any other unknown is solved from it. A held capacitor beside the source,
    /// behind `1e-9` ohm, then reads its current from the exact voltage.
    ///
    /// Each later step takes the unknown coupled to the fewest others, ties to the lowest index,
    /// and couples its neighbours to each other, as eliminating it would. A net that many parts
    /// touch, such as a supply rail, then comes after the nets it couples, and eliminating it
    /// fills almost nothing.
    ///
    /// A `late` unknown waits until the unknowns it is coupled to at first are taken, and its
    /// row is preferred as a pivot. Each of those is eliminated on the late row, and the late
    /// unknown then has its node's row left to pivot on, as a node of a chain would.
    pub(crate) fn order(&self, late: Vec<bool>) -> Order {
        let single = self.single_rows();
        let pinned: Vec<bool> = (0..self.n)
            .map(|col| self.column(col).iter().any(|&row| single[row]))
            .collect();
        let mut adjacent: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); self.n];
        for col in 0..self.n {
            for &row in self.column(col) {
                if row != col && !pinned[row] && !pinned[col] {
                    adjacent[row].insert(col);
                    adjacent[col].insert(row);
                }
            }
        }
        // The unknowns each late one waits for.
        let first: Vec<BTreeSet<usize>> = (0..self.n)
            .map(|col| match late[col] {
                true => adjacent[col].clone(),
                false => BTreeSet::new(),
            })
            .collect();
        let mut waiting: Vec<usize> = first.iter().map(BTreeSet::len).collect();
        let key = |col: usize, waiting: &[usize], adjacent: &[BTreeSet<usize>]| {
            (waiting[col] > 0, adjacent[col].len(), col)
        };
        let mut order: Vec<usize> = (0..self.n).filter(|&col| pinned[col]).collect();
        let mut queue: BTreeSet<(bool, usize, usize)> = (0..self.n)
            .filter(|&col| !pinned[col])
            .map(|col| key(col, &waiting, &adjacent))
            .collect();
        while let Some((_, _, node)) = queue.pop_first() {
            order.push(node);
            let neighbours: Vec<usize> = std::mem::take(&mut adjacent[node]).into_iter().collect();
            for &a in &neighbours {
                queue.remove(&key(a, &waiting, &adjacent));
                adjacent[a].remove(&node);
                if first[a].contains(&node) {
                    waiting[a] -= 1;
                }
            }
            for (k, &a) in neighbours.iter().enumerate() {
                for &b in &neighbours[k + 1..] {
                    adjacent[a].insert(b);
                    adjacent[b].insert(a);
                }
            }
            for &a in &neighbours {
                queue.insert(key(a, &waiting, &adjacent));
            }
        }
        Order {
            columns: order,
            late,
        }
    }
}

impl Pattern {
    /// Whether each unknown is late: not a node's voltage, by `node`, with its diagonal in
    /// `values` under `THRESHOLD` of its column's largest entry. That is a held capacitor's
    /// current, behind `HOLD / C` ohm. Two held in parallel split their current by those tiny
    /// resistances, which only the difference of their rows carries, so the voltage they share
    /// is eliminated after the others, on one of their rows.
    pub(crate) fn late(&self, values: &[f64], node: &[bool]) -> Vec<bool> {
        (0..self.n)
            .map(|col| {
                !node[col]
                    && self.slot(col, col).is_some_and(|diagonal| {
                        let largest = self
                            .column_slots(col)
                            .map(|slot| values[slot].abs())
                            .fold(0.0, f64::max);
                        values[diagonal].abs() < THRESHOLD * largest
                    })
            })
            .collect()
    }
}

/// A column order, and which unknowns are late, whose rows are preferred as pivots.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Order {
    pub(crate) columns: Vec<usize>,
    late: Vec<bool>,
}

#[cfg(test)]
impl Order {
    /// `columns`, with no unknown late.
    fn plain(columns: Vec<usize>) -> Self {
        let late = vec![false; columns.len()];
        Self { columns, late }
    }
}

/// Why a system has no factorization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// The unknown `col` has no pivot: every entry left in its column is zero, or within the
    /// rounding of the sums that made it.
    Singular { col: usize },
    /// A factor or the solution of unknown `col` overflowed.
    NonFinite { col: usize },
}

/// Where L and U have entries, and the pivot of each step. Fixed by a factorization with
/// pivoting, then shared by every refactorization on it.
#[derive(Debug)]
pub(crate) struct Shape {
    /// Step `k` eliminates unknown `order[k]`.
    order: Vec<usize>,
    /// The row step `k` pivots on.
    pivot: Vec<usize>,
    /// L's column `k` has rows `l_rows[l_start[k]..l_start[k + 1]]`, each pivoted after `k`.
    l_start: Vec<usize>,
    l_rows: Vec<usize>,
    /// U's column `k` has the steps `u_steps[u_start[k]..u_start[k + 1]]` above its diagonal,
    /// in an order that solves them.
    u_start: Vec<usize>,
    u_steps: Vec<usize>,
    /// Whether each row has one entry in the pattern.
    single: Vec<bool>,
    /// Whether each row is a late unknown's.
    late: Vec<bool>,
}

impl Shape {
    /// Whether each unknown is late in its order.
    pub(crate) fn late(&self) -> &[bool] {
        &self.late
    }

    /// The column order it eliminates in.
    pub(crate) fn order(&self) -> Order {
        Order {
            columns: self.order.clone(),
            late: self.late.clone(),
        }
    }
}

/// The values of L and U on a shape.
#[derive(Clone, Debug)]
pub(crate) struct Factors {
    l: Vec<f64>,
    u: Vec<f64>,
    diagonal: Vec<f64>,
}

/// Work space as long as the system, kept zero between columns.
#[derive(Clone, Debug, Default)]
pub(crate) struct Scratch {
    x: Vec<f64>,
    /// The sum of the magnitudes that made each entry of `x`, which bounds its rounding.
    bound: Vec<f64>,
    /// How many terms made each entry of `x`.
    terms: Vec<u32>,
    /// The last step whose search reached each row, plus one.
    seen: Vec<usize>,
    /// Each row's sum of magnitudes in the system being factored, which [`choose`] weighs a
    /// candidate against.
    scale: Vec<f64>,
}

impl Scratch {
    fn fit(&mut self, n: usize) {
        if self.x.len() != n {
            self.x = vec![0.0; n];
            self.bound = vec![0.0; n];
            self.terms = vec![0; n];
            self.seen = vec![0; n];
            self.scale = vec![0.0; n];
        }
    }

    /// Each row's sum of magnitudes in `values` on `pattern`.
    fn scale(&mut self, pattern: &Pattern, values: &[f64]) {
        self.scale.fill(0.0);
        for (&row, &value) in pattern.rows.iter().zip(values) {
            self.scale[row] += value.abs();
        }
    }

    /// Clears what a column left at `rows`.
    fn clear(&mut self, rows: impl Iterator<Item = usize>) {
        for row in rows {
            self.x[row] = 0.0;
            self.bound[row] = 0.0;
            self.terms[row] = 0;
        }
    }
}

/// Whether `x`, made of `terms` terms whose magnitudes sum to `bound`, is within their rounding
/// of zero. Each term rounds by at most one unit in the last place of the sum, so `terms`
/// machine epsilons of `bound` covers it.
fn rounding_zero(x: f64, bound: f64, terms: u32) -> bool {
    x.abs() <= f64::from(terms) * f64::EPSILON * bound
}

/// Factors `values` on `pattern` in the column `order`, choosing each pivot: the diagonal when it
/// is at least `THRESHOLD` of the largest candidate, else the largest against its own row's
/// sum of magnitudes, ties to the lowest row.
/// A candidate within its rounding of zero is never chosen. `work` counts the multiply-adds.
pub(crate) fn factor(
    pattern: &Pattern,
    values: &[f64],
    order: Order,
    scratch: &mut Scratch,
    work: &mut u64,
) -> Result<(Shape, Factors), Failure> {
    let n = pattern.n;
    scratch.fit(n);
    scratch.scale(pattern, values);
    scratch.seen.fill(0);
    let mut pivoted = vec![usize::MAX; n];
    let mut shape = Shape {
        pivot: Vec::with_capacity(n),
        l_start: vec![0],
        l_rows: Vec::new(),
        u_start: vec![0],
        u_steps: Vec::new(),
        order: order.columns,
        single: pattern.single_rows(),
        late: order.late,
    };
    let mut factors = Factors {
        l: Vec::new(),
        u: Vec::new(),
        diagonal: Vec::with_capacity(n),
    };
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut finished: Vec<usize> = Vec::new();
    let mut candidates: Vec<usize> = Vec::new();
    for k in 0..n {
        let col = shape.order[k];
        let entries = pattern.start[col]..pattern.start[col + 1];
        // Which rows column `col` reaches through the columns of L already made: the pivoted
        // ones in an order that solves them, and the rest, which are its candidates.
        finished.clear();
        candidates.clear();
        for &root in &pattern.rows[entries.clone()] {
            if scratch.seen[root] == k + 1 {
                continue;
            }
            scratch.seen[root] = k + 1;
            stack.push((root, 0));
            while let Some(top) = stack.last_mut() {
                let (row, next) = *top;
                let step = pivoted[row];
                if step == usize::MAX {
                    candidates.push(row);
                    stack.pop();
                    continue;
                }
                let below = &shape.l_rows[shape.l_start[step]..shape.l_start[step + 1]];
                if let Some(&child) = below.get(next) {
                    top.1 += 1;
                    if scratch.seen[child] != k + 1 {
                        scratch.seen[child] = k + 1;
                        stack.push((child, 0));
                    }
                } else {
                    finished.push(step);
                    stack.pop();
                }
            }
        }
        for (&row, &value) in pattern.rows[entries.clone()].iter().zip(&values[entries]) {
            scratch.x[row] = value;
            scratch.bound[row] = value.abs();
            scratch.terms[row] = 1;
        }
        for &step in finished.iter().rev() {
            let above = scratch.x[shape.pivot[step]];
            shape.u_steps.push(step);
            factors.u.push(above);
            eliminate(&shape, &factors, step, above, scratch, work);
        }
        shape.u_start.push(shape.u_steps.len());
        candidates.sort_unstable();
        let chosen = choose(col, &candidates, &shape, scratch);
        let pivot = match chosen {
            Ok(pivot) => pivot,
            Err(failure) => {
                let touched = finished.iter().map(|&step| shape.pivot[step]);
                scratch.clear(touched.chain(candidates.iter().copied()));
                return Err(failure);
            }
        };
        let diagonal = scratch.x[pivot];
        pivoted[pivot] = k;
        shape.pivot.push(pivot);
        factors.diagonal.push(diagonal);
        for &row in &candidates {
            if row != pivot {
                shape.l_rows.push(row);
                factors.l.push(scratch.x[row] / diagonal);
            }
        }
        shape.l_start.push(shape.l_rows.len());
        let touched = finished.iter().map(|&step| shape.pivot[step]);
        scratch.clear(touched.chain(candidates.iter().copied()));
    }
    Ok((shape, factors))
}

/// The pivot among `candidates`, the rows column `col` has left, ascending. Of those at least
/// `THRESHOLD` of the largest, it is a row with one entry, then a late unknown's row, then the
/// diagonal, then the largest against its row's sum of magnitudes in the system, then the
/// lowest row. Against its row, a source's current is solved from the net whose other entries
/// are smallest. Its 1 in a supply net's row beside 1e5 S would make it a difference of terms
/// that size and carry their rounding, far over Newton's 1 pA on a current near 0. A rail whose
/// entries are the 1s of other sources' and capacitors' currents sums over a light net's 1, where
/// its largest entry would tie and the tie would go to the rail. A row with one entry is a source's
/// constraint, so its unknown comes out of the solve as the source's value, with no rounding.
/// A late row is a held capacitor's, so the voltage it holds comes out of that row.
/// A candidate within its rounding of zero is never chosen. A candidate that overflowed is
/// `NonFinite`.
fn choose(
    col: usize,
    candidates: &[usize],
    shape: &Shape,
    scratch: &Scratch,
) -> Result<usize, Failure> {
    let rank = |row: usize| (shape.single[row], shape.late[row]);
    let scaled = |row: usize| scratch.x[row].abs() / scratch.scale[row];
    if candidates.iter().any(|&row| !scratch.x[row].is_finite()) {
        return Err(Failure::NonFinite { col });
    }
    let usable = |row: &&usize| {
        let x = scratch.x[**row];
        x != 0.0 && !rounding_zero(x, scratch.bound[**row], scratch.terms[**row])
    };
    let largest = candidates
        .iter()
        .filter(usable)
        .map(|&row| scratch.x[row].abs())
        .fold(0.0f64, f64::max);
    candidates
        .iter()
        .filter(usable)
        .filter(|&&row| scratch.x[row].abs() >= THRESHOLD * largest)
        .min_by(|&&a, &&b| {
            rank(b)
                .cmp(&rank(a))
                .then((b == col).cmp(&(a == col)))
                .then(scaled(b).total_cmp(&scaled(a)))
                .then(a.cmp(&b))
        })
        .copied()
        .ok_or(Failure::Singular { col })
}

/// Subtracts L's column `step` times `above` from the rows below that step's pivot.
fn eliminate(
    shape: &Shape,
    factors: &Factors,
    step: usize,
    above: f64,
    scratch: &mut Scratch,
    work: &mut u64,
) {
    let span = shape.l_start[step]..shape.l_start[step + 1];
    for (&row, &l) in shape.l_rows[span.clone()]
        .iter()
        .zip(&factors.l[span.clone()])
    {
        let product = l * above;
        scratch.x[row] -= product;
        scratch.bound[row] += product.abs();
        scratch.terms[row] += 1;
    }
    *work += span.len() as u64;
}

/// Why a refactorization stopped.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    /// A fresh factorization would pivot on another row in some column, or on none. It may
    /// still succeed.
    Pivot,
    NonFinite {
        col: usize,
    },
}

/// Factors `values` on `pattern` with the pivots and the shape of a factorization of an earlier
/// system on the same pattern, into `factors`. Each column checks that [`factor`] would choose
/// the same pivot from the same candidates, so the factors are the bits a fresh factorization
/// gives, and a solve does not depend on the systems solved before it.
pub(crate) fn refactor(
    shape: &Shape,
    pattern: &Pattern,
    values: &[f64],
    factors: &mut Factors,
    scratch: &mut Scratch,
    work: &mut u64,
) -> Result<(), Refused> {
    let n = pattern.n;
    scratch.fit(n);
    scratch.scale(pattern, values);
    let mut candidates = Vec::new();
    for k in 0..n {
        let col = shape.order[k];
        let entries = pattern.start[col]..pattern.start[col + 1];
        for (&row, &value) in pattern.rows[entries.clone()].iter().zip(&values[entries]) {
            scratch.x[row] = value;
            scratch.bound[row] = value.abs();
            scratch.terms[row] = 1;
        }
        let ups = shape.u_start[k]..shape.u_start[k + 1];
        for slot in ups.clone() {
            let step = shape.u_steps[slot];
            let above = scratch.x[shape.pivot[step]];
            factors.u[slot] = above;
            eliminate(shape, factors, step, above, scratch, work);
        }
        let pivot = shape.pivot[k];
        let lows = shape.l_start[k]..shape.l_start[k + 1];
        candidates.clear();
        candidates.push(pivot);
        candidates.extend_from_slice(&shape.l_rows[lows.clone()]);
        let result = match choose(col, &candidates, shape, scratch) {
            Ok(chosen) if chosen == pivot => {
                let diagonal = scratch.x[pivot];
                factors.diagonal[k] = diagonal;
                for slot in lows.clone() {
                    factors.l[slot] = scratch.x[shape.l_rows[slot]] / diagonal;
                }
                Ok(())
            }
            Ok(_) | Err(Failure::Singular { .. }) => Err(Refused::Pivot),
            Err(Failure::NonFinite { col }) => Err(Refused::NonFinite { col }),
        };
        let touched = shape.u_steps[ups].iter().map(|&step| shape.pivot[step]);
        scratch.clear(
            touched
                .chain(std::iter::once(pivot))
                .chain(shape.l_rows[lows].iter().copied()),
        );
        result?;
    }
    Ok(())
}

impl Factors {
    /// The solution of the factored system with right-hand side `b`, in the system's own
    /// unknowns. `work` counts the multiply-adds.
    pub(crate) fn solve(
        &self,
        shape: &Shape,
        b: &[f64],
        work: &mut u64,
    ) -> Result<Vec<f64>, Failure> {
        let n = b.len();
        let mut rest = b.to_vec();
        let mut y = vec![0.0; n];
        for k in 0..n {
            let value = rest[shape.pivot[k]];
            y[k] = value;
            let span = shape.l_start[k]..shape.l_start[k + 1];
            for (&row, &l) in shape.l_rows[span.clone()].iter().zip(&self.l[span.clone()]) {
                rest[row] -= l * value;
            }
            *work += span.len() as u64;
        }
        for k in (0..n).rev() {
            let value = y[k] / self.diagonal[k];
            y[k] = value;
            let span = shape.u_start[k]..shape.u_start[k + 1];
            for (&step, &u) in shape.u_steps[span.clone()]
                .iter()
                .zip(&self.u[span.clone()])
            {
                y[step] -= u * value;
            }
            *work += span.len() as u64 + 1;
        }
        let mut x = vec![0.0; n];
        for (k, &col) in shape.order.iter().enumerate() {
            if !y[k].is_finite() {
                return Err(Failure::NonFinite { col });
            }
            x[col] = y[k];
        }
        Ok(x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dense system as a pattern and values, from rows of `(col, value)`.
    fn sparse(n: usize, entries: &[(usize, usize, f64)]) -> (Pattern, Vec<f64>) {
        let pattern = Pattern::empty(n).with(entries.iter().map(|&(r, c, _)| (r, c)));
        let mut values = vec![0.0; pattern.len()];
        for &(r, c, v) in entries {
            values[pattern.slot(r, c).unwrap()] += v;
        }
        (pattern, values)
    }

    fn solve(n: usize, entries: &[(usize, usize, f64)], b: &[f64]) -> Result<Vec<f64>, Failure> {
        let (pattern, values) = sparse(n, entries);
        let mut scratch = Scratch::default();
        let mut work = 0;
        let (shape, factors) = factor(
            &pattern,
            &values,
            pattern.order(pattern.late(&values, &vec![false; pattern.n()])),
            &mut scratch,
            &mut work,
        )?;
        factors.solve(&shape, b, &mut work)
    }

    /// Gaussian elimination with partial pivoting on a dense copy.
    fn reference(n: usize, entries: &[(usize, usize, f64)], b: &[f64]) -> Vec<f64> {
        let mut a = vec![vec![0.0; n]; n];
        for &(r, c, v) in entries {
            a[r][c] += v;
        }
        let mut b = b.to_vec();
        for k in 0..n {
            let p = (k..n)
                .max_by(|&i, &j| a[i][k].abs().total_cmp(&a[j][k].abs()))
                .unwrap();
            a.swap(k, p);
            b.swap(k, p);
            for i in k + 1..n {
                let f = a[i][k] / a[k][k];
                let (top, below) = a.split_at_mut(i);
                for (x, y) in below[0][k..].iter_mut().zip(&top[k][k..]) {
                    *x -= f * y;
                }
                b[i] -= f * b[k];
            }
        }
        let mut x = vec![0.0; n];
        for k in (0..n).rev() {
            let s: f64 = (k + 1..n).map(|j| a[k][j] * x[j]).sum();
            x[k] = (b[k] - s) / a[k][k];
        }
        x
    }

    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }

        fn unit(&mut self) -> f64 {
            (self.next() % 2001) as f64 / 1000.0 - 1.0
        }
    }

    /// Conductances between random nets and to ground, and voltage sources whose branch rows
    /// have no diagonal, as a circuit stamps them.
    fn circuit(rng: &mut Lcg) -> (usize, Vec<(usize, usize, f64)>) {
        let nets = 1 + (rng.next() % 30) as usize;
        let sources = (rng.next() % 4) as usize;
        let n = nets + sources;
        let mut entries = Vec::new();
        for net in 0..nets {
            entries.push((net, net, 1e-12));
            let g = libm::pow(10.0, (rng.next() % 9) as f64 - 4.0);
            entries.push((net, net, g));
            for _ in 0..rng.next() % 3 {
                let other = (rng.next() as usize) % nets;
                if other != net {
                    let g = libm::pow(10.0, (rng.next() % 9) as f64 - 4.0);
                    entries.extend([
                        (net, net, g),
                        (other, other, g),
                        (net, other, -g),
                        (other, net, -g),
                    ]);
                }
            }
        }
        for s in 0..sources {
            let row = nets + s;
            let p = (rng.next() as usize) % nets;
            entries.extend([(p, row, 1.0), (row, p, 1.0)]);
        }
        (n, entries)
    }

    #[test]
    fn random_circuits_solve_to_the_dense_answer() {
        let mut rng = Lcg(0x5eed);
        let mut solved = 0;
        for trial in 0..3_000 {
            let (n, entries) = circuit(&mut rng);
            let b: Vec<f64> = (0..n).map(|_| rng.unit()).collect();
            let want = reference(n, &entries, &b);
            match solve(n, &entries, &b) {
                // Both answers carry the rounding of an ill-conditioned system, so the check is
                // the largest residual against the largest row of terms.
                Ok(got) => {
                    solved += 1;
                    let mut residual = b.iter().map(|v| -v).collect::<Vec<_>>();
                    let mut size = b.iter().map(|v| v.abs()).collect::<Vec<_>>();
                    for &(row, col, value) in &entries {
                        residual[row] += value * got[col];
                        size[row] += (value * got[col]).abs();
                    }
                    let largest = |v: &[f64]| v.iter().fold(0.0f64, |m, x| m.max(x.abs()));
                    assert!(
                        largest(&residual) <= 1e-12 * largest(&size),
                        "trial {trial}: {got:?} vs {want:?}"
                    );
                }
                Err(Failure::Singular { .. }) => assert!(
                    want.iter().any(|v| !v.is_finite() || v.abs() > 1e12),
                    "trial {trial}: singular, but the dense answer is {want:?}"
                ),
                Err(failure) => panic!("trial {trial}: {failure:?}"),
            }
        }
        assert!(solved > 2_000, "{solved}");
    }

    #[test]
    fn a_refactorization_on_the_kept_pivots_repeats_the_bits_of_the_first() {
        let mut rng = Lcg(7);
        for _ in 0..500 {
            let (n, entries) = circuit(&mut rng);
            let (pattern, values) = sparse(n, &entries);
            let b: Vec<f64> = (0..n).map(|_| rng.unit()).collect();
            let mut scratch = Scratch::default();
            let mut work = 0;
            let Ok((shape, factors)) = factor(
                &pattern,
                &values,
                pattern.order(pattern.late(&values, &vec![false; pattern.n()])),
                &mut scratch,
                &mut work,
            ) else {
                continue;
            };
            let mut again = factors.clone();
            again.l.fill(f64::NAN);
            again.u.fill(f64::NAN);
            refactor(
                &shape,
                &pattern,
                &values,
                &mut again,
                &mut scratch,
                &mut work,
            )
            .unwrap();
            let first = factors.solve(&shape, &b, &mut work).unwrap();
            let second = again.solve(&shape, &b, &mut work).unwrap();
            assert_eq!(
                first.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                second.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn a_kept_pivot_that_shrinks_under_the_threshold_is_refused() {
        // [[4, 1], [1, 4]] pivots on its diagonal. With a 1e-6 corner the kept pivot is under
        // 1e-3 of the entry below it.
        let entries = |corner: f64| [(0, 0, corner), (0, 1, 1.0), (1, 0, 1.0), (1, 1, 4.0)];
        let (pattern, values) = sparse(2, &entries(4.0));
        let mut scratch = Scratch::default();
        let mut work = 0;
        let (shape, mut factors) = factor(
            &pattern,
            &values,
            Order::plain(vec![0, 1]),
            &mut scratch,
            &mut work,
        )
        .unwrap();
        let (_, small) = sparse(2, &entries(1e-6));
        assert_eq!(
            refactor(
                &shape,
                &pattern,
                &small,
                &mut factors,
                &mut scratch,
                &mut work
            ),
            Err(Refused::Pivot)
        );
        // A factorization that pivots again takes the other row.
        let (shape, factors) = factor(
            &pattern,
            &small,
            Order::plain(vec![0, 1]),
            &mut scratch,
            &mut work,
        )
        .unwrap();
        let x = factors.solve(&shape, &[1.0, 2.0], &mut work).unwrap();
        let want = reference(2, &entries(1e-6), &[1.0, 2.0]);
        assert!(
            (x[0] - want[0]).abs() < 1e-12 && (x[1] - want[1]).abs() < 1e-12,
            "{x:?}"
        );
        assert!(scratch.x.iter().all(|&v| v == 0.0));
    }

    /// A 0 V source from a light net to a stiff one, as a USB bridge's idle DTR sits on its
    /// 5 V rail beside a module's 1 uF at a 10 ps step. Its current is the light net's gmin
    /// draw, about 5 pA. Three more nets in a ring on the rail put the current's column first,
    /// with an entry of 1 in the rail's row and in the light net's. Pivoted on the rail's row,
    /// the lower one, the current would be the difference of terms near 5e5 A and carry their
    /// rounding, about 1e-10 A.
    #[test]
    fn a_source_current_pivots_on_the_row_where_its_entry_is_largest_for_that_row() {
        let (rail, light, branch) = (0, 4, 5);
        let g = 1e5;
        let mut entries = vec![
            (rail, rail, g + 3.0 * 1e-3),
            (rail, branch, -1.0),
            (light, light, 1e-12),
            (light, branch, 1.0),
            (branch, light, 1.0),
            (branch, rail, -1.0),
        ];
        for net in 1..4 {
            let next = net % 3 + 1;
            entries.extend([
                (net, net, 4e-3),
                (net, rail, -1e-3),
                (rail, net, -1e-3),
                (net, next, -1e-3),
                (next, net, -1e-3),
            ]);
        }
        let mut b = vec![0.0; 6];
        b[rail] = g * 5.000_000_3;
        let (pattern, values) = sparse(6, &entries);
        let node = [true, true, true, true, true, false];
        let (mut scratch, mut work) = (Scratch::default(), 0);
        let (shape, factors) = factor(
            &pattern,
            &values,
            pattern.order(pattern.late(&values, &node)),
            &mut scratch,
            &mut work,
        )
        .unwrap();
        let x = factors.solve(&shape, &b, &mut work).unwrap();
        let exact = -1e-12 * x[light];
        assert!(
            (x[branch] - exact).abs() <= 1e-6 * exact.abs(),
            "{} A against {exact} A",
            x[branch]
        );
    }

    /// The same source current when the rail's row holds only the 1s of other branches' currents
    /// and conductances under 1 S, as a 5 V rail with its capacitors and sources as currents and
    /// a Schottky's series conductance. The rail's largest entry is then 1, the same as the light
    /// net's, and the tie would go to the rail, the lower row.
    #[test]
    fn a_source_current_pivots_on_the_light_net_when_the_rail_holds_only_currents() {
        let (rail, light, branch) = (0, 1, 2);
        let mut entries = vec![
            (rail, rail, 0.9),
            (rail, branch, -1.0),
            (light, light, 1e-12),
            (light, branch, 1.0),
            (branch, light, 1.0),
            (branch, rail, -1.0),
        ];
        // Three held capacitors' currents into the rail, each behind 1e-6 ohm to ground.
        for cap in 3..6 {
            entries.extend([(rail, cap, 1.0), (cap, rail, 1.0), (cap, cap, -1e-6)]);
        }
        let (pattern, values) = sparse(6, &entries);
        let node = [true, true, false, false, false, false];
        let (mut scratch, mut work) = (Scratch::default(), 0);
        let (shape, _) = factor(
            &pattern,
            &values,
            pattern.order(pattern.late(&values, &node)),
            &mut scratch,
            &mut work,
        )
        .unwrap();
        let step = shape.order.iter().position(|&col| col == branch).unwrap();
        assert_eq!(shape.pivot[step], light, "{shape:?}");
    }

    #[test]
    fn the_dense_table_finds_the_slot_the_search_finds() {
        let mut rng = Lcg(11);
        for n in [1, 7, 40, DENSE_UNKNOWNS, DENSE_UNKNOWNS + 1] {
            let entries: Vec<(usize, usize)> = (0..4 * n)
                .map(|_| ((rng.next() as usize) % n, (rng.next() as usize) % n))
                .collect();
            let pattern = Pattern::empty(n).with(entries);
            assert_eq!(pattern.dense.is_some(), n <= DENSE_UNKNOWNS);
            for _ in 0..20_000 {
                let (row, col) = ((rng.next() as usize) % n, (rng.next() as usize) % n);
                assert_eq!(pattern.slot(row, col), pattern.search(row, col));
            }
            for col in 0..n {
                for slot in pattern.column_slots(col) {
                    assert_eq!(pattern.slot(pattern.row(slot), col), Some(slot));
                }
            }
        }
    }

    #[test]
    fn two_sources_on_one_net_are_singular_at_the_second() {
        // Net 0, then the branch rows of two sources from it to ground at 5 V and 3 V.
        let entries = [
            (0, 0, 1e-12),
            (0, 1, 1.0),
            (1, 0, 1.0),
            (0, 2, 1.0),
            (2, 0, 1.0),
        ];
        assert_eq!(
            solve(3, &entries, &[0.0, 5.0, 3.0]),
            Err(Failure::Singular { col: 2 })
        );
    }

    #[test]
    fn a_pivot_within_the_rounding_of_its_sums_is_singular() {
        // Row 1 is row 0 times 3 in exact arithmetic, and 0.1 * 3 rounds, so elimination leaves
        // a remainder of one rounding, not zero.
        let third = 0.1;
        let entries = [(0, 0, third), (0, 1, 0.7), (1, 0, 0.3), (1, 1, 2.1)];
        let (pattern, values) = sparse(2, &entries);
        let mut scratch = Scratch::default();
        let mut work = 0;
        assert!(matches!(
            factor(
                &pattern,
                &values,
                Order::plain(vec![0, 1]),
                &mut scratch,
                &mut work
            ),
            Err(Failure::Singular { col: 1 })
        ));
        // A nearly singular system whose remainder is far above its rounding solves.
        let entries = [(0, 0, 1.0), (0, 1, 1.0), (1, 0, 1.0), (1, 1, 1.0 + 1e-10)];
        let x = solve(2, &entries, &[2.0, 2.0 + 1e-10]).unwrap();
        assert!(
            (x[0] - 1.0).abs() < 1e-5 && (x[1] - 1.0).abs() < 1e-5,
            "{x:?}"
        );
    }

    #[test]
    fn an_overflowing_solution_names_its_unknown() {
        let entries = [(0, 0, 1e-300), (1, 1, 1.0)];
        assert_eq!(
            solve(2, &entries, &[1e10, 1.0]),
            Err(Failure::NonFinite { col: 0 })
        );
    }

    #[test]
    fn minimum_degree_puts_a_shared_rail_last() {
        // Net 0 couples to nets 1 to 5, which couple to nothing else. The rail waits until one
        // net is left beside it, and the tie goes to the lower index.
        let mut entries = vec![(0, 0, 5.0)];
        for net in 1..6 {
            entries.extend([(net, net, 1.0), (0, net, -1.0), (net, 0, -1.0)]);
        }
        let (pattern, values) = sparse(6, &entries);
        assert_eq!(
            pattern
                .order(pattern.late(&values, &vec![false; pattern.n()]))
                .columns,
            vec![1, 2, 3, 4, 0, 5]
        );
    }

    #[test]
    fn a_column_a_single_row_pins_comes_first_and_fills_nothing() {
        // Row 2 pins unknown 1. Unknown 0 couples to 1 and 2, and would go first on degree.
        let entries = [
            (0, 0, 1.0),
            (0, 1, 1.0),
            (1, 0, 1.0),
            (1, 1, 1.0),
            (1, 2, 1.0),
            (2, 1, 1.0),
        ];
        let (pattern, values) = sparse(3, &entries);
        assert_eq!(
            pattern
                .order(pattern.late(&values, &vec![false; pattern.n()]))
                .columns,
            vec![1, 0, 2]
        );
    }

    #[test]
    fn an_unknown_with_a_tiny_diagonal_waits_for_the_others() {
        // Two held capacitors in parallel on node 0, currents 1 and 2, as `HOLD` stamps them.
        // On degree a current would go first, and the split would be lost to the rounding of
        // node 0.
        let entries = [
            (0, 0, 1e-3),
            (0, 1, 1.0),
            (0, 2, 1.0),
            (1, 0, 1.0),
            (1, 1, -1e-9),
            (2, 0, 1.0),
            (2, 2, -5e-10),
        ];
        let (pattern, values) = sparse(3, &entries);
        let order = pattern.order(pattern.late(&values, &[true, false, false]));
        assert_eq!(order.columns, vec![0, 1, 2]);
        let (mut scratch, mut work) = (Scratch::default(), 0);
        let (shape, factors) = factor(&pattern, &values, order, &mut scratch, &mut work).unwrap();
        let x = factors.solve(&shape, &[0.0, 5.0, 5.0], &mut work).unwrap();
        let total = -5e-3 / (1.0 + 1e-3 * 1e-9 * 5e-10 / (1e-9 + 5e-10));
        assert!((x[1] - total / 3.0).abs() <= 1e-12 * total.abs(), "{x:?}");
        assert!(
            (x[2] - 2.0 * total / 3.0).abs() <= 1e-12 * total.abs(),
            "{x:?}"
        );
    }

    #[test]
    fn a_chain_factors_with_no_fill() {
        let n = 1000;
        let mut entries = Vec::new();
        for k in 0..n {
            entries.push((k, k, 2.0));
            if k + 1 < n {
                entries.extend([(k, k + 1, -1.0), (k + 1, k, -1.0)]);
            }
        }
        let (pattern, values) = sparse(n, &entries);
        let mut scratch = Scratch::default();
        let mut work = 0;
        let (shape, _) = factor(
            &pattern,
            &values,
            pattern.order(pattern.late(&values, &vec![false; pattern.n()])),
            &mut scratch,
            &mut work,
        )
        .unwrap();
        assert_eq!(shape.l_rows.len() + shape.u_steps.len(), 2 * (n - 1));
        assert!(work < 2 * n as u64, "{work}");
    }

    #[test]
    fn a_chain_with_every_capacitor_held_fills_as_little_as_a_stepped_one() {
        // Nodes 0 to n - 1 in a chain of 1 kohm, each with a 1 nF capacitor to ground held as
        // `HOLD` stamps it, its current at n + k. Taking every node before any current would
        // couple the currents to each other on the graph the order is chosen on.
        let n = 1000;
        let mut entries = Vec::new();
        for k in 0..n {
            entries.extend([(k, k, 2e-3), (k, n + k, 1.0), (n + k, k, 1.0)]);
            entries.push((n + k, n + k, -1e-6));
            if k + 1 < n {
                entries.extend([(k, k + 1, -1e-3), (k + 1, k, -1e-3)]);
            }
        }
        let (pattern, values) = sparse(2 * n, &entries);
        let node: Vec<bool> = (0..2 * n).map(|k| k < n).collect();
        let order = pattern.order(pattern.late(&values, &node));
        // Each current comes right after its node, which pivots on the current's row.
        let interleaved: Vec<usize> = (0..n).flat_map(|k| [k, n + k]).collect();
        assert_eq!(order.columns, interleaved);
        let (mut scratch, mut work) = (Scratch::default(), 0);
        let (shape, _) = factor(&pattern, &values, order, &mut scratch, &mut work).unwrap();
        let entries = shape.l_rows.len() + shape.u_steps.len();
        assert!(entries < 8 * n, "{entries}");
        assert!(work < 20 * n as u64, "{work}");
    }
}
