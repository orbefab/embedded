//! Reads a `scene.jsonc` and the part files it uses into the engine's netlist v1, or into
//! the findings that stop it.
//!
//! A scene's folder is its project root. An id such as `led-5mm` reads `parts/led-5mm/` there
//! and needs a line in `parts.lock`. In a Block, a folder that holds `block.jsonc`, an id reads
//! the copy in this binary's catalog. A `./` or `../` path reads a part of the user's own,
//! relative to the file that names it.
//!
//! [`Files`] says where those files are read from. `orbe add`, which writes them, is on Unix only.

#[cfg(unix)]
mod add;
mod blocks;
mod breadboard;
mod buy;
mod catalog;
mod circuit;
mod diagnostic;
mod files;
mod holes;
mod model;
mod name;
mod params;
mod project;
mod published;
mod reuse;
mod shape;
mod source;
mod summary;
mod syntax;
mod units;

use std::path::Path;

#[cfg(unix)]
pub use add::{AddCode, Reply, Request, RunError, run as add_parts};
pub use breadboard::breadboard_line;
pub use catalog::Version;
pub use circuit::{
    Amount, BoardFacts, BoardLed, EnginePart, FilePath, Gap, Instance, MapPart, Netlist, PartData,
    PinWhy, Placement, Reading, RefMap, Supply, View, Wire, board_facts, one_part,
};
pub use diagnostic::{Code, Diagnostic, Lint};
pub use files::Files;
pub use holes::{holes_text, strip_text};
pub use model::{PART_FORMATS, SCENE_FORMATS, STOP_PS, is_id};
pub use project::{BLOCK, REGISTRY, folder_hash, is_registry_name, part_file, shell_join};
pub use published::is_published;
pub use summary::{
    BlockCard, Buy, Card, CardFile, Choice, Doc, Folder, Holes, Literal, Missing, NotSimulated,
    Param, Pin, Rating, Real, Threshold, block_card, builtin_block, builtin_blocks, card, cards,
    catalog_cards, ids_with_segment, missing_fields, near_names, param_sentence, search,
    search_blocks, si_amount,
};

/// Findings the part-file reader reports for `parts/<id>/part.jsonc` in `files`. [`read`] uses
/// that reader on a part a project installed. Empty when the file is missing or not UTF-8.
pub fn part_findings(id: &str, files: &Folder) -> Vec<Diagnostic> {
    let Some(text) = utf8_file(files, "part.jsonc") else {
        return Vec::new();
    };
    project::part_diagnostics(id, &text)
}

/// A finding at each required param of `parts/<id>/part.jsonc` in `files` that has no default
/// and whose `min_exclusive` and `max_exclusive` hold no value a scene could set: no finite
/// number, or for a duration no whole number of picoseconds, zero or more. [`read`] refuses every
/// value such a param is set to. Empty when the file is missing, not UTF-8, or does not read.
pub fn unusable_params(id: &str, files: &Folder) -> Vec<Diagnostic> {
    let Some(text) = utf8_file(files, "part.jsonc") else {
        return Vec::new();
    };
    project::unusable_params(id, &text)
}

/// Findings the scene reader reports for `blocks/<id>/scene.jsonc` in `files`. [`read`] uses
/// that reader on a scene. Empty when the file is missing or not UTF-8.
pub fn block_findings(id: &str, files: &Folder) -> Vec<Diagnostic> {
    let Some(text) = utf8_file(files, "scene.jsonc") else {
        return Vec::new();
    };
    project::scene_diagnostics(id, &text)
}

fn utf8_file(files: &Folder, rel: &str) -> Option<String> {
    let text = std::str::from_utf8(files.get(rel)?).ok()?;
    Some(files::strip_bom(text.to_string()))
}

/// Type-checks a scene and every part file it reaches. On success, the netlist the engine
/// opens. Otherwise every finding, sorted by file, line and column.
pub fn read(files: &Files, scene: &Path) -> Result<Netlist, Vec<Diagnostic>> {
    compile(files, scene, false, false, None).map(|(netlist, _)| netlist)
}

/// The catalog part ids a scene names, including ids reached through other parts.
/// A comment is not a name. The ids are in order. A missing catalog part is one of
/// the ids, and so is a modified, outdated or newer copy. An id the catalog does not have, and any
/// other finding, is the error.
pub fn catalog_ids(files: &Files, scene: &Path) -> Result<Vec<String>, Vec<Diagnostic>> {
    let mut findings = diagnostic::Findings::default();
    let project = match project::load(files, scene, &mut findings) {
        Ok(project) => project,
        Err(sources) => return Err(findings.into_diagnostics(&sources)),
    };
    let diagnostics = findings.into_diagnostics(&project.sources);
    let blocked = project.unknown_part
        || diagnostics.iter().any(|d| {
            !matches!(
                d.code,
                Code::PartMissing | Code::PartModified | Code::PartOutdated | Code::PartNewer
            )
        });
    if blocked {
        Err(diagnostics)
    } else {
        Ok(project.catalog_ids.into_iter().collect())
    }
}

/// What a viewer draws of a scene, read only as far as its instances, their params and its
/// layout: no param, firmware or ground check and no netlist. A page draws one part this way,
/// on its own. A file that does not parse, or a part the scene names that is missing, is the
/// error.
pub fn view(files: &Files, scene: &Path) -> Result<View, Vec<Diagnostic>> {
    let mut findings = diagnostic::Findings::default();
    let project = match project::load(files, scene, &mut findings) {
        Ok(project) => project,
        Err(sources) => return Err(findings.into_diagnostics(&sources)),
    };
    if !findings.is_empty() {
        return Err(findings.into_diagnostics(&project.sources));
    }
    Ok(circuit::view_only(&project))
}

