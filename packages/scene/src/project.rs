//! Finding and reading every part file a scene uses. A project reads `parts/<id>/` for an id
//! the scene names. A child is read from `<that part's folder>/parts/<id>/` when that file is
//! there, and otherwise from the project's `parts/<id>/`. A Block, a folder that holds
//! `block.jsonc`, reads every id from the `parts/` beside `blocks/`, or from the part folders
//! handed in with the files.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::diagnostic::{Code, Diagnostic, Findings};
use crate::files::{Files, Kind};
use crate::model::{Comp, Part, Scene, TypeRef, decode_part, decode_scene};
use crate::source::{FileId, Sources, Span, display};
use crate::syntax;

/// The name of the official registry, the one this repo's `parts/` publishes to. A client maps
/// the name to a URL when it fetches.
pub const REGISTRY: &str = "@orbefab";

/// The file that makes a folder a Block.
pub const BLOCK: &str = "block.jsonc";

/// Where a project reads the catalog ids its files name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Store {
    /// The copies in `parts/<id>/`, as the project left them. A child is read from the folder
    /// of the part that names it.
    Copies,
    /// A Block. Ids are read flat from the part folders handed in, or from the `parts/`
    /// beside `blocks/` when none were handed in.
    Catalog,
}

impl Store {
    pub(crate) fn of(files: &Files, root: &Path) -> Store {
        if files.is_file(&root.join(BLOCK)) {
            Store::Catalog
        } else {
            Store::Copies
        }
    }

    pub(crate) fn has(self, files: &Files, root: &Path, id: &str, rel: &str) -> bool {
        self.read(files, root, id, rel).is_some()
    }

    pub(crate) fn read(self, files: &Files, root: &Path, id: &str, rel: &str) -> Option<Vec<u8>> {
        if self == Store::Catalog
            && let Some(bytes) = files.catalog().file(id, rel)
        {
            return Some(bytes.to_vec());
        }
        let folder = match self {
            Store::Copies => root.join("parts"),
            Store::Catalog => block_parts(root)?,
        };
        files.read(&folder.join(id).join(rel)).ok()
    }
}

/// The `parts/` a Block reads: the folder beside `blocks/`, two levels above the Block.
fn block_parts(root: &Path) -> Option<PathBuf> {
    Some(root.parent()?.parent()?.join("parts"))
}

/// The file `rel` of part `id` as the project in `root` reads it. A project reads
/// `parts/<id>/<rel>`, and a path such as `parts/<board>/parts/<child>/<file>` is `id` of the
/// first folder with the rest as `rel`. A Block reads the `parts/` beside `blocks/`, or the
/// part folders handed in with the files.
pub fn part_file(files: &Files, root: &Path, id: &str, rel: &str) -> Option<Vec<u8>> {
    Store::of(files, root).read(files, root, id, rel)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Owner {
    Scene,
    Part(usize),
}

pub(crate) struct Project<'f> {
    pub(crate) files: &'f Files,
    pub(crate) store: Store,
    pub(crate) sources: Sources,
    pub(crate) scene: Scene,
    pub(crate) parts: Vec<Part>,
    /// The part each instance's `type` names, by its owner and its index in `parts`.
    pub(crate) types: BTreeMap<(Owner, usize), usize>,
    /// Catalog ids the scene names, including ids named inside the parts it reaches.
    /// A missing catalog id is included. An id the catalog does not have is not.
    pub(crate) catalog_ids: BTreeSet<String>,
    /// A `type` names an id the catalog does not have.
    pub(crate) unknown_part: bool,
}

impl Project<'_> {
    pub(crate) fn comp(&self, owner: Owner) -> &Comp {
        match owner {
            Owner::Scene => &self.scene.comp,
            Owner::Part(id) => &self.parts[id].comp,
        }
    }

    pub(crate) fn file(&self, owner: Owner) -> FileId {
        match owner {
            Owner::Scene => self.scene.file,
            Owner::Part(id) => self.parts[id].file,
        }
    }
}

