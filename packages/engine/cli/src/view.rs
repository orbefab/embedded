//! `sim view`: what each display part's glass showed at the end of a run, one PNG a part.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use orbe_circuit::st77916;
use orbe_engine_core::blob::Sha256;
use orbe_engine_core::display::Flags;
use orbe_engine_core::name::Name;
use serde::{Deserialize, Serialize};

use crate::{Failure, png};

/// `sim view` stdout body: the PNG it wrote for each display part, by part name.
#[derive(Serialize)]
pub struct ViewReport {
    pub out: String,
    pub images: BTreeMap<String, String>,
}

/// The `observations.json` keys `sim view` reads. A reader ignores the others.
#[derive(Deserialize)]
struct Observations {
    #[serde(default)]
    displays: BTreeMap<String, Entry>,
}

#[derive(Deserialize)]
struct Entry {
    kind: String,
    flags: Flags,
    image: String,
    sha256: String,
}

/// Reads the run folder `run` and writes `display-<part>.png` into `out` for each display part
/// its `observations.json` names, what [`st77916::glass`] makes of the part's flags and frame
/// memory file. It checks every part before it writes any file.
pub fn view(run: &Path, out: &Path) -> Result<ViewReport, Failure> {
    let path = run.join("observations.json");
    let text = fs::read(&path).map_err(|e| failure(&path, e.to_string()))?;
    let observations: Observations =
        serde_json::from_slice(&text).map_err(|e| failure(&path, e.to_string()))?;
    let mut pngs = Vec::new();
    for (part, entry) in observations.displays {
        if Name::new(&part).is_err() {
            return Err(failure(&path, format!("{part:?} is not a part name")));
        }
        if entry.kind != "st77916" {
            let message = format!(
                "{part} has kind {}, which sim view does not draw",
                entry.kind
            );
            return Err(failure(&path, message));
        }
        let image_name = format!("traces/display-{part}.rgb565");
        if entry.image != image_name {
            return Err(failure(
                &path,
                format!("{part}'s image is not {image_name}"),
            ));
        }
        let image = run.join(&image_name);
        let frame = fs::read(&image).map_err(|e| failure(&image, e.to_string()))?;
        if Sha256::of(&frame).to_string() != entry.sha256 {
            return Err(failure(
                &image,
                format!("its SHA-256 is not {part}'s sha256"),
            ));
        }
        let (width, height) = (st77916::WIDTH, st77916::HEIGHT);
        if frame.len() != usize::from(width) * usize::from(height) * 2 {
            return Err(failure(
                &image,
                format!("not {width} by {height} pixels of two bytes"),
            ));
        }
        let pixels = st77916::glass(entry.flags, &frame);
        let png = png::rgba(u32::from(width), u32::from(height), &pixels);
        let file = out.join(format!("display-{part}.png"));
        pngs.push((part, file, png));
    }
    if !pngs.is_empty() {
        fs::create_dir_all(out).map_err(|e| failure(out, e.to_string()))?;
    }
    let mut images = BTreeMap::new();
    for (part, file, png) in pngs {
        fs::write(&file, png).map_err(|e| failure(&file, e.to_string()))?;
        images.insert(part, file.display().to_string());
    }
    Ok(ViewReport {
        out: out.display().to_string(),
        images,
    })
}

fn failure(path: &Path, message: String) -> Failure {
    Failure {
        message,
        path: Some(path.display().to_string()),
    }
}
