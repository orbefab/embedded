//! `scripts/install.sh` bounds its downloads.

use std::fs;
use std::path::PathBuf;

#[test]
fn downloads_time_out_and_retry() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/install.sh");
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains(
        "curl -fsSL --connect-timeout 15 --retry 3 --retry-delay 2 -o /dev/null -w '%{url_effective}'"
    ));
    assert!(text.contains(
        "curl -fsSL --connect-timeout 15 --retry 3 --retry-delay 2 -o \"$dest\" \"$url\""
    ));
    assert!(text.contains("wget -q --timeout=15 --tries=3 -O \"$dest\" \"$url\""));
    let last = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap();
    assert_eq!(last, "main \"$@\"");
}
