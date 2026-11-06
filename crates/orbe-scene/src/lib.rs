//! Reads a `scene.jsonc` and the part files it uses into the engine's netlist v0, or into
//! the findings that stop it.
//!
//! A scene's folder is its project root. An id such as `led-5mm` reads `parts/led-5mm/` there
//! and needs a line in `parts.lock`. A `./` or `../` path reads a part of the user's own,
//! relative to the file that names it.

mod add;
mod catalog;
mod circuit;
mod diagnostic;
mod kinds;
mod model;
mod name;
mod params;
mod project;
mod shape;
mod source;
mod syntax;
mod units;

use std::path::Path;

pub use add::{Reply, Request, RunError, run as add_parts};
pub use circuit::{Amount, EnginePart, MapPart, Netlist, Reading, RefMap};
pub use diagnostic::{Code, Diagnostic};
pub use model::STOP_PS;
pub use project::{REGISTRY, folder_hash};

/// Type-checks a scene and every part file it reaches. On success, the netlist the engine
/// opens. Otherwise every finding, sorted by file, line and column.
pub fn read(scene: &Path) -> Result<Netlist, Vec<Diagnostic>> {
    let mut findings = diagnostic::Findings::default();
    let project = match project::load(scene, &mut findings) {
        Ok(project) => project,
        Err(sources) => return Err(findings.into_diagnostics(&sources)),
    };
    let netlist = circuit::compile(&project, &mut findings);
    match netlist {
        Some(netlist) if findings.is_empty() => Ok(netlist),
        _ => Err(findings.into_diagnostics(&project.sources)),
    }
}
