/// Dense LU stores an n by n matrix. A larger system is rejected before that allocation.
pub(crate) const MAX_UNKNOWNS: usize = 4096;

pub(crate) struct Matrix {
    n: usize,
    a: Vec<f64>,
}

#[derive(Debug)]
pub(crate) struct Singular {
    pub(crate) row: usize,
}

impl Matrix {
    pub(crate) fn zeros(n: usize) -> Matrix {
        Matrix {
            n,
            a: vec![0.0; n * n],
        }
    }

    pub(crate) fn add(&mut self, row: usize, col: usize, value: f64) {
        self.a[row * self.n + col] += value;
    }

    pub(crate) fn clear(&mut self) {
        self.a.fill(0.0);
    }
}

fn submul(left: f64, factor: f64, right: f64) -> f64 {
    left - factor * right
}

/// Gaussian elimination with partial pivoting, then back-substitution.
///
/// Work on an exact zero is skipped where it cannot change a bit, so the result is the full dense
/// loop's. No entry of `a` or `b` is `-0.0`. Both start at `+0.0` and are only ever accumulated
/// into, `a` through `add` and `b` with `+=` and `-=` by the stamps. In round-to-nearest a sum is
/// `-0.0` only when both operands are `-0.0`, and a difference only when the left is `-0.0` and the
/// right is `+0.0`. Elimination's `v - p` is `-0.0` only when `v` is, so it makes none either. Then
/// `v - factor * above` is `v` whenever one of `factor` and `above` is zero and the other is
/// finite. A row whose entry in the pivot column is zero is skipped while the diagonal is not NaN
/// and the pivot row and its right-hand side are finite. A row update covers only the pivot row's
/// nonzero span while the factor is finite. Back-substitution covers only each row's nonzero span
/// while every solved unknown is finite.
pub(crate) fn solve(a: &mut Matrix, b: &mut [f64]) -> Result<Vec<f64>, Singular> {
    debug_assert!(
        b.iter().all(|v| v.to_bits() != (-0.0f64).to_bits()),
        "a right-hand side entry is -0.0"
    );
    let n = a.n;
    if n == 0 {
        return Ok(Vec::new());
    }
    for column in 0..n {
        let mut pivot = column;
        let mut best = a.a[column * n + column].abs();
        for row in (column + 1)..n {
            let magnitude = a.a[row * n + column].abs();
            if magnitude > best {
                best = magnitude;
                pivot = row;
            }
        }
        if a.a[pivot * n + column] == 0.0 {
            return Err(Singular { row: column });
        }
        if pivot != column {
            let (top, bottom) = a.a.split_at_mut(pivot * n);
            top[column * n + column..(column + 1) * n].swap_with_slice(&mut bottom[column..n]);
            b.swap(pivot, column);
        }
        let (upper, lower) = a.a.split_at_mut((column + 1) * n);
        let pivot_row = &upper[column * n..(column + 1) * n];
        let diag = pivot_row[column];
        let rhs = b[column];
        let (first, end) = nonzero_span(pivot_row, column + 1);
        let finite = !diag.is_nan()
            && rhs.is_finite()
            && pivot_row[first..end].iter().all(|v| v.is_finite());
        for (row, unknown) in lower.chunks_exact_mut(n).zip(&mut b[column + 1..]) {
            if row[column] == 0.0 && finite {
                continue;
            }
            let factor = row[column] / diag;
            let (from, to) = if factor.is_finite() {
                (first, end)
            } else {
                (column + 1, n)
            };
            for (value, &above) in row[from..to].iter_mut().zip(&pivot_row[from..to]) {
                *value = submul(*value, factor, above);
            }
            *unknown = submul(*unknown, factor, rhs);
        }
    }
    let mut x = vec![0.0; n];
    let mut finite = true;
    for (row, entries) in a.a.chunks_exact(n).enumerate().rev() {
        let mut sum = b[row];
        let (from, to) = if finite {
            nonzero_span(entries, row + 1)
        } else {
            (row + 1, n)
        };
        for (&above, &known) in entries[from..to].iter().zip(&x[from..to]) {
            sum = submul(sum, above, known);
        }
        x[row] = sum / entries[row];
        finite &= x[row].is_finite();
    }
    Ok(x)
}

