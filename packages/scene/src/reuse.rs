//! A folder's `REUSE.toml`, read as REUSE 3.3 reads one
//! (<https://reuse.software/spec-3.3/>, "REUSE.toml"). It gives each file the folder holds its
//! licence, so a file that cannot carry an SPDX line, such as a firmware image or a licence text,
//! still names its own.

use serde::Deserialize;

/// The file a folder names its files' licences in.
pub const FILE: &str = "REUSE.toml";

#[derive(Debug, Deserialize)]
struct Toml {
    version: u32,
    #[serde(default)]
    annotations: Vec<Annotation>,
}

#[derive(Debug, Deserialize)]
struct Annotation {
    path: OneOrMany,
    #[serde(default)]
    precedence: Precedence,
    #[serde(rename = "SPDX-License-Identifier")]
    license: Option<OneOrMany>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn items(&self) -> &[String] {
        match self {
            OneOrMany::One(one) => std::slice::from_ref(one),
            OneOrMany::Many(many) => many,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Precedence {
    /// The file's own SPDX line wins, and the annotation fills in when it has none.
    #[default]
    Closest,
    /// The file's own licence and the annotation's both apply.
    Aggregate,
    /// The annotation wins over the file's own SPDX line.
    Override,
}

/// The licence annotations of one `REUSE.toml`, in the file's order.
#[derive(Debug, Default)]
pub struct Reuse {
    rules: Vec<(Vec<String>, Precedence, String)>,
}

impl Reuse {
    /// Reads `REUSE.toml`'s text. The error says what does not read.
    pub fn parse(text: &str) -> Result<Reuse, String> {
        let toml: Toml =
            toml::from_str(text).map_err(|error| format!("{FILE} does not read: {error}"))?;
        if toml.version != 1 {
            return Err(format!(
                "{FILE} is version {}, and orbe reads version 1",
                toml.version
            ));
        }
        let rules = toml
            .annotations
            .into_iter()
            .filter_map(|annotation| {
                let license = annotation.license?;
                let license = match license.items() {
                    [one] => one.clone(),
                    many => many
                        .iter()
                        .map(|id| grouped(id))
                        .collect::<Vec<_>>()
                        .join(" AND "),
                };
                Some((
                    annotation.path.items().to_vec(),
                    annotation.precedence,
                    license,
                ))
            })
            .collect();
        Ok(Reuse { rules })
    }

    /// The licence of the file at `path`, whose own SPDX line, if it has one, says `own`. Of the
    /// annotations whose `path` matches, the last one applies, as the specification says.
    pub fn license(&self, path: &str, own: Option<String>) -> Option<String> {
        let Some((_, precedence, license)) = self.rules.iter().rev().find(|(globs, _, _)| {
            globs
                .iter()
                .any(|glob| matches(glob.as_bytes(), path.as_bytes()))
        }) else {
            return own;
        };
        Some(match (precedence, own) {
            (Precedence::Override, _) | (_, None) => license.clone(),
            (Precedence::Closest, Some(own)) => own,
            (Precedence::Aggregate, Some(own)) => {
                format!("{} AND {}", grouped(&own), grouped(license))
            }
        })
    }
}

fn grouped(expression: &str) -> String {
    if expression.contains(' ') {
        format!("({expression})")
    } else {
        expression.to_string()
    }
}

/// REUSE's glob: `*` matches any run of characters but `/`, `**` any run at all, and `\` takes
/// the next character as it is.
fn matches(glob: &[u8], path: &[u8]) -> bool {
    match glob {
        [] => path.is_empty(),
        [b'*', b'*', rest @ ..] => (0..=path.len()).any(|skip| matches(rest, &path[skip..])),
        [b'*', rest @ ..] => {
            let run = path.iter().position(|&c| c == b'/').unwrap_or(path.len());
            (0..=run).any(|skip| matches(rest, &path[skip..]))
        }
        [b'\\', c, rest @ ..] => path.first() == Some(c) && matches(rest, &path[1..]),
        [c, rest @ ..] => path.first() == Some(c) && matches(rest, &path[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_glob_matches_as_reuse_reads_it() {
        assert!(matches(b"**", b"firmware/button/button.ino"));
        assert!(matches(b"firmware/*.hex", b"firmware/button.hex"));
        assert!(!matches(b"firmware/*.hex", b"firmware/button/button.hex"));
        assert!(matches(b"firmware/**.hex", b"firmware/button/button.hex"));
        assert!(matches(b"firmware/LICENSE-libc", b"firmware/LICENSE-libc"));
        assert!(!matches(
            b"firmware/LICENSE-libc",
            b"firmware/LICENSE-libcx"
        ));
        assert!(matches(br"a\*b", b"a*b"));
        assert!(!matches(br"a\*b", b"axb"));
    }

    #[test]
    fn the_last_matching_annotation_gives_the_licence() {
        let reuse = Reuse::parse(
            r#"
version = 1

[[annotations]]
path = "**"
SPDX-License-Identifier = "MIT OR Apache-2.0"

[[annotations]]
path = ["firmware/*.hex", "firmware/LICENSE-runtime"]
SPDX-License-Identifier = "LGPL-2.1-or-later"

[[annotations]]
path = "notes/*"
precedence = "override"
SPDX-License-Identifier = ["CC-BY-4.0", "MIT OR Apache-2.0"]

[[annotations]]
path = "both/*"
precedence = "aggregate"
SPDX-License-Identifier = "MIT"
"#,
        )
        .unwrap();
        assert_eq!(
            reuse.license("scene.jsonc", None).as_deref(),
            Some("MIT OR Apache-2.0")
        );
        assert_eq!(
            reuse.license("firmware/button.hex", None).as_deref(),
            Some("LGPL-2.1-or-later")
        );
        assert_eq!(
            reuse
                .license("firmware/button.hex", Some("CC0-1.0".into()))
                .as_deref(),
            Some("CC0-1.0"),
            "closest: the file's own SPDX line wins"
        );
        assert_eq!(
            reuse
                .license("notes/a.md", Some("CC0-1.0".into()))
                .as_deref(),
            Some("CC-BY-4.0 AND (MIT OR Apache-2.0)")
        );
        assert_eq!(
            reuse
                .license("both/a.md", Some("BSD-3-Clause".into()))
                .as_deref(),
            Some("BSD-3-Clause AND MIT")
        );
        assert_eq!(Reuse::default().license("a", None), None);
    }

    #[test]
    fn a_reuse_toml_that_does_not_read_says_why() {
        assert!(
            Reuse::parse("version = 2\n")
                .unwrap_err()
                .contains("version 2")
        );
        assert!(
            Reuse::parse("version = \n")
                .unwrap_err()
                .starts_with("REUSE.toml does not read")
        );
    }
}
