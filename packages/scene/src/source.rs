//! The project files the reader read, and spans into them.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use crate::syntax::Node;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FileId(usize);

/// A byte range in one file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Span {
    pub(crate) file: FileId,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

impl Span {
    pub(crate) fn start_of(file: FileId) -> Span {
        Span {
            file,
            start: 0,
            end: 0,
        }
    }
}

pub(crate) struct SourceFile {
    pub(crate) path: PathBuf,
    /// The path from the project root, with `/` between parts.
    pub(crate) display: String,
    pub(crate) text: String,
}

pub(crate) struct Sources {
    root: PathBuf,
    files: Vec<SourceFile>,
    /// `None` until [`crate::syntax::parse`] has read the file. `Some(None)` did not parse.
    trees: RefCell<Vec<Option<Option<Node>>>>,
}

impl Sources {
    pub(crate) fn new(root: &Path) -> Sources {
        Sources {
            root: root.to_path_buf(),
            files: Vec::new(),
            trees: RefCell::new(Vec::new()),
        }
    }

    /// The tree from the first parse of `file`, once one has run.
    pub(crate) fn parsed(&self, file: FileId) -> Option<Option<Node>> {
        self.trees
            .borrow()
            .get(file.0)
            .and_then(|slot| slot.clone())
    }

    pub(crate) fn remember(&self, file: FileId, node: &Option<Node>) {
        let mut trees = self.trees.borrow_mut();
        if trees.len() <= file.0 {
            trees.resize(file.0 + 1, None);
        }
        trees[file.0] = Some(node.clone());
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn add(&mut self, path: &Path, text: String) -> FileId {
        let display = display(&self.root, path);
        self.files.push(SourceFile {
            path: path.to_path_buf(),
            display,
            text,
        });
        FileId(self.files.len() - 1)
    }

    pub(crate) fn find(&self, path: &Path) -> Option<FileId> {
        self.files.iter().position(|f| f.path == path).map(FileId)
    }

    pub(crate) fn file(&self, id: FileId) -> &SourceFile {
        &self.files[id.0]
    }

    pub(crate) fn files(&self) -> &[SourceFile] {
        &self.files
    }

    /// The 1-based line and column of a span's start, the column counted in characters.
    pub(crate) fn line_col(&self, span: Span) -> (usize, usize) {
        let text = &self.file(span.file).text;
        let before = &text[..span.start.min(text.len())];
        let line = before.matches('\n').count() + 1;
        let line_start = before.rfind('\n').map_or(0, |i| i + 1);
        (line, before[line_start..].chars().count() + 1)
    }

    /// `file:line:col`, as the reader prints a location.
    pub(crate) fn location(&self, span: Span) -> String {
        let (line, col) = self.line_col(span);
        format!("{}:{line}:{col}", self.file(span.file).display)
    }
}

pub(crate) fn display(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
