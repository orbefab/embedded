//! A board's holes as text: which holes it has, and which holes a strip joins. Both name only
//! holes that exist, so a rail with a gap every sixth column never reads as one span.

/// A hole's name as its row and its column, as `W` and 12 for `W12`.
fn row_column(name: &str) -> Option<(&str, u32)> {
    let at = name.find(|c: char| c.is_ascii_digit())?;
    let (row, column) = name.split_at(at);
    if row.is_empty() || !row.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some((row, column.parse().ok()?))
}

/// Whether row `b` is the row after `a`, as `b` after `a`.
fn next_row(a: &str, b: &str) -> bool {
    let (mut a, mut b) = (a.chars(), b.chars());
    match (a.next(), b.next(), a.next(), b.next()) {
        (Some(a), Some(b), None, None) => b as u32 == a as u32 + 1,
        _ => false,
    }
}

/// Whether hole `b` is the hole after `a` along a row or down a column.
fn next(a: &str, b: &str) -> bool {
    let (Some((ra, ca)), Some((rb, cb))) = (row_column(a), row_column(b)) else {
        return false;
    };
    (ra == rb && cb == ca + 1) || (ca == cb && next_row(ra, rb))
}

/// Runs of items, each item following the one before it by `follows`.
fn runs<T: Copy>(items: &[T], follows: impl Fn(T, T) -> bool) -> Vec<(T, T)> {
    let mut out: Vec<(T, T)> = Vec::new();
    for &item in items {
        match out.last_mut() {
            Some((_, last)) if follows(*last, item) => *last = item,
            _ => out.push((item, item)),
        }
    }
    out
}

fn span<T: std::fmt::Display + PartialEq>(prefix: &str, (first, last): (T, T)) -> String {
    if first == last {
        format!("{prefix}{first}")
    } else {
        format!("{prefix}{first} to {prefix}{last}")
    }
}

/// The holes one strip joins, as runs of holes side by side, each name after `prefix`: `a1 to
/// e1`, or a rail with gaps as `W1 to W5, W7 to W11`.
pub fn strip_text(strip: &[String], prefix: &str) -> String {
    let names: Vec<&str> = strip.iter().map(String::as_str).collect();
    runs(&names, next)
        .into_iter()
        .map(|run| span(prefix, run))
        .collect::<Vec<_>>()
        .join(", ")
}

/// How many rows apart `a` and `b` are, when both are one letter of the same case. `None` when
/// they are not rows a person could step between, such as `j` and `J`.
fn row_steps(a: &str, b: &str) -> Option<u32> {
    if a.len() != 1 || b.len() != 1 {
        return (a == b).then_some(0);
    }
    let (a, b) = (a.as_bytes()[0], b.as_bytes()[0]);
    if a.is_ascii_lowercase() != b.is_ascii_lowercase() {
        return None;
    }
    Some(u32::from(a.abs_diff(b)))
}

/// The hole `name` most likely meant: the same hole in the other case, or one hole one row or
/// one column away. A name many columns off, or diagonal from a hole, is not close, so `j99`
/// does not mean `j9`.
pub(crate) fn near_hole<'a>(
    name: &str,
    holes: impl IntoIterator<Item = &'a str>,
) -> Option<&'a str> {
    let holes: Vec<&str> = holes.into_iter().collect();
    if let Some(same) = holes
        .iter()
        .copied()
        .filter(|hole| hole.eq_ignore_ascii_case(name) && *hole != name)
        .min()
    {
        return Some(same);
    }
    let (asked_row, asked_col) = row_column(name)?;
    let mut best: Option<(u32, &str)> = None;
    for hole in holes {
        let Some((row, col)) = row_column(hole) else {
            continue;
        };
        let Some(rows) = row_steps(asked_row, row) else {
            continue;
        };
        let dist = rows + asked_col.abs_diff(col);
        if dist == 0 || dist > 1 {
            continue;
        }
        best = Some(match best {
            Some((have, prev)) if have < dist => (have, prev),
            Some((have, prev)) if have == dist => (have, closer(asked_row, prev, hole)),
            _ => (dist, hole),
        });
    }
    best.map(|(_, hole)| hole)
}

/// Of two holes the same distance away, the one on the asked row, else the earlier name.
fn closer<'a>(asked_row: &str, a: &'a str, b: &'a str) -> &'a str {
    let shares = |name: &str| row_column(name).is_some_and(|(row, _)| row == asked_row);
    match (shares(a), shares(b)) {
        (true, false) => a,
        (false, true) => b,
        _ => std::cmp::min(a, b),
    }
}

