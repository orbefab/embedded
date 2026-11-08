//! The patterns the schemas give strings. They state the rules the build enforces, so a client
//! that validates a record against the schema checks the same.

macro_rules! name {
    () => {
        "[a-z][a-z0-9-]{0,63}"
    };
}

macro_rules! numbers {
    () => {
        r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    };
}

macro_rules! path_char {
    () => {
        r#"[^/\\:*?"<>|\x00-\x1f\x7f]"#
    };
}

macro_rules! path_segment {
    () => {
        concat!(path_char!(), "*", r#"[^/\\:*?"<>|\x00-\x1f\x7f. ]"#)
    };
}

/// An item's id: kebab case, starting with a letter, at most 64 characters, and none of
/// [`crate::RESERVED`].
pub const ID: &str = concat!(
    "^(?!(index|con|prn|aux|nul|com[0-9]|lpt[0-9])$)",
    name!(),
    "$"
);
/// A version: three numbers with no sign and no leading zero.
pub const VERSION: &str = concat!("^", numbers!(), "$");
/// `[@<registry>/]<id>@<version>`.
pub const DEPENDENCY: &str = concat!("^(@", name!(), "/)?", name!(), "@", numbers!(), "$");
/// A registry's name: `@` and an id.
pub const REGISTRY: &str = concat!("^@", name!(), "$");
/// A record's URL relative to the index: `<id>@<version>.json`.
pub const RECORD_URL: &str = concat!("^", name!(), "@", numbers!(), r"\.json$");
pub const SHA256: &str = "^[0-9a-f]{64}$";
/// A folder hash.
pub const HASH: &str = "^sha256:[0-9a-f]{64}$";
/// A file's URL relative to its record.
pub const FILE_URL: &str = "^files/[0-9a-f]{64}$";
pub const HTTPS: &str = "^https://";
/// A file's path in its item's folder: `/`-separated segments, none empty, none ending in a dot
/// or a space (so no `.` or `..`), and no backslash, control character or `:*?"<>|`.
pub const PATH: &str = concat!("^", path_segment!(), "(/", path_segment!(), ")*$");