struct Loader<'a> {
    files: &'a Files,
    store: Store,
    out: &'a mut Findings,
    sources: Sources,
    scene_path: PathBuf,
    parts: Vec<Part>,
    /// `None` for a file that did not read, so it is reported once.
    loaded: BTreeMap<PathBuf, Option<usize>>,
    stack: Vec<PathBuf>,
    types: BTreeMap<(Owner, usize), usize>,
    missing: Vec<Gap>,
    /// The catalog ids whose `part.jsonc` read.
    used: BTreeSet<String>,
    /// `orbe add`, run from where the scene was named, as hints print it.
    add: String,
    /// The project folder as the user named it, when that is not the current folder.
    /// Printed commands are relative to where `orbe` ran, and `chmod` has no `-C`.
    hint_at: Option<PathBuf>,
    /// `parts/` is a directory this process cannot list. Catalog ids are not reported
    /// missing: the folder is there, and a missing-part hint would not be the repair.
    parts_unreadable: bool,
}

/// Parses `file` and reads it as a part. This is the part-file reader `orbe check` runs.
pub(crate) fn parse_part(
    sources: &Sources,
    file: FileId,
    label: String,
    out: &mut Findings,
    add: &str,
) -> Option<Part> {
    syntax::parse(sources, file, out)
        .and_then(|tree| decode_part(sources, file, label, &tree, out, add))
}

/// Parses `file` and reads it as a scene. This is the scene reader `orbe check` runs.
pub(crate) fn parse_scene(sources: &Sources, file: FileId, out: &mut Findings) -> Option<Scene> {
    syntax::parse(sources, file, out).and_then(|tree| decode_scene(sources, file, &tree, out))
}

/// Findings [`parse_part`] reports for the text of `parts/<id>/part.jsonc`.
pub(crate) fn part_diagnostics(id: &str, text: &str) -> Vec<Diagnostic> {
    reader_diagnostics(
        &format!("parts/{id}/part.jsonc"),
        text,
        |sources, file, out| {
            parse_part(sources, file, id.to_string(), out, "orbe add");
        },
    )
}

/// Findings [`parse_scene`] reports for the text of `blocks/<id>/scene.jsonc`.
pub(crate) fn scene_diagnostics(id: &str, text: &str) -> Vec<Diagnostic> {
    reader_diagnostics(
        &format!("blocks/{id}/scene.jsonc"),
        text,
        |sources, file, out| {
            parse_scene(sources, file, out);
        },
    )
}

fn reader_diagnostics(
    rel: &str,
    text: &str,
    decode: impl FnOnce(&Sources, FileId, &mut Findings),
) -> Vec<Diagnostic> {
    let root = Path::new("registry");
    let mut sources = Sources::new(root);
    let file = sources.add(&root.join(rel), text.to_string());
    let mut out = Findings::default();
    decode(&sources, file, &mut out);
    out.into_diagnostics(&sources)
}

/// Reads the scene and everything it uses. When the scene itself does not read, the files
/// read so far, so its findings can be placed.
pub(crate) fn load<'f>(
    files: &'f Files,
    scene_path: &Path,
    out: &mut Findings,
) -> Result<Project<'f>, Sources> {
    let hint_at = hint_dir(files, scene_path);
    let add = add_command(hint_at.as_deref());
    let scene_path = files
        .real(scene_path)
        .unwrap_or_else(|_| scene_path.to_path_buf());
    let root = scene_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let store = Store::of(files, &root);
    let mut loader = Loader {
        files,
        store,
        out,
        sources: Sources::new(&root),
        scene_path: scene_path.clone(),
        parts: Vec::new(),
        loaded: BTreeMap::new(),
        stack: vec![scene_path.clone()],
        types: BTreeMap::new(),
        missing: Vec::new(),
        used: BTreeSet::new(),
        add,
        hint_at,
        parts_unreadable: false,
    };
    let scene = loader
        .read(&scene_path)
        .and_then(|file| parse_scene(&loader.sources, file, loader.out));
    let Some(scene) = scene else {
        return Err(loader.sources);
    };
    match store {
        Store::Copies => loader.note_parts_dir(&root),
        Store::Catalog => loader.no_copies(&root, scene.file),
    }
    let types: Vec<_> = scene.comp.parts.iter().map(|i| i.ty.clone()).collect();
    loader.resolve(Owner::Scene, &scene_path, false, types);
    loader.report_missing();
    let mut catalog_ids = loader.used;
    for gap in &loader.missing {
        catalog_ids.insert(gap.id.clone());
    }
    let unknown_part = false;
    Ok(Project {
        files,
        store,
        sources: loader.sources,
        scene,
        parts: loader.parts,
        types: loader.types,
        catalog_ids,
        unknown_part,
    })
}

