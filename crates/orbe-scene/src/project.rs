//! Finding and reading every part file a scene uses, and checking the catalog copies against
//! `parts.lock`.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::catalog;
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
}

/// Reads the scene and everything it uses. When the scene itself does not read, the files
/// read so far, so its findings can be placed.
pub(crate) fn load(scene_path: &Path, out: &mut Findings) -> Result<Project, Sources> {
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
                self.out.push(
                    Code::Io,
                    Span::start_of(file),
                    format!(
                        "cannot read {}: {error}",
                        display(self.sources.root(), path)
                    ),
                    None,
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
            let fields: Vec<&str> = line.split_whitespace().collect();
            match fields.as_slice() {
                [] => {}
                [id, version, registry, hash] if hash.starts_with("sha256:") => {
                    self.lock.insert(
                        id.to_string(),
                        LockLine {
                            version: version.to_string(),
                            registry: registry.to_string(),
                            hash: hash.to_string(),
                            span,
                        },
                    );
                }
                _ => self.out.push(
                    Code::Syntax,
                    span,
                    "a parts.lock line is `<id> <version> <registry> sha256:<hash>`",
                    Some("`orbe add` writes this file".into()),
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
                    self.load_part(&path.canonicalize().unwrap_or(path), id, true, span)
                }
                TypeRef::Path(rel) => {
                    if catalog_owner {
                        self.out.push(
                            Code::Type,
                            span,
                            format!(
                                "a catalog part names its children by id, not by the path `{rel}`"
                            ),
                            None,
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
                Some("a path is relative to the file that names it".into()),
            );
            return None;
        }
        let path = path.canonicalize().unwrap_or(path);
        if path == self.scene_path {
            self.out.push(
                Code::Role,
                span,
                format!("`{rel}` is the scene; a scene is not a part and cannot be a `type`"),
                None,
            );
            return None;
        }
        let parts = self.sources.root().join("parts");
        if let Some(id) = self.lock.keys().find(|id| path.starts_with(parts.join(id))) {
            self.out.push(
                Code::Type,
                span,
                format!("`{rel}` is inside the catalog part `{id}`"),
                Some(format!("name it by its id, as \"type\": \"{id}\"")),
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
                None,
            );
            return None;
        }
        if let Some(loaded) = self.loaded.get(path) {
            return *loaded;
        }
        let part = self.read(path).and_then(|file| {
            let tree = syntax::parse(&self.sources, file, self.out)?;
            decode_part(&self.sources, file, label, &tree, self.out)
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

    /// One finding per missing id, each with one `orbe add` command for all of them.
    fn report_missing(&mut self) {
        let mut ids: Vec<&str> = self.missing.iter().map(|(id, _)| id.as_str()).collect();
        ids.sort();
        ids.dedup();
        let command = format!("orbe add {}", ids.join(" "));
        let have: Vec<String> = self.lock.keys().cloned().chain(catalog::ids()).collect();
        let mut seen = Vec::new();
        for (id, span) in &self.missing {
            if seen.contains(id) {
                continue;
            }
            seen.push(id.clone());
            let close = did_you_mean(id, have.iter().map(String::as_str).filter(|h| h != id), "");
            let why = if self.lock.contains_key(id) {
                "has a parts.lock line but no folder"
            } else {
                "is not in this project's parts.lock"
            };
            self.out.push(
                Code::PartMissing,
                *span,
                format!("part `{id}` {why}"),
                Some(format!(
                    "{close}{}run `{command}`",
                    if close.is_empty() { "" } else { "otherwise " }
                )),
            );
        }
    }

    /// Hashes every folder `parts.lock` lists. A sealed copy that differs is `part_modified`.
    /// Returns the copies that differ and are not sealed.
    fn check_copies(&mut self, root: &Path) -> Vec<String> {
        let mut modified = Vec::new();
        let lock = std::mem::take(&mut self.lock);
        for (id, line) in &lock {
            let folder = root.join("parts").join(id);
            let Ok(hash) = folder_hash(&folder) else {
                continue;
            };
            let original = (line.registry == REGISTRY)
                .then(|| catalog::find(id))
                .flatten()
                .filter(|o| o.version == line.version);
            let differs = hash != line.hash || original.as_ref().is_some_and(|o| o.hash != hash);
            if !differs {
                continue;
            }
            let path = folder.join("part.jsonc");
            let file = self.file_for(&path.canonicalize().unwrap_or(path));
            let tree = file.and_then(|f| syntax::parse(&self.sources, f, &mut Findings::default()));
            let sealed = tree
                .as_ref()
                .is_none_or(|t| t.get("model").is_some() || t.get("parts").is_none());
            if !sealed {
                modified.push(id.clone());
                continue;
            }
            let hint = Some(format!(
                "restore it with `orbe add {id} --overwrite`, or copy it to a path to make it your own"
            ));
            let (message, span) = match (&original, &tree) {
                (Some(original), Some(tree)) => {
                    match first_difference(tree, &original.tree, String::new()) {
                        Some((field, span)) => (
                            format!(
                                "sealed part `{id}` differs from `{id}@{}` in `{field}`",
                                line.version
                            ),
                            span,
                        ),
                        None => match original.files_differing(&folder) {
                            Some(files) => (
                                format!(
                                    "sealed part `{id}` differs from `{id}@{}` in {files}",
                                    line.version
                                ),
                                Span::start_of(tree.span.file),
                            ),
                            None => (
                                format!(
                                    "parts.lock's line for `{id}` does not match its folder or `{id}@{}`",
                                    line.version
                                ),
                                line.span,
                            ),
                        },
                    }
                }
                _ => (
                    format!("sealed part `{id}` does not match its parts.lock line"),
                    file.map_or(line.span, Span::start_of),
                ),
            };
            self.out.push(Code::PartModified, span, message, hint);
        }
        self.lock = lock;
        modified
    }

    fn file_for(&mut self, path: &Path) -> Option<FileId> {
        self.sources.find(path).or_else(|| self.read(path))
    }
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
    let mut files = Vec::new();
    collect(dir, dir, &mut files)?;
    Ok(hash_files(
        files.iter().map(|(p, b)| (p.as_str(), b.as_slice())),
    ))
}

fn collect(root: &Path, dir: &Path, files: &mut Vec<(String, Vec<u8>)>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(root, &path, files)?;
        } else {
            files.push((display(root, &path), fs::read(&path)?));
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