/// [`read`], then on a scene that reads, the lint rules' warnings, sorted by file, line and
/// column. Otherwise `read`'s errors.
pub fn lint(files: &Files, scene: &Path) -> Result<Vec<Diagnostic>, Vec<Diagnostic>> {
    compile(files, scene, true, false, None).map(|(_, warnings)| warnings)
}

/// How `orbe check` and `orbe open` name the two counts: the parts the scene lists, and the
/// engine parts those expand to. A breadboard is a scene part and adds no engine part.
pub fn part_counts(scene: usize, engine: usize) -> String {
    let noun = |n: usize| if n == 1 { "part" } else { "parts" };
    format!(
        "{scene} scene {}, expanding to {engine} engine {}",
        noun(scene),
        noun(engine)
    )
}

#[cfg(test)]
mod tests {
    use super::part_counts;

    #[test]
    fn part_counts_names_the_scene_parts_and_the_engine_parts_they_expand_to() {
        assert_eq!(
            part_counts(5, 19),
            "5 scene parts, expanding to 19 engine parts"
        );
        assert_eq!(
            part_counts(1, 1),
            "1 scene part, expanding to 1 engine part"
        );
    }
}

/// What [`read_placed`] gives: the findings, and each path a file param takes.
#[derive(Clone, Debug, Default)]
pub struct Placed {
    /// Every finding, sorted as [`read`] sorts them. Empty when the scene reads.
    pub findings: Vec<Diagnostic>,
    /// Each path a file param takes in the scene and the part files it reaches, the way
    /// [`read`] resolves them: the value an instance gets, or else its part's default. Empty when
    /// the scene does not load.
    pub file_paths: Vec<FilePath>,
}

/// [`read`], with the file paths it reads. With `placing`, only the placement of the scene's
/// parts is read, before a scene gives them what only it can: a value for each required param
/// with no default, and a ground. An instance the scene places may then leave such a param out.
/// Its model is not whole, so neither it nor a part under it that the param passes to goes to
/// the engine. A scene that names no ground, with parts that name none, is not a finding. Every
/// other finding is `read`'s.
pub fn read_placed(files: &Files, scene: &Path, placing: bool) -> Placed {
    let mut file_paths = Vec::new();
    let findings = match compile(files, scene, false, placing, Some(&mut file_paths)) {
        Ok(_) => Vec::new(),
        Err(found) => found,
    };
    Placed {
        findings,
        file_paths,
    }
}

fn compile(
    files: &Files,
    scene: &Path,
    lint: bool,
    placing: bool,
    paths: Option<&mut Vec<FilePath>>,
) -> Result<(Netlist, Vec<Diagnostic>), Vec<Diagnostic>> {
    let mut findings = diagnostic::Findings::default();
    let project = match project::load(files, scene, &mut findings) {
        Ok(project) => project,
        Err(sources) => return Err(findings.into_diagnostics(&sources)),
    };
    let mut warnings = diagnostic::Findings::default();
    let netlist = circuit::compile(
        &project,
        &mut findings,
        lint.then_some(&mut warnings),
        placing,
        paths,
    );
    match netlist {
        Some(netlist) if findings.is_empty() => {
            Ok((netlist, warnings.into_diagnostics(&project.sources)))
        }
        _ => Err(findings.into_diagnostics(&project.sources)),
    }
}

#[cfg(test)]
mod parse_once {
    use std::collections::BTreeMap;
    use std::path::Path;

    use crate::files::Files;

    #[test]
    fn a_read_parses_each_file_once() {
        let scene = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../blocks/nano-light-comparator/scene.jsonc");
        crate::syntax::take_parses();
        let read = crate::read(&Files::disk(), &scene);
        assert!(read.is_ok(), "the comparator scene reads");
        let mut counts = BTreeMap::<String, usize>::new();
        for path in crate::syntax::take_parses() {
            *counts.entry(path).or_default() += 1;
        }
        let repeated: Vec<_> = counts.iter().filter(|(_, n)| **n > 1).collect();
        assert!(
            !counts.is_empty(),
            "a read parsed nothing, so the count did not see the parse"
        );
        assert!(
            repeated.is_empty(),
            "a read parsed a file more than once: {repeated:?}"
        );
    }

    #[test]
    fn a_cached_parse_still_reports_its_findings() {
        let root = Path::new("proj");
        let path = Path::new("proj/dup.jsonc");
        let text = "{\n  \"color\": \"red\",\n  \"color\": \"blue\"\n}\n";
        let mut sources = crate::source::Sources::new(root);
        let file = sources.add(path, text.to_string());

        crate::syntax::take_parses();
        let mut quiet = crate::diagnostic::Findings::default();
        let first = crate::syntax::parse(&sources, file, &mut quiet);
        assert!(first.is_some(), "a duplicate key still parses");

        let mut reported = crate::diagnostic::Findings::default();
        let second = crate::syntax::parse(&sources, file, &mut reported);
        assert!(second.is_some(), "the cached parse returns the tree");

        let parses = crate::syntax::take_parses();
        assert_eq!(
            parses,
            vec!["dup.jsonc".to_string()],
            "the second call is a cache hit"
        );

        let got = reported.into_diagnostics(&sources);
        assert_eq!(
            got,
            quiet.into_diagnostics(&sources),
            "a cache hit reports the same findings as the first parse"
        );
        assert!(
            got.iter().any(|d| {
                d.code == crate::Code::Duplicate && d.message == "`color` is listed twice"
            }),
            "the hit reports the duplicate key, got {got:?}"
        );
    }
}
