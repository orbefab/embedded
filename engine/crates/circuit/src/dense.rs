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

pub(crate) fn solve(a: &mut Matrix, b: &mut [f64]) -> Result<Vec<f64>, Singular> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
