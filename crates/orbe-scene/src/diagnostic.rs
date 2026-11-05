//! Findings, each with a code from investigation 4's errors table and a place in a file.

use std::fmt;

use crate::source::{Sources, Span};

/// What a finding is about. Every finding is an error: `orbe check` is type checking only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Code {
    /// The file is not the dialect, or a value has the wrong JSON shape or an unknown key.
    Syntax,
    Format,
    Name,
    Duplicate,
    Type,
    Role,
    UnknownRef,
    UnknownPin,
    Param,
    Layout,
    Wires,
    NotSimulated,
    Ground,
    Cycle,
    NameTooLong,
    PartMissing,
    FirmwareMissing,
    PartModified,
    /// A project file that exists could not be read.
    Io,
    /// The engine refused a value the reader wrote, and no param set it. The engine's own code.
    Engine(&'static str),
}

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Code::Syntax => "scene.syntax",
            Code::Format => "scene.format",
            Code::Name => "scene.name",
            Code::Duplicate => "scene.duplicate",
            Code::Type => "scene.type",
            Code::Role => "scene.role",
            Code::UnknownRef => "scene.unknown_ref",
            Code::UnknownPin => "scene.unknown_pin",
            Code::Param => "scene.param",
            Code::Layout => "scene.layout",
            Code::Wires => "scene.wires",
            Code::NotSimulated => "not_simulated",
            Code::Ground => "scene.ground",
            Code::Cycle => "scene.cycle",
            Code::NameTooLong => "scene.name_too_long",
            Code::PartMissing => "part_missing",
            Code::FirmwareMissing => "firmware_missing",
            Code::PartModified => "part_modified",
            Code::Io => "io_error",
            Code::Engine(code) => code,
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One finding, placed at a file, line and column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: Code,
    /// The path from the project root, the scene's folder.
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
    pub hint: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: error[{}]: {}",
            self.file, self.line, self.column, self.code, self.message
        )?;
        write!(f, "\n  hint: {}", self.hint)?;
        Ok(())
    }
}

pub(crate) struct Finding {
    code: Code,
    span: Span,
    message: String,
    hint: String,
}

#[derive(Default)]
pub(crate) struct Findings(Vec<Finding>);

impl Findings {
    pub(crate) fn push(
        &mut self,
        code: Code,
        span: Span,
        message: impl Into<String>,
        hint: String,
    ) {
        self.0.push(Finding {
            code,
            span,
            message: message.into(),
            hint,
        });
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Placed, sorted by file, line and column, with repeats dropped. A part used several times
    /// reports a finding inside it once.
    pub(crate) fn into_diagnostics(self, sources: &Sources) -> Vec<Diagnostic> {
        let mut out: Vec<Diagnostic> = self
            .0
            .into_iter()
            .map(|f| {
                let (line, column) = sources.line_col(f.span);
                Diagnostic {
                    code: f.code,
                    file: sources.file(f.span.file).display.clone(),
                    line,
                    column,
                    message: f.message,
                    hint: f.hint,
                }
            })
            .collect();
        out.sort_by(|a, b| {
            (&a.file, a.line, a.column, a.code, &a.message)
                .cmp(&(&b.file, b.line, b.column, b.code, &b.message))
        });
        out.dedup();
        out
    }
}
