//! Reads a `scene.jsonc` and the part files it uses into the engine's netlist v0, or into
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
mod buy;
mod catalog;
mod circuit;
mod diagnostic;
mod files;
mod kinds;
mod model;
mod name;
mod params;
mod project;
mod reuse;
mod shape;
mod source;
mod summary;
mod syntax;
mod units;

use std::path::Path;

#[cfg(unix)]
pub use add::{AddCode, Reply, Request, RunError, run as add_parts};
pub use catalog::Version;
pub use circuit::{
    Amount, EnginePart, Instance, MapPart, Netlist, PartData, PinWhy, Placement, Reading, RefMap,
    View, Wire,
};
pub use diagnostic::{Code, Diagnostic, Lint};
pub use files::Files;
pub use model::{STOP_PS, is_id};
pub use project::{BLOCK, REGISTRY, folder_hash, is_registry_name, part_file};
pub use summary::{
    BlockCard, Buy, Card, CardFile, Choice, Doc, Folder, Literal, Missing, NotSimulated, On, Param,
    Pin, Rating, Real, block_card, card, cards, catalog_cards, ids_with_segment, missing_fields,
    near_names, param_sentence, search, si_amount,
};

/// Type-checks a scene and every part file it reaches. On success, the netlist the engine
/// opens. Otherwise every finding, sorted by file, line and column.
pub fn read(files: &Files, scene: &Path) -> Result<Netlist, Vec<Diagnostic>> {
    compile(files, scene, false).map(|(netlist, _)| netlist)
}

/// The catalog part ids a scene names, including ids reached through other parts.
/// A comment is not a name. The ids are in order. A missing catalog part is one of
/// the ids, and so is a modified copy. An id the catalog does not have, and any
/// other finding, is the error.
pub fn catalog_ids(files: &Files, scene: &Path) -> Result<Vec<String>, Vec<Diagnostic>> {
    let mut findings = diagnostic::Findings::default();
    let project = match project::load(files, scene, &mut findings) {
        Ok(project) => project,
        Err(sources) => return Err(findings.into_diagnostics(&sources)),
    };
    let diagnostics = findings.into_diagnostics(&project.sources);
    let blocked = project.unknown_part
        || diagnostics
            .iter()
            .any(|d| !matches!(d.code, Code::PartMissing | Code::PartModified));
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
    compile(files, scene, true).map(|(_, warnings)| warnings)
}

fn compile(
    files: &Files,
    scene: &Path,
    lint: bool,
) -> Result<(Netlist, Vec<Diagnostic>), Vec<Diagnostic>> {
    let mut findings = diagnostic::Findings::default();
    let project = match project::load(files, scene, &mut findings) {
        Ok(project) => project,
        Err(sources) => return Err(findings.into_diagnostics(&sources)),
    };
    let mut warnings = diagnostic::Findings::default();
    let netlist = circuit::compile(&project, &mut findings, lint.then_some(&mut warnings));
    match netlist {
        Some(netlist) if findings.is_empty() => {
            Ok((netlist, warnings.into_diagnostics(&project.sources)))
        }
        _ => Err(findings.into_diagnostics(&project.sources)),
    }
}