/// "did you mean `{prefix}{hole}`? " when [`near_hole`] finds one, else "".
pub(crate) fn near_hole_hint<'a>(
    name: &str,
    holes: impl IntoIterator<Item = &'a str>,
    prefix: &str,
) -> String {
    near_hole(name, holes)
        .map(|hole| format!("did you mean `{prefix}{hole}`? "))
        .unwrap_or_default()
}

/// Every hole of a board, as rows that share their columns: `rows a to j at columns 1 to 30;
/// rows W to Z at columns 1 to 5, 7 to 11`. A hole not named row then column is listed whole.
pub fn holes_text<'a>(names: impl IntoIterator<Item = &'a str>) -> String {
    let mut rows: Vec<(&str, Vec<u32>)> = Vec::new();
    let mut other: Vec<&str> = Vec::new();
    for name in names {
        let Some((row, column)) = row_column(name) else {
            other.push(name);
            continue;
        };
        match rows.iter_mut().find(|(r, _)| *r == row) {
            Some((_, columns)) => columns.push(column),
            None => rows.push((row, vec![column])),
        }
    }
    let mut groups: Vec<(Vec<&str>, Vec<u32>)> = Vec::new();
    for (row, mut columns) in rows {
        columns.sort_unstable();
        match groups.last_mut() {
            Some((names, same)) if *same == columns => names.push(row),
            _ => groups.push((vec![row], columns)),
        }
    }
    let mut parts: Vec<String> = groups
        .into_iter()
        .map(|(names, columns)| {
            let rows = runs(&names, next_row).into_iter().map(|run| span("", run));
            let columns = runs(&columns, |a, b| b == a + 1)
                .into_iter()
                .map(|run| span("", run));
            let noun = if names.len() == 1 { "row" } else { "rows" };
            format!(
                "{noun} {} at columns {}",
                rows.collect::<Vec<_>>().join(", "),
                columns.collect::<Vec<_>>().join(", ")
            )
        })
        .collect();
    if !other.is_empty() {
        parts.push(other.join(", "));
    }
    parts.join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn a_column_strip_is_one_run_and_a_rail_lists_its_groups() {
        assert_eq!(
            strip_text(&strip(&["a1", "b1", "c1", "d1", "e1"]), "bb."),
            "bb.a1 to bb.e1"
        );
        let rail: Vec<String> = [1, 2, 3, 4, 5, 7, 8, 9, 10, 11]
            .iter()
            .map(|c| format!("W{c}"))
            .collect();
        assert_eq!(strip_text(&rail, ""), "W1 to W5, W7 to W11");
    }

    #[test]
    fn holes_read_as_rows_that_share_their_columns() {
        let mut names: Vec<String> = Vec::new();
        for row in ["a", "b", "c"] {
            names.extend((1..=4).map(|c| format!("{row}{c}")));
        }
        for row in ["W", "X"] {
            names.extend([1, 2, 4].iter().map(|c| format!("{row}{c}")));
        }
        assert_eq!(
            holes_text(names.iter().map(String::as_str)),
            "rows a to c at columns 1 to 4; rows W to X at columns 1 to 2, 4"
        );
    }

    /// A half-size board: rows a to j, columns 1 to 30, and rails with a gap every sixth column.
    fn board_400() -> Vec<String> {
        let mut names = Vec::new();
        for row in b'a'..=b'j' {
            for col in 1..=30 {
                names.push(format!("{}{col}", row as char));
            }
        }
        for row in ["W", "X", "Y", "Z"] {
            for col in 1..=30 {
                if col % 6 != 0 {
                    names.push(format!("{row}{col}"));
                }
            }
        }
        names
    }

    #[test]
    fn a_far_hole_is_not_a_near_name_and_a_neighbor_is() {
        let names = board_400();
        let at = |name| near_hole(name, names.iter().map(String::as_str));
        assert_eq!(at("j99"), None, "j99 is not close to j9 or j30");
        assert_eq!(at("j31"), Some("j30"));
        assert_eq!(at("J9"), Some("j9"));
        assert_eq!(at("W6"), Some("W5"));
        assert_eq!(at("k1"), Some("j1"));
        assert_eq!(at("k31"), None, "diagonal from j30");
    }
}
