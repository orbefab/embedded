//! `orbe add <block>` installs a Block. It copies the parts the scene names, as `orbe add <part>`
//! does, then writes the Block's scene, firmware, tests and README. A part id and a Block id
//! never match, so one word names one of them.
//!
//! Every check that can refuse runs before anything is written. A link where a directory would be
//! created or entered, at the temp file, or at a destination is refused, and the writer never
//! follows one. The project folder the user named may itself be a link. A file already there with
//! the Block's bytes is kept, so running the same install again finishes one that stopped part way.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io::{self, Write};
use std::path::Path;

use super::at::{Folder, Found};

use serde_json::{Value, json};

use super::{AddCode, AddReply, Again, Fault, Refusal, Reply, Request, RunError, io_at};
use crate::files::Files;
use crate::project::{self, BLOCK};

/// `block.jsonc` stays behind. It is what makes a folder a Block, and an installed project
/// reads its own part copies, so it must not be one.
const KEPT: [&str; 1] = [BLOCK];

/// A Block this binary was built with, from the repo's `blocks/`.
pub(super) struct Block {
    pub(super) id: &'static str,
    /// The files the install writes, by their path from the Block's folder.
    files: Vec<(&'static str, &'static [u8])>,
}

pub(crate) fn ids() -> impl Iterator<Item = &'static str> {
    crate::blocks::ids()
}

fn find(id: &str) -> Option<Block> {
    let id = ids().find(|block| *block == id)?;
    let prefix = format!("{id}/");
    let files = crate::blocks::FILES
        .iter()
        .filter_map(|(path, bytes)| Some((path.strip_prefix(prefix.as_str())?, *bytes)))
        .filter(|(rel, _)| !KEPT.contains(rel))
        .collect();
    Some(Block { id, files })
}

/// The Block a request names, or the usage error when it names one with anything else.
/// `dir` is the folder `-C` named, so the hint repeats it.
pub(super) fn named(dir: Option<&Path>, request: &Request) -> Option<Result<Block, String>> {
    let (raw, block) = request.ids.iter().find_map(|raw| {
        let id = raw.split('@').next().unwrap_or(raw);
        Some((raw, find(id)?))
    })?;
    let alone = request.ids.len() == 1 && !request.overwrite && !request.diff && !request.relock;
    Some(if raw != block.id {
        let command =
            project::add_writing(&project::add_command(dir), &[block.id.to_string()], false)
                .expect("a block id");
        Err(format!(
            "`{}` is a Block, and orbe installs the one version of it built in. run `{command}`",
            block.id
        ))
    } else if !alone {
        Err(format!(
            "`{}` is a Block. orbe installs a Block on its own, with no other id and no --overwrite, --diff or --relock",
            block.id
        ))
    } else {
        Ok(block)
    })
}

pub struct BlockReply {
    pub id: String,
    /// Each file the Block brings, with whether this run wrote it. One not written was already
    /// there with the Block's bytes.
    pub files: Vec<(String, bool)>,
    pub parts: AddReply,
    /// How to check and test the installed project, from where `orbe add` ran.
    pub next: String,
}

