//! `orbe add <block>` installs a Block: it writes the Block's scene, firmware and tests into the
//! project, then copies the parts the scene names as `orbe add <part>` does. A part id and a
//! Block id never match, so one word names one of them.
//!
//! Every check that can refuse runs before anything is written. A file already there with the
//! Block's bytes is kept, so running the same install again finishes one that stopped part way.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;

use serde_json::{Value, json};

use super::{AddCode, AddReply, Again, Fault, Refusal, Reply, Request, RunError, io_at};
use crate::files::Files;
use crate::project::{self, BLOCK};

include!(concat!(env!("OUT_DIR"), "/blocks.rs"));

/// The files a Block keeps to itself: the one that makes its folder a Block, and its page.
/// An installed project reads its own copies of the parts, so it must not be a Block.
const KEPT: [&str; 2] = [BLOCK, "README.md"];

/// A Block this binary was built with, from the repo's `blocks/`.
pub(super) struct Block {
    pub(super) id: &'static str,
    /// The files the install writes, by their path from the Block's folder.
    files: Vec<(&'static str, &'static [u8])>,
}

pub(crate) fn ids() -> impl Iterator<Item = &'static str> {
    FILES
        .iter()
        .filter_map(|(path, _)| path.strip_suffix(&format!("/{BLOCK}")))
}

fn find(id: &str) -> Option<Block> {
    let id = ids().find(|block| *block == id)?;
    let prefix = format!("{id}/");
    let files = FILES
        .iter()
        .filter_map(|(path, bytes)| Some((path.strip_prefix(prefix.as_str())?, *bytes)))
        .filter(|(rel, _)| !KEPT.contains(rel))
        .collect();
    Some(Block { id, files })
}

/// The Block a request names, or the usage error when it names one with anything else.
pub(super) fn named(request: &Request) -> Option<Result<Block, String>> {
    let (raw, block) = request.ids.iter().find_map(|raw| {
        let id = raw.split('@').next().unwrap_or(raw);
        Some((raw, find(id)?))
    })?;
    let alone = request.ids.len() == 1 && !request.overwrite && !request.diff && !request.relock;
    Some(if raw != block.id {
        Err(format!(
            "`{}` is a Block, and a Block has no versions. run `orbe add {}`",
            block.id, block.id
        ))
    } else if !alone {
        Err(format!(
            "`{}` is a Block. `orbe add` installs a Block on its own, with no other id and no --overwrite, --diff or --relock",
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
    let again = Again::new(dir, format!("{add} {}", block.id));
    let mut present = BTreeSet::new();
    for (rel, bytes) in &block.files {
        match fs::read(root.join(rel)) {
            Ok(found) if found == *bytes => {
                present.insert(*rel);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            _ => return Ok(Reply::Refuse(in_the_way(&block, rel, &again))),
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
            write(&root.join(rel), bytes).map_err(|error| RunError {
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
        "mkdir {id} && orbe add -C {id} {id}",
        id = project::shell_word(block.id)
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

/// Writes beside `path`, then renames over it, so a file is either whole or not there.
fn write(path: &Path, bytes: &[u8]) -> Result<(), RunError> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|error| io_at(parent, error))?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = parent.join(format!(".{name}.orbe-add"));
    fs::write(&tmp, bytes)
        .and_then(|()| fs::rename(&tmp, path))
        .map_err(|error| io_at(path, error))
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
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `orbe add` reads one word as a part or a Block, so no word may be both.
    #[test]
    fn no_block_is_named_like_a_catalog_part() {
        let parts: BTreeSet<String> = crate::catalog::ids().collect();
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
