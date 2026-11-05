//! Investigation 4's name rule, and "did you mean".

/// Words the engine's ref grammar uses after a dot, and `net`, which it reserves as a part name.
const RESERVED: &[&str] = &[
    "i", "v", "p", "ib", "ic", "id", "iin", "iout", "pin", "logic", "usart", "net",
];

/// A pin name or label may start with a digit, such as `5V`, because a ref segment is not an
/// engine name. Every other name starts with a letter.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum NameKind {
    Name,
    Pin,
}

/// Why `name` breaks the rule, or `None` when it keeps it.
pub(crate) fn problem(name: &str, kind: NameKind) -> Option<String> {
    let Some(first) = name.chars().next() else {
        return Some("is empty".into());
    };
    if kind == NameKind::Name && !first.is_ascii_alphabetic() {
        return Some("does not start with a letter".into());
    }
    if !first.is_ascii_alphanumeric() {
        return Some("does not start with a letter or digit".into());
    }
    if let Some(bad) = name
        .chars()
        .find(|c| !c.is_ascii_alphanumeric() && *c != '_')
    {
        return Some(format!(
            "contains `{bad}`; names take letters, digits and `_`"
        ));
    }
    if name.contains("__") {
        return Some("contains `__`, which joins flat engine names".into());
    }
    if name.ends_with('_') {
        return Some("ends in `_`".into());
    }
    if RESERVED.contains(&name) {
        return Some(format!(
            "is reserved, because a ref such as `@led1.{name}` uses it"
        ));
    }
    None
}

/// The candidate closest to `word`, when one is close enough to be a likely typo.
pub(crate) fn closest<'a>(
    word: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Option<&'a str> {
    candidates
        .into_iter()
        .map(|c| (strsim::jaro_winkler(word, c), c))
        .filter(|(score, _)| *score >= 0.8)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, c)| c)
}

/// What to write instead, for the reason `problem` gave.
pub(crate) fn hint(reason: &str) -> String {
    if reason.contains("__") {
        "use a single underscore, because `__` joins flat names".into()
    } else if reason.contains("reserved") {
        "pick another name".into()
    } else if reason.starts_with("contains `") {
        "use letters, digits and underscores".into()
    } else if reason.contains("empty") {
        "give it a name".into()
    } else if reason.contains("start") {
        "start the name with a letter".into()
    } else if reason.contains("ends in") {
        "drop the trailing underscore".into()
    } else {
        "rename it to fit the name rule".into()
    }
}

/// "did you mean `x`? " when one is close, else "".
pub(crate) fn did_you_mean<'a>(
    word: &str,
    candidates: impl IntoIterator<Item = &'a str>,
    prefix: &str,
) -> String {
    closest(word, candidates)
        .map(|c| format!("did you mean `{prefix}{c}`? "))
        .unwrap_or_default()
}