impl Loader<'_> {
    fn read(&mut self, path: &Path) -> Option<FileId> {
        match self.files.read_to_string(path) {
            Ok(text) => Some(self.sources.add(path, text)),
            Err(error) => {
                let file = self.sources.add(path, String::new());
                let shown = display(self.sources.root(), path);
                let hint = if error.kind() == io::ErrorKind::PermissionDenied {
                    chmod_hint(self.hint_at.as_deref(), Path::new(&shown), false)
                } else {
                    "check that the path exists and is readable".into()
                };
                self.out.push(
                    Code::Io,
                    Span::start_of(file),
                    format!("cannot read {shown}: {error}"),
                    hint,
                );
                None
            }
        }
    }

    /// `parts/` exists and cannot be listed. One finding names that directory. A later
    /// catalog id is not `part_missing`: the copy may be there, and `orbe add` cannot see it.
    fn note_parts_dir(&mut self, root: &Path) {
        let parts = root.join("parts");
        if !self.files.is_dir(&parts) {
            return;
        }
        if let Err(error) = self.files.list(&parts) {
            self.parts_unreadable = true;
            self.note_unreadable(&parts, &error);
        }
    }

    /// A directory that could not be listed, or a file that could not be read. A directory
    /// needs search permission as well as read, so its hint is `chmod u+rx`.
    fn note_unreadable(&mut self, path: &Path, error: &io::Error) {
        let file = self.sources.add(path, String::new());
        let shown = display(self.sources.root(), path);
        let directory = self.files.is_dir(path);
        let hint = if error.kind() == io::ErrorKind::PermissionDenied {
            chmod_hint(self.hint_at.as_deref(), Path::new(&shown), directory)
        } else {
            "check that the path exists and is readable".into()
        };
        self.out.push(
            Code::Io,
            Span::start_of(file),
            format!("cannot read {shown}: {error}"),
            hint,
        );
    }

    /// A Block reads every id from the catalog, so a `parts/` folder beside it would not be read.
    fn no_copies(&mut self, root: &Path, scene: FileId) {
        if self.files.link_kind(&root.join("parts")).is_some() {
            self.out.push(
                Code::Role,
                Span::start_of(scene),
                format!(
                    "this folder holds {BLOCK}, so it is a Block, which reads the parts/ beside blocks/, and it also holds `parts`"
                ),
                format!(
                    "remove `parts`, or remove {BLOCK} to make the folder a project that reads its own copies"
                ),
            );
        }
    }

    /// A Block's catalog part, placed at the path its copy would have in a project.
    fn read_catalog(&mut self, path: &Path, id: &str) -> Option<FileId> {
        let bytes = self.files.catalog().file(id, "part.jsonc")?;
        let text = String::from_utf8_lossy(bytes).into_owned();
        Some(self.sources.add(path, text))
    }

    /// Resolves each `type` in a file's `parts`, reading every part file it reaches.
    fn resolve(
        &mut self,
        owner: Owner,
        owner_path: &Path,
        catalog_owner: bool,
        types: Vec<Option<(TypeRef, Span)>>,
    ) {
        let root = self.sources.root().to_path_buf();
        for (index, ty) in types.into_iter().enumerate() {
            let Some((ty, span)) = ty else { continue };
            let found = match ty {
                TypeRef::Id(id) => {
                    if self.store == Store::Copies
                        && self.parts_unreadable
                        && matches!(owner, Owner::Scene)
                    {
                        continue;
                    }
                    let given = self.store == Store::Catalog && !self.files.catalog().is_empty();
                    if given {
                        let path = root.join("parts").join(&id).join("part.jsonc");
                        if self.files.catalog().file(&id, "part.jsonc").is_none() {
                            self.missing.push(Gap::new(
                                id.clone(),
                                span,
                                format!("parts/{id}"),
                                GapKind::Absent,
                            ));
                            continue;
                        }
                        let found = self.load_part(&path, id.clone(), true, span);
                        if found.is_some() {
                            self.used.insert(id.clone());
                        }
                        found
                    } else {
                        let mut opened = None;
                        let mut gap = None;
                        for folder in self.id_folders(owner, owner_path, &id) {
                            if self.files.is_dir(&folder)
                                && let Err(error) = self.files.list(&folder)
                            {
                                self.note_unreadable(&folder, &error);
                                opened = None;
                                gap = None;
                                break;
                            }
                            let path = folder.join("part.jsonc");
                            if self.files.is_file(&path) {
                                opened = Some(self.files.real(&path).unwrap_or(path));
                                break;
                            }
                            if gap.is_none() {
                                let shown = display(&root, &folder);
                                gap =
                                    Some(Gap::new(id.clone(), span, shown, self.gap_kind(&folder)));
                            }
                        }
                        let Some(path) = opened else {
                            if let Some(gap) = gap {
                                self.missing.push(gap);
                            }
                            continue;
                        };
                        let found = self.load_part(&path, id.clone(), false, span);
                        if found.is_some() {
                            self.used.insert(id.clone());
                        }
                        found
                    }
                }
                TypeRef::Path(rel) => {
                    if catalog_owner {
                        self.out.push(
                            Code::Type,
                            span,
                            format!(
                                "a catalog part names its children by id, not by the path `{rel}`"
                            ),
                            "name the child by its catalog id, as \"type\": \"led-5mm\"".into(),
                        );
                        continue;
                    }
                    let Some(path) = self.part_path(owner_path, &rel, span) else {
                        continue;
                    };
                    self.load_part(&path, display(&root, &path), false, span)
                }
            };
            if let Some(part) = found {
                self.types.insert((owner, index), part);
            }
        }
    }

    fn part_path(&mut self, owner_path: &Path, rel: &str, span: Span) -> Option<PathBuf> {
        let mut path = owner_path.parent().unwrap_or(Path::new(".")).join(rel);
        if self.files.is_dir(&path) {
            path = path.join("part.jsonc");
        }
        let is_json = path
            .extension()
            .is_some_and(|e| e == "jsonc" || e == "json");
        if !self.files.is_file(&path) || !is_json {
            self.out.push(
                Code::Type,
                span,
                format!("`{rel}` is not a part file or a folder holding part.jsonc"),
                "a path is relative to the file that names it".into(),
            );
            return None;
        }
        let path = self.files.real(&path).unwrap_or(path);
        if path == self.scene_path {
            self.out.push(
                Code::Role,
                span,
                format!("`{rel}` is the scene; a scene is not a part and cannot be a `type`"),
                "point `type` at a part file".into(),
            );
            return None;
        }
        // `parts/<id>/` is the project's copy of that part. A scene names it by id.
        let parts = self.sources.root().join("parts");
        if let Ok(rest) = path.strip_prefix(&parts)
            && let Some(std::path::Component::Normal(name)) = rest.components().next()
        {
            let id = name.to_string_lossy();
            let folder = parts.join(name);
            if self.files.is_dir(&folder) || self.files.is_file(&folder.join("part.jsonc")) {
                self.out.push(
                    Code::Type,
                    span,
                    format!("`{rel}` is inside the part `{id}`"),
                    format!("name it by its id, as \"type\": \"{id}\""),
                );
                return None;
            }
        }
        Some(path)
    }

    fn load_part(
        &mut self,
        path: &Path,
        label: String,
        catalog: bool,
        span: Span,
    ) -> Option<usize> {
        if self.stack.iter().any(|p| p == path) {
            let chain: Vec<String> = self
                .stack
                .iter()
                .chain([&path.to_path_buf()])
                .map(|p| display(self.sources.root(), p))
                .collect();
            self.out.push(
                Code::Cycle,
                span,
                format!("this part reaches itself: {}", chain.join(" -> ")),
                "a part's type cannot reach its own file".into(),
            );
            return None;
        }
        if let Some(loaded) = self.loaded.get(path) {
            return *loaded;
        }
        let id = label.clone();
        let file = match self.store {
            Store::Catalog if catalog => self.read_catalog(path, &id),
            _ => self.read(path),
        };
        let part = file.and_then(|file| {
            let before = self.out.len();
            let mut part = parse_part(&self.sources, file, label, self.out, &self.add);
            if let Some(part) = &mut part {
                part.reported = self.out.since(before);
            }
            part
        });
        let Some(part) = part else {
            self.loaded.insert(path.to_path_buf(), None);
            return None;
        };
        let types: Vec<_> = part.comp.parts.iter().map(|i| i.ty.clone()).collect();
        self.parts.push(part);
        let id = self.parts.len() - 1;
        self.loaded.insert(path.to_path_buf(), Some(id));
        self.stack.push(path.to_path_buf());
        self.resolve(Owner::Part(id), path, catalog, types);
        self.stack.pop();
        Some(id)
    }

    /// Where an id is read, first choice first. A scene's ids are `parts/<id>/`. A part's
    /// child is `<that part's folder>/parts/<id>/` when the part file is `part.jsonc`, and
    /// the project's `parts/<id>/` when that copy is not there. A loose part file has no
    /// folder of its own, so its children are read from the project. A Block reads one flat
    /// `parts/` beside `blocks/`.
    fn id_folders(&self, owner: Owner, owner_path: &Path, id: &str) -> Vec<PathBuf> {
        let root = self.sources.root();
        if self.store == Store::Catalog {
            let base = block_parts(root).unwrap_or_else(|| root.join("parts"));
            return vec![base.join(id)];
        }
        let project = root.join("parts").join(id);
        if matches!(owner, Owner::Scene) {
            return vec![project];
        }
        let nested = owner_path
            .file_name()
            .is_some_and(|name| name == "part.jsonc")
            .then(|| {
                owner_path
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join("parts")
                    .join(id)
            });
        match nested {
            Some(folder) if folder != project => vec![folder, project],
            _ => vec![project],
        }
    }

    fn gap_kind(&self, folder: &Path) -> GapKind {
        match self.files.link_kind(folder) {
            Some(Kind::Link) => GapKind::Link,
            Some(Kind::Dir) => GapKind::NoFile,
            Some(_) => GapKind::File,
            None => GapKind::Absent,
        }
    }

    /// One finding per missing id. A scene-level id that is simply absent shares one
    /// `orbe add`. A folder with no `part.jsonc`, a file or a link in its place, and a child
    /// that is not inside its parent, each name the top part folder to remove.
    fn report_missing(&mut self) {
        let root = self.sources.root().to_path_buf();
        let mut found: Vec<Gap> = Vec::new();
        for gap in &self.missing {
            if found
                .iter()
                .any(|seen| seen.id == gap.id && seen.place == gap.place)
            {
                continue;
            }
            found.push(gap.clone());
        }
        let mut absent: Vec<String> = found
            .iter()
            .filter(|gap| gap.kind == GapKind::Absent && gap.scene_level())
            .map(|gap| gap.id.clone())
            .collect();
        absent.sort();
        absent.dedup();
        let command = add_writing(&self.add, &absent);
        let parts = root.join("parts");
        let parts_kind = self.files.link_kind(&parts);
        let blocked =
            self.store == Store::Copies && parts_kind.is_some() && !self.files.is_dir(&parts);
        let catalog = self.store == Store::Catalog;
        for gap in &found {
            let (message, hint) = gap_text(
                gap,
                command.as_deref(),
                blocked,
                parts_kind == Some(Kind::File),
                catalog,
                &self.add,
            );
            self.out.push(Code::PartMissing, gap.span, message, hint);
        }
    }
}

