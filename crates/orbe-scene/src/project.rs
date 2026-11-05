//! Finding and reading every part file a scene uses, and checking the catalog copies against
//! `parts.lock`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::catalog::{self, Version};
use crate::diagnostic::{Code, Findings};
use crate::model::{Comp, Part, Scene, TypeRef, decode_part, decode_scene};
use crate::name::did_you_mean;
use crate::source::{FileId, Sources, Span, display};
use crate::syntax::{self, Node};

/// The official catalog's registry, the one this repo's `parts/` publishes to.
pub const REGISTRY: &str = "https://github.com/orbefab/embedded/releases/download/parts";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Owner {
    Scene,
    Part(usize),
}

pub(crate) struct Project {
    pub(crate) sources: Sources,
    pub(crate) scene: Scene,
    pub(crate) parts: Vec<Part>,
    /// The part each instance's `type` names, by its owner and its index in `parts`.
    pub(crate) types: BTreeMap<(Owner, usize), usize>,
    /// Catalog copies with children and no model that differ from `parts.lock`.
    pub(crate) modified: Vec<String>,
}

impl Project {
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
pub(crate) fn load(scene_path: &Path, out: &mut Findings) -> Result<Project, Sources> {
    let add = add_command(scene_path.parent());
    let scene_path = scene_path
        .canonicalize()
        .unwrap_or_else(|_| scene_path.to_path_buf());
    let root = scene_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut loader = Loader {
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
    loader.read_lock(&root);
    let types: Vec<_> = scene.comp.parts.iter().map(|i| i.ty.clone()).collect();
    loader.resolve(Owner::Scene, &scene_path, false, types);
    loader.report_missing();
    let modified = loader.check_copies(&root);
    Ok(Project {
        sources: loader.sources,
        scene,
        parts: loader.parts,
        types: loader.types,
        modified,
    })
}

impl Loader<'_> {
    fn read(&mut self, path: &Path) -> Option<FileId> {
        match fs::read_to_string(path) {
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

    fn read_lock(&mut self, root: &Path) {
        let path = root.join("parts.lock");
        if !path.is_file() {
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
                    if !self.lock.contains_key(&id) || !path.is_file() {
                        self.missing.push((id, span));
                        continue;
                    }
                    let found = self.load_part(
                        &path.canonicalize().unwrap_or(path),
                        id.clone(),
                        true,
                        span,
                    );
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
        if path.is_dir() {
            path = path.join("part.jsonc");
        }
        let is_json = path
            .extension()
            .is_some_and(|e| e == "jsonc" || e == "json");
        if !path.is_file() || !is_json {
            self.out.push(
                Code::Type,
                span,
                format!("`{rel}` is not a part file or a folder holding part.jsonc"),
                "a path is relative to the file that names it".into(),
            );
            return None;
        }
        let path = path.canonicalize().unwrap_or(path);
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
        let part = self.read(path).and_then(|file| {
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
        for (id, span, why, _) in &found {
            let hint = if ids.contains(&id.as_str()) {
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
        let folder = root.join("parts").join(id);
        let listed = self.lock.contains_key(id);
        let Ok(meta) = fs::symlink_metadata(&folder) else {
            let why = if listed {
                "has a parts.lock line but no folder"
            } else {
                "is not in this project's parts.lock"
            };
            return (why.into(), false);
        };
        if meta.file_type().is_symlink() {
            return (
                format!("is parts/{id}, which is a link, not a folder"),
                true,
            );
        }
        if !meta.is_dir() {
            return (
                format!("is parts/{id}, which is a file, not a folder"),
                true,
            );
        }
        if !folder.join("part.jsonc").is_file() {
            return (
                format!("has a folder, parts/{id}, with no part.jsonc"),
                true,
            );
        }
        let differs = catalog::find(id)
            .is_none_or(|original| hash_folder(&folder).ok() != Some(original.hash));
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
            let file = self.sources.find(&path.canonicalize().unwrap_or(path));
            let at = file.map_or(line.span, Span::start_of);
            let hash = match hash_folder(&folder) {
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
                    None => match original.files_differing(&folder) {
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
            .filter(|p| p.is_symlink())
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
    hash_folder(dir).map_err(|(_, error)| error)
}

/// [`folder_hash`], with the path that could not be read.
fn hash_folder(dir: &Path) -> Result<String, (PathBuf, io::Error)> {
    let mut files = Vec::new();
    collect(dir, dir, &mut files)?;
    Ok(hash_files(
        files.iter().map(|(p, b)| (p.as_str(), b.as_slice())),
    ))
}

fn collect(
    root: &Path,
    dir: &Path,
    files: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), (PathBuf, io::Error)> {
    let at = |path: &Path, error| (path.to_path_buf(), error);
    for entry in fs::read_dir(dir).map_err(|e| at(dir, e))? {
        let path = entry.map_err(|e| at(dir, e))?.path();
        if path.is_dir() {
            collect(root, &path, files)?;
        } else {
            let bytes = fs::read(&path).map_err(|e| at(&path, e))?;
            files.push((display(root, &path), bytes));
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

/// `orbe add`, with `-C <dir>` when the scene was named from another folder. The folder is the
/// one the scene path was given in, so the command runs from where `orbe check` ran.
pub(crate) fn add_command(scene_dir: Option<&Path>) -> String {
    match scene_dir {
        Some(dir) if !dir.as_os_str().is_empty() && dir != Path::new(".") => {
            format!("orbe add -C {}", shell_word(&dir.display().to_string()))
        }
        _ => "orbe add".into(),
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