pub(super) fn install(
    dir: Option<&Path>,
    block: Block,
    fault: &mut Fault,
) -> Result<Reply, RunError> {
    if let Some(dir) = dir
        && !dir.is_dir()
    {
        return Ok(Reply::Refuse(Refusal {
            code: AddCode::NotAProject,
            message: format!("`{}` is not a folder", dir.display()),
            hint: String::new(),
        }));
    }
    let root = dir.unwrap_or(Path::new("."));
    let add = project::add_command(dir);
    let again = Again::new(
        dir,
        project::add_writing(&add, &[block.id.to_string()], false).expect("a block id"),
    );
    let opened = Folder::open_chosen(root).map_err(|error| io_at(root, error))?;
    let mut present = BTreeSet::new();
    // The scene goes first, so a project that has one hears `scene_exists`, whatever else differs.
    let scene = |file: &&(&str, &[u8])| file.0 == "scene.jsonc";
    let ordered = block
        .files
        .iter()
        .filter(scene)
        .chain(block.files.iter().filter(|file| !scene(file)));
    for (rel, bytes) in ordered {
        let found = seen(&opened, rel, bytes).map_err(|error| io_at(&root.join(rel), error))?;
        if let Seen::Link(at) = &found {
            return Ok(Reply::Refuse(is_a_link(at, &again)));
        }
        if let Some(at) = temp_link(&opened, rel).map_err(|error| io_at(&root.join(rel), error))? {
            return Ok(Reply::Refuse(is_a_link(&at, &again)));
        }
        match found {
            Seen::Match => {
                present.insert(*rel);
            }
            Seen::Blocked => return Ok(Reply::Refuse(in_the_way(&block, rel, &again))),
            Seen::Missing | Seen::Link(_) => {}
        }
    }
    let parts = Request {
        ids: block.part_ids()?,
        overwrite: false,
        diff: false,
        relock: false,
    };
    let parts = match super::run_in(dir, &parts, fault)? {
        Reply::Add(reply) => reply,
        refused => return Ok(refused),
    };
    let mut files = Vec::new();
    for (rel, bytes) in &block.files {
        let wrote = !present.contains(rel);
        if wrote {
            write(root, rel, bytes).map_err(|error| RunError {
                message: format!(
                    "{error}. the parts are in, and the Block's other files may not be. run `{again}` again to finish"
                ),
            })?;
        }
        files.push((rel.to_string(), wrote));
    }
    let next = format!(
        "run `orbe check {}` and `orbe test{}`",
        again.path("scene.jsonc"),
        dir.map(|dir| format!(" {}", project::shell_path(dir.as_os_str())))
            .unwrap_or_default()
    );
    Ok(Reply::Block(BlockReply {
        id: block.id.to_string(),
        files,
        parts,
        next,
    }))
}

impl Block {
    /// The catalog ids the Block's scene names, read as the Block reads them.
    fn part_ids(&self) -> Result<Vec<String>, RunError> {
        let files = Files::memory(
            self.files
                .iter()
                .map(|(rel, bytes)| (*rel, bytes.to_vec()))
                .chain([(BLOCK, Vec::new())]),
        );
        crate::catalog_ids(&files, Path::new("scene.jsonc")).map_err(|found| RunError {
            message: format!(
                "the Block `{}` built into this orbe does not name its parts: {}",
                self.id,
                found
                    .iter()
                    .map(|d| d.message.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        })
    }
}

/// A file the Block brings is already in the project with other bytes. A scene is never merged,
/// since two scenes' names and wires would collide, so the way out is an empty folder.
fn in_the_way(block: &Block, rel: &str, again: &Again) -> Refusal {
    let fresh = format!(
        "mkdir {id} && {cmd}",
        id = project::shell_word(block.id),
        cmd = project::add_writing(
            &project::add_command(Some(Path::new(block.id))),
            &[block.id.to_string()],
            false,
        )
        .expect("a block id"),
    );
    if rel == "scene.jsonc" {
        return Refusal {
            code: AddCode::SceneExists,
            message: format!(
                "{} already holds a scene, and the Block `{}` brings its own. orbe does not merge scenes. nothing was written",
                again.path("scene.jsonc"),
                block.id
            ),
            hint: format!("install it into an empty folder: `{fresh}`"),
        };
    }
    Refusal {
        code: AddCode::FileExists,
        message: format!(
            "{} is already there and is not the Block `{}`'s. nothing was written",
            again.path(rel),
            block.id
        ),
        hint: format!(
            "move {} out of the way and run `{again}` again, or install into an empty folder: `{fresh}`",
            again.path(rel)
        ),
    }
}

/// What is at one Block path, read without following a link.
enum Seen {
    Missing,
    Match,
    Blocked,
    Link(String),
}

fn is_a_link(at: &str, again: &Again) -> Refusal {
    let path = again.path(at);
    Refusal {
        code: AddCode::Symlink,
        message: format!(
            "{path} is a link, and orbe add does not write through a link. nothing was written"
        ),
        hint: format!("remove the link {path}, then run `{again}` again"),
    }
}

fn component_names(rel: &str) -> io::Result<Vec<&str>> {
    if rel.is_empty()
        || rel
            .split('/')
            .any(|name| name.is_empty() || name == "." || name == "..")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a Block path is not a file under the project",
        ));
    }
    Ok(rel.split('/').collect())
}