#[derive(Clone)]
struct Gap {
    id: String,
    span: Span,
    /// The folder that should hold the part, with `/` between names, from the project.
    place: String,
    kind: GapKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GapKind {
    Absent,
    NoFile,
    File,
    Link,
}

impl Gap {
    fn new(id: String, span: Span, place: String, kind: GapKind) -> Gap {
        Gap {
            id,
            span,
            place,
            kind,
        }
    }

    /// A scene names this id at `parts/<id>/`, rather than a child nested further down.
    fn scene_level(&self) -> bool {
        self.place == format!("parts/{}", self.id)
    }
}

fn gap_text(
    gap: &Gap,
    command: Option<&str>,
    blocked: bool,
    parts_file: bool,
    catalog: bool,
    add: &str,
) -> (String, String) {
    if catalog {
        return (
            format!("part `{}` is not in the catalog this Block reads", gap.id),
            format!("the catalog has no part named `{}`", gap.id),
        );
    }
    if blocked && gap.scene_level() {
        let what = if parts_file {
            "is a file, not a folder"
        } else {
            "is not a folder"
        };
        return (
            format!("part `{}` cannot be read, because `parts` {what}", gap.id),
            "move `parts` out of the way, since part folders go in a folder named `parts`. then check again for the command that adds them".into(),
        );
    }
    let top = gap
        .place
        .strip_prefix("parts/")
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(gap.id.as_str());
    let again = add_writing(add, &[top.to_string()]).unwrap_or_else(|| format!("{add} {top}"));
    match gap.kind {
        GapKind::NoFile => (
            format!("{} has no part.jsonc", gap.place),
            format!("remove parts/{top} and run `{again}`"),
        ),
        GapKind::File => (
            format!(
                "part `{}` is {}, which is a file, not a folder",
                gap.id, gap.place
            ),
            format!("remove parts/{top} and run `{again}`"),
        ),
        GapKind::Link => (
            format!(
                "part `{}` is {}, which is a link, not a folder",
                gap.id, gap.place
            ),
            format!("remove parts/{top} and run `{again}`"),
        ),
        GapKind::Absent if !gap.scene_level() => {
            let message = match gap.place.rfind("/parts/") {
                Some(at) => {
                    let parent = &gap.place[..at];
                    let child = &gap.place[at + "/parts/".len()..];
                    format!("{parent} has no parts/{child}")
                }
                None => format!("part `{}` is not in parts/", gap.id),
            };
            (message, format!("remove parts/{top} and run `{again}`"))
        }
        GapKind::Absent => (
            format!("part `{}` is not in parts/", gap.id),
            match command {
                Some(command) => format!("run `{command}`"),
                None => format!("run `{again}`"),
            },
        ),
    }
}

/// A registry's name: `@` and an id, such as `@orbefab`.
pub fn is_registry_name(text: &str) -> bool {
    text.strip_prefix('@').is_some_and(crate::model::is_id)
}

/// The SHA-256 over a folder's published files, paths included: one line `<sha256>  <path>`
/// per file, sorted by path, with `/` between path parts. Written as `sha256:<hex>`. Build
/// output is left out. The paths are [`crate::published::published_paths`], the same set a
/// registry record publishes.
pub fn folder_hash(dir: &Path) -> io::Result<String> {
    let paths = crate::published::published_paths(dir)?;
    let mut found = Vec::new();
    for path in &paths {
        let bytes = std::fs::read(path)?;
        found.push((display(dir, path), bytes));
    }
    Ok(hash_files(
        found
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
    ))
}

pub(crate) fn hash_files<'a>(files: impl Iterator<Item = (&'a str, &'a [u8])>) -> String {
    let mut lines: Vec<String> = files
        .map(|(path, bytes)| format!("{}  {path}\n", hex(&Sha256::digest(bytes))))
        .collect();
    lines.sort_by(|a, b| a[66..].cmp(&b[66..]));
    format!("sha256:{}", hex(&Sha256::digest(lines.concat().as_bytes())))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The `orbe add` command that writes `specs`. Each spec is an id. `None` when `specs` is
/// empty, so a hint never prints `orbe add` with no part id. Check and add both build that
/// command here, from the ids the add would write.
pub(crate) fn add_writing(add: &str, specs: &[String]) -> Option<String> {
    format_add(add, specs)
}

/// A printed `orbe add`. `add` is already `orbe add` or `orbe add -C <folder>`. Each id is
/// one shell word.
pub(crate) fn format_add(add: &str, ids: &[String]) -> Option<String> {
    let ids: Vec<&str> = ids
        .iter()
        .map(String::as_str)
        .filter(|id| !id.is_empty())
        .collect();
    if ids.is_empty() {
        return None;
    }
    let mut command = add.to_string();
    for id in ids {
        command.push(' ');
        command.push_str(&shell_word(id));
    }
    Some(command)
}

/// Shell words joined by one space.
pub fn shell_join(words: &[&str]) -> String {
    words
        .iter()
        .map(|word| shell_word(word))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `orbe add` as a hint prints it, with `-C <dir>` when a folder is named. A folder that starts
/// with `-` gets `./` first, so the argument parser reads it as the folder.
pub(crate) fn add_command(dir: Option<&Path>) -> String {
    match dir {
        Some(dir) if !dir.as_os_str().is_empty() => {
            let shown = dir.display().to_string();
            let shown = if shown.starts_with('-') {
                format!("./{shown}")
            } else {
                shown
            };
            format!("orbe add -C {}", shell_word(&shown))
        }
        _ => "orbe add".into(),
    }
}

/// The folder `orbe add` must name for the scene `orbe check` read, run from where check ran.
/// It is the folder check reads parts from, the scene's real folder, as the scene path named it
/// when that path names the same folder, else relative to the current folder. `None` when that
/// is the current folder and the scene is `scene.jsonc`, which `orbe add` finds there by itself.
fn hint_dir(files: &Files, scene: &Path) -> Option<PathBuf> {
    let typed = scene
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let real = files.real(scene).ok();
    let dir = match real.as_deref().and_then(Path::parent) {
        Some(real_dir) if files.real(typed).ok().as_deref() != Some(real_dir) => from_cwd(real_dir),
        _ => typed.to_path_buf(),
    };
    let found = real.as_deref().unwrap_or(scene).file_name() == Some("scene.jsonc".as_ref());
    (!found || dir != Path::new(".")).then_some(dir)
}

/// The `chmod` hint for `rel`, a path from the project root. `dir` is the project folder as
/// the user named it from where `orbe` ran, or `None` when that folder is the current one.
/// The command has to work from that directory: `chmod` has no `-C`.
pub(crate) fn chmod_hint(dir: Option<&Path>, rel: &Path, directory: bool) -> String {
    // A directory needs search permission to open what it holds. `u+r` lists nothing
    // the next command can open, so a directory's hint is `u+rx`.
    let mode = if directory { "u+rx" } else { "u+r" };
    let path = chmod_path(dir, rel);
    format!(
        "make it readable, such as `chmod {mode} {}`",
        quote_path(&path)
    )
}

/// `rel` joined onto the project folder, from where `orbe` ran. A path that starts with `-`
/// gets `./` first, so `chmod` does not read it as an option.
fn chmod_path(dir: Option<&Path>, rel: &Path) -> PathBuf {
    let dir = dir.filter(|dir| !dir.as_os_str().is_empty() && *dir != Path::new("."));
    let path = match dir {
        Some(dir) => dir.join(rel),
        None => rel.to_path_buf(),
    };
    if path.to_string_lossy().starts_with('-') {
        Path::new(".").join(path)
    } else {
        path
    }
}

fn quote_path(path: &Path) -> String {
    #[cfg(unix)]
    {
        shell_path(path.as_os_str())
    }
    #[cfg(not(unix))]
    {
        shell_word(&path.to_string_lossy())
    }
}

/// `dir`, an absolute path, relative to the current folder.
fn from_cwd(dir: &Path) -> PathBuf {
    let Some(cwd) = std::env::current_dir()
        .ok()
        .and_then(|cwd| cwd.canonicalize().ok())
    else {
        return dir.to_path_buf();
    };
    let shared = cwd
        .components()
        .zip(dir.components())
        .take_while(|(a, b)| a == b)
        .count();
    if shared == 0 {
        return dir.to_path_buf();
    }
    let mut rel = PathBuf::new();
    for _ in cwd.components().skip(shared) {
        rel.push("..");
    }
    for part in dir.components().skip(shared) {
        rel.push(part);
    }
    if rel.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        rel
    }
}

/// A word a POSIX shell reads back unchanged.
pub fn shell_word(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+:@%=,".contains(c));
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

/// A path as one word a POSIX shell reads back as that path. A path that is not UTF-8, or holds
/// a control character, is written `"$(printf '...')"` with those bytes as octal escapes. A
/// command substitution drops trailing newlines, so those follow it inside single quotes.
#[cfg(unix)]
pub(crate) fn shell_path(path: &std::ffi::OsStr) -> String {
    use std::os::unix::ffi::OsStrExt;
    if let Some(word) = path.to_str()
        && !word.chars().any(char::is_control)
    {
        return shell_word(word);
    }
    let bytes = path.as_bytes();
    let newlines = bytes
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\n')
        .count();
    let body = &bytes[..bytes.len() - newlines];
    let mut format = String::new();
    for chunk in body.utf8_chunks() {
        for c in chunk.valid().chars() {
            match c {
                '\\' => format.push_str("\\\\"),
                '%' => format.push_str("%%"),
                '\'' => format.push_str("'\\''"),
                c if c.is_control() => {
                    let mut utf8 = [0; 4];
                    for byte in c.encode_utf8(&mut utf8).bytes() {
                        format.push_str(&format!("\\{byte:03o}"));
                    }
                }
                c => format.push(c),
            }
        }
        for byte in chunk.invalid() {
            format.push_str(&format!("\\{byte:03o}"));
        }
    }
    let mut word = format!("\"$(printf '{format}')\"");
    if newlines > 0 {
        word.push_str(&format!("'{}'", "\n".repeat(newlines)));
    }
    word
}

#[cfg(test)]
mod example {
    use super::part_diagnostics;
    use crate::diagnostic::Code;

    fn lamp(params: &str, model: &str) -> String {
        format!(
            r#"{{
  "format": 1,
  "part": "lamp",
  "version": "0.1.0",
  "title": "Lamp",
  "license": "MIT",
  "pins": {{ "a": [1], "b": [2] }},
  "params": {{ {params} }},
  "model": {model}
}}
"#
        )
    }

