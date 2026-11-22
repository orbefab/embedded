//! Finding and reading every part file a scene uses. A project reads `parts/<id>/` as it is.
//! A Block reads its catalog ids from this binary's catalog instead.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::diagnostic::{Code, Diagnostic, Findings};
use crate::files::{Files, Kind};
use crate::model::{Comp, ParamKind, Part, Scene, TypeRef, decode_part, decode_scene};
use crate::name::did_you_mean;
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
    /// The copies in `parts/<id>/`, as the project left them.
    Copies,
    /// A Block: every id is the copy in this binary's catalog. A Block holds no `parts/`, so it
    /// always runs the catalog it ships with.
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
        match self {
            Store::Copies => files.is_file(&root.join("parts").join(id).join(rel)),
            Store::Catalog => files.catalog().file(id, rel).is_some(),
        }
    }

    pub(crate) fn read(self, files: &Files, root: &Path, id: &str, rel: &str) -> Option<Vec<u8>> {
        match self {
            Store::Copies => files.read(&root.join("parts").join(id).join(rel)).ok(),
            Store::Catalog => files.catalog().file(id, rel).map(<[u8]>::to_vec),
        }
    }
}

/// The file `rel` of the catalog part `id` as the project in `root` reads it: its copy in
/// `parts/<id>/`, or in a Block, this binary's catalog's.
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
    missing: Vec<(String, Span)>,
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