fn missing(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound
}

/// The first link on `rel`, or what the file itself is when every component is real.
fn seen(root: &Folder, rel: &str, want: &[u8]) -> io::Result<Seen> {
    let names = component_names(rel)?;
    let mut folder = root.dup()?;
    let mut at = String::new();
    for (index, name) in names.iter().enumerate() {
        if !at.is_empty() {
            at.push('/');
        }
        at.push_str(name);
        let last = index + 1 == names.len();
        match folder.find(OsStr::new(name)) {
            Err(error) if missing(&error) => return Ok(Seen::Missing),
            Ok(Found::Link(_)) => return Ok(Seen::Link(at)),
            Ok(Found::Folder) if !last => folder = folder.sub(OsStr::new(name))?,
            Ok(Found::File(found)) if last => {
                return Ok(if found == want {
                    Seen::Match
                } else {
                    Seen::Blocked
                });
            }
            Ok(_) => return Ok(Seen::Blocked),
            Err(error) => return Err(error),
        }
    }
    Ok(Seen::Missing)
}

/// The temp name beside `rel`, when that name is a link. A missing parent has no temp file.
fn temp_link(root: &Folder, rel: &str) -> io::Result<Option<String>> {
    let names = component_names(rel)?;
    let file = names[names.len() - 1];
    let mut folder = root.dup()?;
    let mut at = String::new();
    for name in &names[..names.len() - 1] {
        if !at.is_empty() {
            at.push('/');
        }
        at.push_str(name);
        match folder.find(OsStr::new(name)) {
            Err(error) if missing(&error) => return Ok(None),
            Ok(Found::Link(_)) => return Ok(Some(at)),
            Ok(Found::Folder) => folder = folder.sub(OsStr::new(name))?,
            Ok(_) => return Ok(None),
            Err(error) => return Err(error),
        }
    }
    let tmp = format!(".{file}.orbe-add");
    let tmp_at = if at.is_empty() {
        tmp.clone()
    } else {
        format!("{at}/{tmp}")
    };
    match folder.find(OsStr::new(&tmp)) {
        Ok(Found::Link(_)) => Ok(Some(tmp_at)),
        Ok(_) => Ok(None),
        Err(error) if missing(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Writes `rel` under `root` without following a link. The temp file is created exclusively.
fn write(root: &Path, rel: &str, bytes: &[u8]) -> Result<(), RunError> {
    let mut folder = Folder::open_chosen(root).map_err(|error| io_at(root, error))?;
    let names = component_names(rel).map_err(|error| io_at(&root.join(rel), error))?;
    let file = names[names.len() - 1];
    let mut at = String::new();
    for name in &names[..names.len() - 1] {
        if !at.is_empty() {
            at.push('/');
        }
        at.push_str(name);
        let path = root.join(&at);
        match folder.find(OsStr::new(name)) {
            Ok(Found::Folder) => {}
            Ok(Found::Link(_)) => {
                return Err(RunError {
                    message: format!("{at} is a link, and orbe add does not write through a link"),
                });
            }
            Err(error) if missing(&error) => {
                folder
                    .mkdir(OsStr::new(name))
                    .map_err(|error| io_at(&path, error))?;
            }
            Ok(_) => {
                return Err(io_at(
                    &path,
                    io::Error::new(io::ErrorKind::AlreadyExists, "not a folder"),
                ));
            }
            Err(error) => return Err(io_at(&path, error)),
        }
        folder = folder
            .sub(OsStr::new(name))
            .map_err(|error| io_at(&path, error))?;
    }
    let tmp = format!(".{file}.orbe-add");
    let tmp_rel = if at.is_empty() {
        tmp.clone()
    } else {
        format!("{at}/{tmp}")
    };
    let tmp_path = root.join(&tmp_rel);
    match folder.find(OsStr::new(&tmp)) {
        Ok(Found::File(_)) => {
            folder
                .remove_file(OsStr::new(&tmp))
                .map_err(|error| io_at(&tmp_path, error))?;
        }
        Ok(Found::Link(_)) => {
            return Err(RunError {
                message: format!("{tmp_rel} is a link, and orbe add does not write through a link"),
            });
        }
        Err(error) if missing(&error) => {}
        Ok(_) => {
            return Err(io_at(
                &tmp_path,
                io::Error::new(io::ErrorKind::AlreadyExists, "not a file"),
            ));
        }
        Err(error) => return Err(io_at(&tmp_path, error)),
    }
    let mut created = folder
        .create_new(OsStr::new(&tmp))
        .map_err(|error| io_at(&tmp_path, error))?;
    created
        .write_all(bytes)
        .map_err(|error| io_at(&tmp_path, error))?;
    drop(created);
    let dest = root.join(rel);
    match folder.find(OsStr::new(file)) {
        Ok(Found::Link(_)) => {
            let _ = folder.remove_file(OsStr::new(&tmp));
            return Err(RunError {
                message: format!("{rel} is a link, and orbe add does not write through a link"),
            });
        }
        Err(error) if missing(&error) => {}
        Ok(Found::File(_)) => {}
        Ok(_) => {
            let _ = folder.remove_file(OsStr::new(&tmp));
            return Err(io_at(
                &dest,
                io::Error::new(io::ErrorKind::AlreadyExists, "not a file"),
            ));
        }
        Err(error) => return Err(io_at(&dest, error)),
    }
    folder
        .rename_within(OsStr::new(&tmp), OsStr::new(file))
        .map_err(|error| io_at(&dest, error))?;
    Ok(())
}

pub(super) fn text(reply: &BlockReply) -> String {
    let mut lines: Vec<String> = reply
        .files
        .iter()
        .map(|(rel, wrote)| {
            let verb = if *wrote { "wrote" } else { "kept" };
            format!("{verb} {rel}")
        })
        .collect();
    lines.push(super::add_text(&reply.parts));
    lines.push(format!(
        "installed the Block `{}`. {}",
        reply.id, reply.next
    ));
    lines.join("\n")
}

pub(super) fn json(reply: &BlockReply) -> Value {
    let files = |wrote: bool| -> Vec<&str> {
        reply
            .files
            .iter()
            .filter(|(_, w)| *w == wrote)
            .map(|(rel, _)| rel.as_str())
            .collect()
    };
    json!({
        "ok": true,
        "data": {
            "block": reply.id,
            "files": { "written": files(true), "kept": files(false) },
            "parts": super::add_data(&reply.parts),
            "next": reply.next,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `orbe add` reads one word as a part or a Block, so no word may be both.
    #[test]
    fn no_block_is_named_like_a_catalog_part() {
        let parts: BTreeSet<String> = crate::catalog::Source::Builtin.ids().collect();
        let both: Vec<&str> = ids().filter(|id| parts.contains(*id)).collect();
        assert!(
            both.is_empty(),
            "these ids name a catalog part and a Block: {both:?}"
        );
        assert!(ids().count() >= 3, "the Blocks are embedded");
    }

    #[test]
    fn a_block_installs_no_file_that_would_make_the_project_a_block() {
        for id in ids() {
            let block = find(id).unwrap();
            assert!(block.files.iter().any(|(rel, _)| *rel == "scene.jsonc"));
            for (rel, _) in &block.files {
                assert!(!KEPT.contains(rel), "{id} installs {rel}");
                assert!(!rel.starts_with("parts"), "{id} carries {rel}");
            }
        }
    }
}