    const RESISTOR: &str = r#"{ "kind": "resistor", "ratings": { "power": null } }"#;

    #[test]
    fn an_example_of_zero_on_a_resistance_above_zero_is_out_of_range() {
        let text = lamp(
            r#""resistance": { "unit": "ohm", "required": true, "min_exclusive": 0, "example": 0, "sets": "/resistance" }"#,
            RESISTOR,
        );
        let found = part_diagnostics("lamp", &text);
        assert!(
            found.iter().any(|d| {
                d.code == Code::Param
                    && d.message.contains("param `resistance` example:")
                    && d.message.contains("out of range")
            }),
            "{found:?}"
        );
    }

    #[test]
    fn an_example_beside_a_default_is_a_syntax_error_and_not_also_checked() {
        let text = lamp(
            r#""resistance": { "unit": "ohm", "default": "220", "example": 0, "min_exclusive": 0, "sets": "/resistance" }"#,
            RESISTOR,
        );
        let found = part_diagnostics("lamp", &text);
        assert!(
            found
                .iter()
                .any(|d| d.code == Code::Syntax && d.message.contains("has no `example`")),
            "{found:?}"
        );
        assert!(
            found
                .iter()
                .all(|d| !d.message.contains("param `resistance` example:")),
            "{found:?}"
        );
    }

