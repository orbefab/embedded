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

    fn get(&self, row: usize, col: usize) -> f64 {
        self.a[row * self.n + col]
    }

    fn set(&mut self, row: usize, col: usize, value: f64) {
        self.a[row * self.n + col] = value;
    }

    fn swap_rows(&mut self, i: usize, j: usize) {
        if i == j {
            return;
        }
        for col in 0..self.n {
            self.a.swap(i * self.n + col, j * self.n + col);
        }
    }
}

fn submul(left: f64, factor: f64, right: f64) -> f64 {
    left - factor * right
}

pub(crate) fn solve(mut a: Matrix, mut b: Vec<f64>) -> Result<Vec<f64>, Singular> {
    let n = a.n;
    if n == 0 {
        return Ok(Vec::new());
    }
    for column in 0..n {
        let mut pivot = column;
        let mut best = a.get(column, column).abs();
        for row in (column + 1)..n {
            let magnitude = a.get(row, column).abs();
            if magnitude > best {
                best = magnitude;
                pivot = row;
            }
        }
        if a.get(pivot, column) == 0.0 {
            return Err(Singular { row: column });
        }
        if pivot != column {
            a.swap_rows(pivot, column);
            b.swap(pivot, column);
        }
        let diag = a.get(column, column);
        for row in (column + 1)..n {
            let factor = a.get(row, column) / diag;
            for col in (column + 1)..n {
                let value = submul(a.get(row, col), factor, a.get(column, col));
                a.set(row, col, value);
            }
            b[row] = submul(b[row], factor, b[column]);
        }
    }
    let mut x = vec![0.0; n];
    for row in (0..n).rev() {
        let mut sum = b[row];
        for (col, &x_col) in x.iter().enumerate().skip(row + 1) {
            sum = submul(sum, a.get(row, col), x_col);
        }
        x[row] = sum / a.get(row, row);
    }
    Ok(x)
}