/// A finding at each required number param with no default in the text of
/// `parts/<id>/part.jsonc` whose bounds hold no value a scene could set. Empty when the file does
/// not read, since [`part_diagnostics`] reports why.
pub(crate) fn unusable_params(id: &str, text: &str) -> Vec<Diagnostic> {
    reader_diagnostics(
        &format!("parts/{id}/part.jsonc"),
        text,
        |sources, file, out| {
            let mut quiet = Findings::default();
            let Some(part) = parse_part(sources, file, id.to_string(), &mut quiet, "orbe add")
            else {
                return;
            };
            for decl in part.params.iter().filter(|decl| decl.default.is_none()) {
                let ParamKind::Number { unit, range } = decl.kind else {
                    continue;
                };
                if let Some((why, hint)) = crate::params::no_value(unit, range) {
                    out.push(
                        Code::Param,
                        decl.span,
                        format!("param `{}` takes no value: {why}", decl.name),
                        hint,
                    );
                }
            }
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
    let mut unknown_part = false;
    for (id, _) in loader.missing {
        if files.catalog().find(&id).is_some() {
            catalog_ids.insert(id);
        } else {
            unknown_part = true;
        }
    }
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
                    "this folder holds {BLOCK}, so it is a Block, which reads its parts from the catalog built into orbe, and it also holds `parts`"
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
                    let folder = root.join("parts").join(&id);
                    let path = folder.join("part.jsonc");
                    if self.store == Store::Copies && self.parts_unreadable {
                        continue;
                    }
                    if self.store == Store::Copies
                        && self.files.is_dir(&folder)
                        && let Err(error) = self.files.list(&folder)
                    {
                        self.note_unreadable(&folder, &error);
                        continue;
                    }
                    let path = match self.store {
                        Store::Copies if self.files.is_file(&path) => {
                            self.files.real(&path).unwrap_or(path)
                        }
                        // The folder is there and part.jsonc is not. Reading it is an io error.
                        Store::Copies if self.files.is_dir(&folder) => path,
                        Store::Catalog
                            if self.files.catalog().file(&id, "part.jsonc").is_some() =>
                        {
                            path
                        }
                        _ => {
                            self.missing.push((id, span));
                            continue;
                        }
                    };
                    let found = self.load_part(&path, id.clone(), true, span);
                    if found.is_some() {
                        self.used.insert(id);
                    }
                    found
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

    /// One finding per missing id. Every catalog id shares one `orbe add` command that adds
    /// them all. A suggestion is only for an id the catalog does not have, since that command
    /// cannot add it.
    fn report_missing(&mut self) {
        let root = self.sources.root().to_path_buf();
        let mut found: Vec<(String, Span, String)> = Vec::new();
        for (id, span) in &self.missing {
            if found.iter().any(|(seen, ..)| seen == id) {
                continue;
            }
            found.push((id.clone(), *span, self.why_missing(&root, id)));
        }
        let mut ids: Vec<&str> = found
            .iter()
            .map(|(id, ..)| id.as_str())
            .filter(|id| self.files.catalog().find(id).is_some())
            .collect();
        ids.sort();
        let specs: Vec<String> = ids.iter().map(|id| (*id).to_string()).collect();
        let command = self.missing_command(&root, &specs);
        let known: Vec<String> = self.files.catalog().ids().collect();
        let parts = root.join("parts");
        let blocked = self.store == Store::Copies
            && self.files.link_kind(&parts).is_some()
            && !self.files.is_dir(&parts);
        for (id, span, why) in &found {
            let hint = if blocked {
                "move `parts` out of the way, since part folders go in a folder named `parts`. then check again for the command that adds them".into()
            } else if let Some(command) = command.as_ref().filter(|_| ids.contains(&id.as_str())) {
                format!("run `{command}`")
            } else {
                let close = did_you_mean(id, known.iter().map(String::as_str), "");
                format!("{close}the catalog has no part named `{id}`")
            };
            self.out
                .push(Code::PartMissing, *span, format!("part `{id}` {why}"), hint);
        }
    }

    /// The command that repairs these missing catalog ids. On disk it is the command
    /// `orbe add` of those ids would print, so a folder already there is named there too.
    fn missing_command(&self, root: &Path, specs: &[String]) -> Option<String> {
        self.disk_repair(root, specs)
            .or_else(|| add_writing(&self.add, specs))
    }

    /// The command `orbe add` of these ids would print, when this project is a folder on disk.
    /// A memory set and a non-unix build keep the ids check already gathered.
    #[cfg(unix)]
    fn disk_repair(&self, root: &Path, specs: &[String]) -> Option<String> {
        if self.store == Store::Copies && self.files.is_disk() {
            crate::add::repair_command(root, &self.add, specs)
        } else {
            None
        }
    }

    #[cfg(not(unix))]
    fn disk_repair(&self, _root: &Path, _specs: &[String]) -> Option<String> {
        None
    }

    /// What is at `parts/<id>` for an id that did not resolve.
    fn why_missing(&self, root: &Path, id: &str) -> String {
        if self.store == Store::Catalog {
            return "is not in the catalog this Block reads".into();
        }
        let parts = root.join("parts");
        if let Some(kind) = self.files.link_kind(&parts)
            && !self.files.is_dir(&parts)
        {
            let what = if kind == Kind::File {
                "is a file, not a folder"
            } else {
                "is not a folder"
            };
            return format!("cannot be read, because `parts` {what}");
        }
        let folder = parts.join(id);
        let Some(kind) = self.files.link_kind(&folder) else {
            return "is not in parts/".into();
        };
        if kind == Kind::Link {
            return format!("is parts/{id}, which is a link, not a folder");
        }
        if kind != Kind::Dir {
            return format!("is parts/{id}, which is a file, not a folder");
        }
        "is not in parts/".into()
    }
}

/// A registry's name: `@` and an id, such as `@orbefab`.
pub fn is_registry_name(text: &str) -> bool {
    text.strip_prefix('@').is_some_and(crate::model::is_id)
}

/// The SHA-256 over a folder's published files, paths included: one line `<sha256>  <path>`
/// per file, sorted by path, with `/` between path parts. Written as `sha256:<hex>`. Build
/// output is left out. The paths are [`crate::published::published_paths`], the same set the
/// catalog embeds.
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

/// The `orbe add` command that writes `specs`. Each spec is an id, or `id@version`. `None`
/// when `specs` is empty, so a hint never prints `orbe add` with no part id. Check and add
/// both build that command here, from the ids the add would write.
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
pub(crate) fn shell_word(word: &str) -> String {
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
