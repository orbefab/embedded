//! The digit a single 7-segment part shows, from which of a–g are lit.
//!
//! The decimal point is not part of the digit. A pattern that is one of 0–9 is
//! that digit. 6 without its top bar and 9 without its bottom bar are 6 and 9, as the
//! 74LS47 draws them, and 7 with `f` is 7, as many firmwares draw it. None of a–g lit is
//! `blank`. Any other pattern names the lit segments, so a caller can still see the
//! display.

/// `lit` is the letters among `a`–`g` that are lit, in letter order.
pub(crate) fn shown(lit: &[&str]) -> String {
    match lit.join("").as_str() {
        "abcdef" => "0".to_string(),
        "bc" => "1".to_string(),
        "abdeg" => "2".to_string(),
        "abcdg" => "3".to_string(),
        "bcfg" => "4".to_string(),
        "acdfg" => "5".to_string(),
        "acdefg" | "cdefg" => "6".to_string(),
        "abc" | "abcf" => "7".to_string(),
        "abcdefg" => "8".to_string(),
        "abcdfg" | "abcfg" => "9".to_string(),
        "" => "blank".to_string(),
        _ => format!("segments {}", lit.join(",")),
    }
}

/// The engine-part suffix of segment `letter`, such as `seg_a`.
pub(crate) fn segment(letter: &str) -> String {
    format!("seg_{letter}")
}

pub(crate) const LETTERS: [&str; 7] = ["a", "b", "c", "d", "e", "f", "g"];

#[cfg(test)]
mod tests {
    use super::shown;

    #[test]
    fn each_digit_and_a_pattern_that_is_not_one() {
        assert_eq!(shown(&["a", "b", "c", "d", "e", "f"]), "0");
        assert_eq!(shown(&["b", "c"]), "1");
        assert_eq!(shown(&["a", "b", "c", "d", "e", "f", "g"]), "8");
        assert_eq!(shown(&["a", "b", "c", "d", "f", "g"]), "9");
        assert_eq!(shown(&[]), "blank");
        assert_eq!(shown(&["a", "g"]), "segments a,g");
        assert_eq!(shown(&["b", "c", "g"]), "segments b,c,g");
    }

    #[test]
    fn a_six_seven_or_nine_without_its_tail_is_that_digit() {
        assert_eq!(shown(&["c", "d", "e", "f", "g"]), "6");
        assert_eq!(shown(&["a", "b", "c", "f", "g"]), "9");
        assert_eq!(shown(&["a", "b", "c", "f"]), "7");
    }
}
