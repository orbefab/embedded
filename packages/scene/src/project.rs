//! Finding and reading every part file a scene uses, and checking the catalog copies against
//! `parts.lock`. A Block reads its catalog ids from this binary's catalog instead.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::catalog::{self, Version};
use crate::diagnostic::{Code, Findings};
use crate::files::{Files, Kind};
use crate::model::{Comp, Part, Scene, TypeRef, decode_part, decode_scene};
use crate::name::did_you_mean;
use crate::source::{FileId, Sources, Span, display};
use crate::syntax::{self, Node};

/// The official catalog's registry, the one this repo's `parts/` publishes to.
pub const REGISTRY: &str = "https://github.com/orbefab/embedded/releases/download/parts";

/// The file that makes a folder a Block.
pub const BLOCK: &str = "block.jsonc";

/// Where a project reads the catalog ids its files name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Store {
    /// The copies in `parts/<id>/`, each with a line in `parts.lock`.
    Copies,
    /// A Block: every id is the copy in this binary's catalog. A Block holds no `parts/` and no
    /// `parts.lock`, so it always runs the catalog it ships with.
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
            Store::Catalog => catalog::file(id, rel).is_some(),
        }
    }

    pub(crate) fn read(self, files: &Files, root: &Path, id: &str, rel: &str) -> Option<Vec<u8>> {
        match self {
            Store::Copies => files.read(&root.join("parts").join(id).join(rel)).ok(),
            Store::Catalog => catalog::file(id, rel).map(<[u8]>::to_vec),
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
    /// Catalog copies with children and no model that differ from `parts.lock`.
    pub(crate) modified: Vec<String>,
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

struct LockLine {
    version: String,
    registry: String,
    hash: String,
    span: Span,
}

struct Loader<'a> {
    files: &'a Files,
    store: Store,
    out: &'a mut Findings,
    sources: Sources,
    scene_path: PathBuf,
    lock: BTreeMap<String, LockLine>,
    parts: Vec<Part>,
    /// `None` for a file that did not read, so it is reported once.
    loaded: BTreeMap<PathBuf, Option<usize>>,
    stack: Vec<PathBuf>,
    types: BTreeMap<(Owner, usize), usize>,
    missing: Vec<(String, Span)>,
    /// The catalog ids whose `part.jsonc` read, the copies `check_copies` compares.
    used: BTreeSet<String>,
    /// `orbe add`, run from where the scene was named, as hints print it.
    add: String,
}

/// Reads the scene and everything it uses. When the scene itself does not read, the files
/// read so far, so its findings can be placed.
pub(crate) fn load<'f>(
    files: &'f Files,
    scene_path: &Path,
    out: &mut Findings,
) -> Result<Project<'f>, Sources> {
    let add = add_command(hint_dir(files, scene_path).as_deref());
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
        lock: BTreeMap::new(),
        parts: Vec::new(),
        loaded: BTreeMap::new(),
        stack: vec![scene_path.clone()],
        types: BTreeMap::new(),
        missing: Vec::new(),
        used: BTreeSet::new(),
        add,
    };
    let scene = loader.read(&scene_path).and_then(|file| {
        let tree = syntax::parse(&loader.sources, file, loader.out)?;
        decode_scene(&loader.sources, file, &tree, loader.out)
    });
    let Some(scene) = scene else {
        return Err(loader.sources);
    };
    match store {
        Store::Copies => loader.read_lock(&root),
        Store::Catalog => loader.no_copies(&root, scene.file),
    }
    let types: Vec<_> = scene.comp.parts.iter().map(|i| i.ty.clone()).collect();
    loader.resolve(Owner::Scene, &scene_path, false, types);
    loader.report_missing();
    let modified = loader.check_copies(&root);
    let mut catalog_ids = loader.used;
    let mut unknown_part = false;
    for (id, _) in loader.missing {
        if catalog::find(&id).is_some() {
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
        modified,
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
                    format!("make it readable, such as `chmod u+r {shown}`")
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

    /// A Block reads every id from the catalog, so a copy or a lock beside it would not be read.
    fn no_copies(&mut self, root: &Path, scene: FileId) {
        for held in ["parts.lock", "parts"] {
            if self.files.link_kind(&root.join(held)).is_some() {
                self.out.push(
                    Code::Role,
                    Span::start_of(scene),
                    format!(
                        "this folder holds {BLOCK}, so it is a Block, which reads its parts from the catalog built into orbe, and it also holds `{held}`"
                    ),
                    format!(
                        "remove `{held}`, or remove {BLOCK} to make the folder a project that reads its own copies"
                    ),
                );
            }
        }
    }

    /// A Block's catalog part, placed at the path its copy would have in a project.
    fn read_catalog(&mut self, path: &Path, id: &str) -> Option<FileId> {
        let bytes = catalog::file(id, "part.jsonc")?;
        let text = String::from_utf8_lossy(bytes).into_owned();
        Some(self.sources.add(path, text))
    }

    fn read_lock(&mut self, root: &Path) {
        let path = root.join("parts.lock");
        if !self.files.is_file(&path) {
            return;
        }
        let Some(file) = self.read(&path) else { return };
        let text = self.sources.file(file).text.clone();
        let mut start = 0;
        for line in text.split_inclusive('\n') {
            let span = Span {
                file,
                start,
                end: start + line.trim_end().len(),
            };
            start += line.len();
            match lock_fields(line) {
                Ok(None) => {}
                Ok(Some(fields)) => {
                    self.lock.insert(
                        fields.id,
                        LockLine {
                            version: fields.version,
                            registry: fields.registry,
                            hash: fields.hash,
                            span,
                        },
                    );
                }
                Err(()) => self.out.push(
                    Code::Syntax,
                    span,
                    "a parts.lock line is `<id> <version> <registry> sha256:<hash>`",
                    format!(
                        "`orbe add` writes this file. rebuild it with `{} --relock`",
                        self.add
                    ),
                ),
            }
        }
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
                    let path = root.join("parts").join(&id).join("part.jsonc");
                    let path = match self.store {
                        Store::Copies
                            if self.lock.contains_key(&id) && self.files.is_file(&path) =>
                        {
                            self.files.real(&path).unwrap_or(path)
                        }
                        Store::Catalog if catalog::file(&id, "part.jsonc").is_some() => path,
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
        let parts = self.sources.root().join("parts");
        if let Some(id) = self.lock.keys().find(|id| path.starts_with(parts.join(id))) {
            self.out.push(
                Code::Type,
                span,
                format!("`{rel}` is inside the catalog part `{id}`"),
                format!("name it by its id, as \"type\": \"{id}\""),
            );
            return None;
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
            let mut part = syntax::parse(&self.sources, file, self.out)
                .and_then(|tree| decode_part(&self.sources, file, label, &tree, self.out));
            if let Some(part) = &mut part {
                part.reported = self.out.since(before);
            }
            // A sealed copy that no longer reads was edited, and the fix is to restore it.
            if part.is_none()
                && catalog
                && catalog::find(&id).is_none_or(|original| is_sealed(&original.tree))
            {
                self.out.rehint_since(before, &self.restore(&id, None));
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
        let mut found: Vec<(String, Span, String, bool)> = Vec::new();
        for (id, span) in &self.missing {
            if found.iter().any(|(seen, ..)| seen == id) {
                continue;
            }
            let (why, overwrite) = self.why_missing(&root, id);
            found.push((id.clone(), *span, why, overwrite));
        }
        let mut ids: Vec<&str> = found
            .iter()
            .map(|(id, ..)| id.as_str())
            .filter(|id| catalog::find(id).is_some())
            .collect();
        ids.sort();
        let overwrite = found
            .iter()
            .any(|(id, _, _, overwrite)| *overwrite && ids.contains(&id.as_str()));
        let command = format!(
            "{} {}{}",
            self.add,
            ids.join(" "),
            if overwrite { " --overwrite" } else { "" }
        );
        let known: Vec<String> = self.lock.keys().cloned().chain(catalog::ids()).collect();
        let parts = root.join("parts");
        let blocked = self.store == Store::Copies
            && self.files.link_kind(&parts).is_some()
            && !self.files.is_dir(&parts);
        for (id, span, why, _) in &found {
            let hint = if blocked {
                "move `parts` out of the way, since part folders go in a folder named `parts`. then check again for the command that adds them".into()
            } else if ids.contains(&id.as_str()) {
                format!("run `{command}`")
            } else {
                let close = did_you_mean(id, known.iter().map(String::as_str), "");
                format!("{close}the catalog has no part named `{id}`")
            };
            self.out
                .push(Code::PartMissing, *span, format!("part `{id}` {why}"), hint);
        }
    }

    /// What is at `parts/<id>` for an id that did not resolve, and whether `orbe add` has to
    /// replace it.
    fn why_missing(&self, root: &Path, id: &str) -> (String, bool) {
        if self.store == Store::Catalog {
            return ("is not in the catalog this Block reads".into(), false);
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
            return (format!("cannot be read, because `parts` {what}"), false);
        }
        let folder = parts.join(id);
        let listed = self.lock.contains_key(id);
        let Some(kind) = self.files.link_kind(&folder) else {
            let why = if listed {
                "has a parts.lock line but no folder"
            } else {
                "is not in this project's parts.lock"
            };
            return (why.into(), false);
        };
        if kind == Kind::Link {
            return (
                format!("is parts/{id}, which is a link, not a folder"),
                true,
            );
        }
        if kind != Kind::Dir {
            return (
                format!("is parts/{id}, which is a file, not a folder"),
                true,
            );
        }
        if !self.files.is_file(&folder.join("part.jsonc")) {
            return (
                format!("has a folder, parts/{id}, with no part.jsonc"),
                true,
            );
        }
        let differs = catalog::find(id)
            .is_none_or(|original| hash_folder(self.files, &folder).ok() != Some(original.hash));
        ("is not in this project's parts.lock".into(), differs)
    }

    /// The command that puts the catalog's copy back. It names the version when the copy's own
    /// version is in another major version, since `--overwrite` alone keeps the major.
    fn restore(&self, id: &str, copy: Option<&Node>) -> String {
        if catalog::find(id).is_none() {
            return format!(
                "this binary's catalog has no `{id}` to restore. copy it to a path to make it your own"
            );
        }
        format!(
            "restore it with `{}`, or copy it to a path to make it your own",
            self.restore_command(id, copy)
        )
    }

    fn restore_command(&self, id: &str, copy: Option<&Node>) -> String {
        let named = catalog::find(id).and_then(|original| {
            let copy = Version::parse(copy?.get("version")?.str()?)?;
            let theirs = Version::parse(&original.version)?;
            (copy.major != theirs.major).then_some(original.version)
        });
        match named {
            Some(version) => format!("{} {id}@{version} --overwrite", self.add),
            None => format!("{} {id} --overwrite", self.add),
        }
    }

    /// Hashes the folder of every catalog copy the scene uses. A missing folder is the
    /// scene's `part_missing`, and a lock line for an id the scene does not use is not
    /// checked. A sealed catalog part is compared with the copy this binary embeds, and a lock
    /// line for one of those ids has to name that copy. Returns the unsealed copies that
    /// differ and whose lock line still matches.
    fn check_copies(&mut self, root: &Path) -> Vec<String> {
        let mut modified = Vec::new();
        let lock = std::mem::take(&mut self.lock);
        for (id, line) in &lock {
            if !self.used.contains(id) {
                continue;
            }
            let folder = root.join("parts").join(id);
            let path = folder.join("part.jsonc");
            let file = self.sources.find(&self.files.real(&path).unwrap_or(path));
            let at = file.map_or(line.span, Span::start_of);
            let hash = match hash_folder(self.files, &folder) {
                Ok(hash) => Some(hash),
                Err((path, error)) => {
                    self.unhashable(id, &folder, &path, &error, at);
                    None
                }
            };
            let tree = file.and_then(|f| syntax::parse(&self.sources, f, &mut Findings::default()));
            let Some(original) = catalog::find(id) else {
                if self.note_unlisted(id, line, file, &tree, hash) {
                    modified.push(id.clone());
                }
                continue;
            };
            let diff = tree
                .as_ref()
                .and_then(|t| first_difference(t, &original.tree, String::new()));
            let bytes_differ = hash
                .as_ref()
                .is_some_and(|h| h != &line.hash || h != &original.hash);
            let restore = self.restore(id, tree.as_ref());
            if is_sealed(&original.tree) && (diff.is_some() || bytes_differ) {
                let (message, span) = match diff {
                    Some((field, span)) => (
                        format!(
                            "sealed part `{id}` differs from `{id}@{}` in `{field}`",
                            original.version
                        ),
                        span,
                    ),
                    None => match original.files_differing(self.files, &folder) {
                        Some(files) => (
                            format!(
                                "sealed part `{id}` differs from `{id}@{}` in {files}",
                                original.version
                            ),
                            file.map_or(line.span, Span::start_of),
                        ),
                        None => (
                            format!(
                                "parts.lock's line for `{id}` does not match `{id}@{}`",
                                original.version
                            ),
                            line.span,
                        ),
                    },
                };
                self.out.push(Code::PartModified, span, message, restore);
                continue;
            }
            let indexed = line.registry == REGISTRY && line.version == original.version;
            let copy_version = tree
                .as_ref()
                .and_then(|t| t.get("version").and_then(Node::str));
            let version_problem =
                tree.as_ref().is_some() && copy_version != Some(line.version.as_str());
            if !indexed || version_problem {
                let message = if !indexed {
                    format!(
                        "parts.lock lists `{id}` at `{}` from `{}`, and this binary's catalog has `{id}@{}`",
                        line.version, line.registry, original.version
                    )
                } else {
                    format!(
                        "parts.lock lists `{id}` at `{}`, and the copy's version is `{}`",
                        line.version,
                        copy_version.unwrap_or("")
                    )
                };
                self.out.push(
                    Code::PartModified,
                    line.span,
                    message,
                    format!(
                        "this binary knows `{id}@{}` from the official registry. restore the catalog's copy with `{} {id}@{} --overwrite`",
                        original.version, self.add, original.version
                    ),
                );
                continue;
            }
            if !is_sealed(&original.tree) && (diff.is_some() || bytes_differ) {
                modified.push(id.clone());
            }
        }
        self.lock = lock;
        modified
    }

    /// A file in a copy that cannot be read. A link on its path is named, since a link that
    /// loops is the usual cause.
    fn unhashable(&mut self, id: &str, folder: &Path, path: &Path, error: &io::Error, at: Span) {
        let root = self.sources.root().to_path_buf();
        let link = path
            .ancestors()
            .take_while(|p| *p != folder && p.starts_with(folder))
            .filter(|p| self.files.link_kind(p) == Some(Kind::Link))
            .last();
        let (shown, hint) = match link {
            Some(link) => {
                let link = display(&root, link);
                (
                    link.clone(),
                    format!(
                        "remove the link that loops at {link}, or restore the folder with `{} {id} --overwrite`",
                        self.add
                    ),
                )
            }
            None if error.kind() == io::ErrorKind::PermissionDenied => {
                let shown = display(&root, path);
                let hint = format!("make it readable, such as `chmod u+r {shown}`");
                (shown, hint)
            }
            None => (
                display(&root, path),
                format!("restore the folder with `{} {id} --overwrite`", self.add),
            ),
        };
        self.out.push(
            Code::Io,
            at,
            format!("cannot hash parts/{id}: cannot read {shown}: {error}"),
            hint,
        );
    }

    /// An id this binary's catalog does not have. A differing copy is sealed from its own
    /// file, which is all there is to judge. Returns whether the copy differs and is not sealed.
    fn note_unlisted(
        &mut self,
        id: &str,
        line: &LockLine,
        file: Option<FileId>,
        tree: &Option<Node>,
        hash: Option<String>,
    ) -> bool {
        let Some(hash) = hash else {
            return false;
        };
        if hash == line.hash {
            return false;
        }
        let sealed = tree
            .as_ref()
            .is_none_or(|t| t.get("model").is_some() || t.get("parts").is_none());
        if !sealed {
            return true;
        }
        self.out.push(
            Code::PartModified,
            file.map_or(line.span, Span::start_of),
            format!("part `{id}` does not match its parts.lock line"),
            self.restore(id, tree.as_ref()),
        );
        false
    }
}

/// A catalog part is sealed when it has a model, or when it has no children of its own.
pub(crate) fn is_sealed(tree: &Node) -> bool {
    tree.get("model").is_some() || tree.get("parts").is_none()
}

/// One `parts.lock` line. The hash includes the `sha256:` prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LockFields {
    pub(crate) id: String,
    pub(crate) version: String,
    pub(crate) registry: String,
    pub(crate) hash: String,
}

#[cfg(unix)]
impl LockFields {
    pub(crate) fn line(&self) -> String {
        format!(
            "{} {} {} {}",
            self.id, self.version, self.registry, self.hash
        )
    }
}

/// `Ok(None)` is a blank line. `Err` is a line that is not the lock format.
pub(crate) fn lock_fields(line: &str) -> Result<Option<LockFields>, ()> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    match fields.as_slice() {
        [] => Ok(None),
        [id, version, registry, hash] if hash.starts_with("sha256:") => Ok(Some(LockFields {
            id: (*id).to_string(),
            version: (*version).to_string(),
            registry: (*registry).to_string(),
            hash: (*hash).to_string(),
        })),
        _ => Err(()),
    }
}

/// Lock text sorted by id, one line each, with a trailing newline when there is a line.
#[cfg(unix)]
pub(crate) fn render_lock(lines: &BTreeMap<String, LockFields>) -> String {
    let mut out = String::new();
    for line in lines.values() {
        out.push_str(&line.line());
        out.push('\n');
    }
    out
}

/// The first field where a copy's `part.jsonc` differs from the catalog's, as a dotted path,
/// with the span of the copy's value there.
fn first_difference(copy: &Node, original: &Node, path: String) -> Option<(String, Span)> {
    let join = |key: &str| {
        if path.is_empty() {
            key.to_string()
        } else {
            format!("{path}.{key}")
        }
    };
    match (
        copy.props(),
        original.props(),
        copy.items(),
        original.items(),
    ) {
        (Some(copy_props), Some(original_props), _, _) => {
            for prop in copy_props {
                match original.get(&prop.key) {
                    Some(theirs) => {
                        if let Some(found) = first_difference(&prop.value, theirs, join(&prop.key))
                        {
                            return Some(found);
                        }
                    }
                    None => return Some((join(&prop.key), prop.key_span)),
                }
            }
            original_props
                .iter()
                .find(|p| copy.get(&p.key).is_none())
                .map(|p| (join(&p.key), copy.span))
        }
        (_, _, Some(mine), Some(theirs)) if mine.len() == theirs.len() => mine
            .iter()
            .zip(theirs)
            .enumerate()
            .find_map(|(i, (a, b))| first_difference(a, b, format!("{path}[{i}]"))),
        _ => (copy.to_value() != original.to_value()).then_some((path, copy.span)),
    }
}

/// The SHA-256 over a folder's files, paths included: one line `<sha256>  <path>` per file,
/// sorted by path, with `/` between path parts. Written as `sha256:<hex>`, as in `parts.lock`.
pub fn folder_hash(dir: &Path) -> io::Result<String> {
    hash_folder(&Files::disk(), dir).map_err(|(_, error)| error)
}

/// [`folder_hash`] in `files`, with the path that could not be read.
fn hash_folder(files: &Files, dir: &Path) -> Result<String, (PathBuf, io::Error)> {
    let mut found = Vec::new();
    collect(files, dir, dir, &mut found)?;
    Ok(hash_files(
        found.iter().map(|(p, b)| (p.as_str(), b.as_slice())),
    ))
}

fn collect(
    files: &Files,
    root: &Path,
    dir: &Path,
    found: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), (PathBuf, io::Error)> {
    let at = |path: &Path, error| (path.to_path_buf(), error);
    for path in files.list(dir).map_err(|e| at(dir, e))? {
        if files.is_dir(&path) {
            collect(files, root, &path, found)?;
        } else {
            let bytes = files.read(&path).map_err(|e| at(&path, e))?;
            found.push((display(root, &path), bytes));
        }
    }
    Ok(())
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