    #[test]
    fn an_example_file_outside_the_folder_is_refused() {
        let text = lamp(
            r#""note": { "file": "txt", "as": "text", "required": true, "example": "../x", "sets": "/note" }"#,
            r#"{ "kind": "resistor", "ratings": { "power": null }, "note": "" }"#,
        );
        let found = part_diagnostics("lamp", &text);
        assert!(
            found.iter().any(|d| {
                d.code == Code::Param && d.message.contains("`../x` is outside this part's folder")
            }),
            "{found:?}"
        );
    }

    #[test]
    fn a_through_param_does_not_read_an_example() {
        let text = lamp(r#""firmware": { "example": "example.hex" }"#, RESISTOR);
        let found = part_diagnostics("lamp", &text);
        assert!(
            found.iter().any(|d| {
                d.code == Code::Syntax && d.message.contains("does not read `example`")
            }),
            "{found:?}"
        );
    }

    #[test]
    fn a_string_example_on_a_number_param_is_refused() {
        let text = lamp(
            r#""resistance": { "unit": "ohm", "required": true, "example": "220", "sets": "/resistance" }"#,
            RESISTOR,
        );
        let found = part_diagnostics("lamp", &text);
        assert!(
            found.iter().any(|d| {
                d.code == Code::Param
                    && d.message
                        == "param `resistance` example: expected a JSON number, found a string"
                    && d.hint == "write a JSON number, in this param's unit"
            }),
            "{found:?}"
        );
    }

    #[test]
    fn an_example_a_scene_could_set_is_not_a_finding() {
        let text = lamp(
            r#""resistance": { "unit": "ohm", "required": true, "min_exclusive": 0, "example": 220, "sets": "/resistance" }"#,
            RESISTOR,
        );
        let found = part_diagnostics("lamp", &text);
        assert!(found.is_empty(), "{found:?}");
    }
}
