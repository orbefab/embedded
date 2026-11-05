//! The file-by-file text `orbe add --diff` prints.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::CatalogEntry;

/// Each file of `parts/<id>` that the catalog's copy would add, remove or change. `have` is
/// `None` when there is no folder to compare.
pub(super) fn diff_files(
    id: &str,
    have: Option<&BTreeMap<String, Vec<u8>>>,
    entry: &CatalogEntry,
) -> String {
    let empty = BTreeMap::new();
    let files = have.unwrap_or(&empty);
    let paths: BTreeSet<&str> = files
        .keys()
        .map(String::as_str)
        .chain(entry.files.keys().map(String::as_str))
        .collect();
    let mut out = String::new();
    for rel in paths {
        let path = format!("parts/{id}/{rel}");
        match (files.get(rel), entry.files.get(rel)) {
            (None, Some(_)) => out.push_str(&format!("{path} added\n")),
            (Some(_), None) => out.push_str(&format!("{path} removed\n")),
            (Some(old), Some(new)) if old == new => {}
            (Some(old), Some(new)) => out.push_str(&diff_one(&path, old, new)),
            (None, None) => {}
        }
    }
    out
}

fn diff_one(path: &str, old: &[u8], new: &[u8]) -> String {
    let (Ok(old), Ok(new)) = (std::str::from_utf8(old), std::str::from_utf8(new)) else {
        return format!("{path} differs\n");
    };
    let body = line_diff(old, new);
    if body.is_empty() {
        format!("{path} differs\n")
    } else {
        format!("{path}\n{body}")
    }
}

/// Lines removed from the project copy, then lines the catalog copy would write.
fn line_diff(old: &str, new: &str) -> String {
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    let (n, m) = (old.len(), new.len());
    if n.saturating_mul(m) > 2_000_000 {
        return String::new();
    }
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if old[i] == new[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut out = String::new();
    let mut i = 0;
    let mut j = 0;
    while i < n && j < m {
        if old[i] == new[j] {
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            out.push_str(&format!("-{}\n", old[i]));
            i += 1;
        } else {
            out.push_str(&format!("+{}\n", new[j]));
            j += 1;
        }
    }
    for line in &old[i..] {
        out.push_str(&format!("-{line}\n"));
    }
    for line in &new[j..] {
        out.push_str(&format!("+{line}\n"));
    }
    out
}