/// The index of the first nonzero of `row[from..]` and one past its last, or `from` twice.
fn nonzero_span(row: &[f64], from: usize) -> (usize, usize) {
    let Some(first) = row[from..].iter().position(|v| *v != 0.0) else {
        return (from, from);
    };
    let last = row
        .iter()
        .rposition(|v| *v != 0.0)
        .expect("a nonzero exists");
    (from + first, last + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `solve` before it skipped zeros.
    fn dense_solve(a: &mut Matrix, b: &mut [f64]) -> Result<Vec<f64>, Singular> {
        let n = a.n;
        if n == 0 {
            return Ok(Vec::new());
        }
        for column in 0..n {
            let mut pivot = column;
            let mut best = a.a[column * n + column].abs();
            for row in (column + 1)..n {
                let magnitude = a.a[row * n + column].abs();
                if magnitude > best {
                    best = magnitude;
                    pivot = row;
                }
            }
            if a.a[pivot * n + column] == 0.0 {
                return Err(Singular { row: column });
            }
            if pivot != column {
                let (top, bottom) = a.a.split_at_mut(pivot * n);
                top[column * n + column..(column + 1) * n].swap_with_slice(&mut bottom[column..n]);
                b.swap(pivot, column);
            }
            let (upper, lower) = a.a.split_at_mut((column + 1) * n);
            let pivot_row = &upper[column * n..];
            let diag = pivot_row[column];
            let rhs = b[column];
            for (row, unknown) in lower.chunks_exact_mut(n).zip(&mut b[column + 1..]) {
                let factor = row[column] / diag;
                for (value, &above) in row[column + 1..].iter_mut().zip(&pivot_row[column + 1..]) {
                    *value = submul(*value, factor, above);
                }
                *unknown = submul(*unknown, factor, rhs);
            }
        }
        let mut x = vec![0.0; n];
        for (row, entries) in a.a.chunks_exact(n).enumerate().rev() {
            let mut sum = b[row];
            for (&above, &known) in entries[row + 1..].iter().zip(&x[row + 1..]) {
                sum = submul(sum, above, known);
            }
            x[row] = sum / entries[row];
        }
        Ok(x)
    }

    fn index_solve(a: &mut Matrix, b: &mut [f64]) -> (Result<Vec<f64>, Singular>, Vec<usize>) {
        let n = a.n;
        let mut chosen = Vec::new();
        if n == 0 {
            return (Ok(Vec::new()), chosen);
        }
        for column in 0..n {
            let mut pivot = column;
            let mut best = a.a[column * n + column].abs();
            for row in (column + 1)..n {
                let magnitude = a.a[row * n + column].abs();
                if magnitude > best {
                    best = magnitude;
                    pivot = row;
                }
            }
            chosen.push(pivot);
            if a.a[pivot * n + column] == 0.0 {
                return (Err(Singular { row: column }), chosen);
            }
            if pivot != column {
                for col in 0..n {
                    a.a.swap(column * n + col, pivot * n + col);
                }
                b.swap(pivot, column);
            }
            let diag = a.a[column * n + column];
            for row in (column + 1)..n {
                let factor = a.a[row * n + column] / diag;
                for col in (column + 1)..n {
                    let value = submul(a.a[row * n + col], factor, a.a[column * n + col]);
                    a.a[row * n + col] = value;
                }
                b[row] = submul(b[row], factor, b[column]);
            }
        }
        let mut x = vec![0.0; n];
        for row in (0..n).rev() {
            let mut sum = b[row];
            for (col, &known) in x.iter().enumerate().skip(row + 1) {
                sum = submul(sum, a.a[row * n + col], known);
            }
            x[row] = sum / a.a[row * n + row];
        }
        (Ok(x), chosen)
    }

    fn bits(value: f64) -> u64 {
        if value.is_nan() {
            f64::NAN.to_bits()
        } else {
            value.to_bits()
        }
    }

    fn coded(result: Result<Vec<f64>, Singular>) -> Result<Vec<u64>, usize> {
        match result {
            Ok(x) => Ok(x.into_iter().map(bits).collect()),
            Err(singular) => Err(singular.row),
        }
    }

    fn expect_same(label: &str, values: &[f64], b: &[f64]) -> Vec<usize> {
        let n = b.len();
        let mut left = Matrix {
            n,
            a: values.to_vec(),
        };
        let mut right = Matrix {
            n,
            a: values.to_vec(),
        };
        let (reference, pivots) = index_solve(&mut right, &mut b.to_vec());
        assert_eq!(
            coded(solve(&mut left, &mut b.to_vec())),
            coded(reference),
            "{label}"
        );
        pivots
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

        fn value(&mut self, wide: bool) -> f64 {
            let magnitude = (self.next() % 1000) as f64 / 37.0 - 13.0;
            match self.next() % 100 {
                0..=59 => 0.0,
                60..=69 => 1.0,
                70..=74 => -1.0,
                75..=77 if wide => 1e-300 * magnitude,
                78..=80 if wide => 1e300 * magnitude,
                81 if wide => f64::INFINITY,
                82 if wide => f64::NAN,
                83..=86 => -0.0,
                _ => magnitude,
            }
        }
    }

    #[test]
    fn sparse_systems_built_like_the_stamps_match_the_dense_loop_by_bits() {
        let mut rng = Lcg(0x5eed);
        for trial in 0..200_000_u32 {
            let n = 1 + (rng.next() % 12) as usize;
            let wide = trial.is_multiple_of(3);
            let mut matrix = Matrix::zeros(n);
            let mut b = vec![0.0; n];
            for (i, rhs) in b.iter_mut().enumerate() {
                if !rng.next().is_multiple_of(8) {
                    matrix.add(i, i, 2.0 + rng.value(wide));
                }
                for _ in 0..rng.next() % 6 {
                    let j = (rng.next() as usize) % n;
                    matrix.add(i, j, rng.value(wide));
                }
                if rng.next().is_multiple_of(2) {
                    *rhs += rng.value(wide);
                } else {
                    *rhs -= rng.value(wide);
                }
            }
            let mut reference = Matrix {
                n,
                a: matrix.a.clone(),
            };
            let expected = dense_solve(&mut reference, &mut b.clone());
            assert_eq!(
                coded(solve(&mut matrix, &mut b)),
                coded(expected),
                "trial {trial}"
            );
        }
    }

    #[test]
    fn finite_results_match_the_index_loop_by_bits() {
        expect_same("n = 0", &[], &[]);
        expect_same("n = 1", &[2.5], &[7.5]);
        expect_same("n = 1 zero", &[0.0], &[1.0]);
        expect_same("n = 1 negative zero", &[-0.0], &[1.0]);
        expect_same("singular at column 0", &[0.0, 4.0, 1.0, 0.0], &[1.0, 2.0]);
        expect_same("singular at column 1", &[1.0, 2.0, 3.0, 6.0], &[1.0, 1.0]);

        let singular_after_swap = [1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 3.0, 3.0];
        assert_eq!(
            expect_same(
                "singular at column 2 after a row swap",
                &singular_after_swap,
                &[1.0, 1.0, 1.0],
            ),
            vec![0, 2, 2],
        );

        let swap_columns_0_and_2 = [
            1.0, 1.0, 1.0, 1.0, 0.0, 5.0, 1.0, 1.0, 4.0, 1.0, 1.0, 1.0, 0.0, 1.0, 6.0, 1.0,
        ];
        assert_eq!(
            expect_same(
                "row swap at columns 0 and 2",
                &swap_columns_0_and_2,
                &[1.0, 2.0, 3.0, 4.0],
            ),
            vec![2, 1, 3, 3],
        );

        let swap_column_2 = [
            5.0, 1.0, 1.0, 1.0, 1.0, 4.0, 1.0, 1.0, 1.0, 1.0, 0.5, 1.0, 1.0, 1.0, -3.0, 1.0,
        ];
        assert_eq!(
            expect_same(
                "row swap at column 2",
                &swap_column_2,
                &[1.0, 2.0, 3.0, 4.0],
            ),
            vec![0, 1, 3, 3],
        );

        let tie_keeps_the_first_row = [2.0, 1.0, 0.0, -2.0, 0.0, 5.0, 1.0, 9.0, 1.0];
        assert_eq!(
            expect_same(
                "equal magnitude keeps the earlier row",
                &tie_keeps_the_first_row,
                &[1.0, 1.0, 1.0],
            ),
            vec![0, 2, 2],
        );

        let mut fractions = Vec::new();
        let rows = [
            [4.254, -7.582, -4.037, -7.786],
            [1.203, -2.949, -3.316, -1.156],
            [-6.554, 4.832, 4.937, -7.894],
            [-3.918, 4.182, 2.965, -7.926],
        ];
        for row in rows {
            for value in row {
                fractions.push(value / 7.0);
            }
        }
        assert_eq!(
            expect_same(
                "fractions over 7 and 3",
                &fractions,
                &[-1.838 / 3.0, -3.623 / 3.0, 1.748 / 3.0, -4.049 / 3.0],
            ),
            vec![2, 2, 2, 3],
        );

        let mut wide = Vec::new();
        for i in 0..6 {
            for j in 0..6 {
                wide.push((((i * 17 + j * 13) % 19) as f64 - 9.0) / 7.0);
            }
        }
        assert_eq!(
            expect_same(
                "swaps at columns 1 through 4",
                &wide,
                &[0.5, -1.0, 2.0, 0.25, -3.0, 1.5],
            ),
            vec![0, 5, 3, 5, 5, 5],
        );

        let nonfinite = [1.0, f64::INFINITY, f64::NAN, 4.0];
        expect_same("NaN and infinity", &nonfinite, &[1.0, 2.0]);
        let mut matrix = Matrix {
            n: 2,
            a: nonfinite.to_vec(),
        };
        let solved = solve(&mut matrix, &mut [1.0, 2.0]).expect("NaN and infinity");
        assert!(solved.iter().any(|value| value.is_nan()));
    }
}
