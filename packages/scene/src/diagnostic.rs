//! Findings, each with a code from investigation 4's errors table and a place in a file.

use std::fmt;

use crate::source::{Sources, Span};

/// What a finding is about. Every code is an error except a [`Lint`], which only `orbe lint`
/// reports and which never blocks.
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
    /// A `world` quantity the table lacks, a value outside its range, or a step out of order.
    World,
    Layout,
    Wires,
    NotSimulated,
    Ground,
    Cycle,
    NameTooLong,
    PartMissing,
    FirmwareMissing,
    PartModified,
    /// The copy matches its lock line and is another version than this binary carries.
    /// A run reads the version in the binary, so this stops check, open and test.
    PartOutdated,
    /// A project file that exists could not be read.
    Io,
    /// The engine refused a value the reader wrote, and no param set it. The engine's own code.
    Engine(&'static str),
    Lint(Lint),
}

/// A lint rule: the scene checks, and is likely wrong or unclear.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Lint {
    Unplaced,
    Unconnected,
    GroundUnnamed,
    Rotation,
    NoModel,
    /// The engine's own static check.
    Floating,
    LedNoResistor,
    /// Two pins of one part in one breadboard strip.
    Shorted,
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
            Code::World => "scene.world",
            Code::Layout => "scene.layout",
            Code::Wires => "scene.wires",
            Code::NotSimulated => "not_simulated",
            Code::Ground => "scene.ground",
            Code::Cycle => "scene.cycle",
            Code::NameTooLong => "scene.name_too_long",
            Code::PartMissing => "part_missing",
            Code::FirmwareMissing => "firmware_missing",
            Code::PartModified => "part_modified",
            Code::PartOutdated => "part_outdated",
            Code::Io => "io_error",
            Code::Engine(code) => code,
            Code::Lint(Lint::Unplaced) => "scene.unplaced",
            Code::Lint(Lint::Unconnected) => "scene.unconnected",
            Code::Lint(Lint::GroundUnnamed) => "scene.ground_unnamed",
            Code::Lint(Lint::Rotation) => "scene.rotation",
            Code::Lint(Lint::NoModel) => "scene.no_model",
            Code::Lint(Lint::Floating) => "net.floating",
            Code::Lint(Lint::LedNoResistor) => "scene.led_no_resistor",
            Code::Lint(Lint::Shorted) => "scene.shorted",
        }
    }

    pub fn severity(self) -> &'static str {
        match self {
            Code::Lint(_) => "warning",
            _ => "error",
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
            "{}:{}:{}: {}[{}]: {}",
            self.file,
            self.line,
            self.column,
            self.code.severity(),
            self.code,
            self.message
        )?;
        write!(f, "\n  hint: {}", self.hint)?;
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct Finding {
    code: Code,
    span: Span,
    message: String,
    hint: String,
}

#[derive(Default)]
pub(crate) struct Findings(Vec<Finding>);

/// Proof that a finding was pushed. Only [`Findings::since`] makes one, so whatever holds it
/// is explained by a finding the reader reports.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Reported(());

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

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    /// Proof of a finding pushed after `mark`, a [`Findings::len`] taken earlier.
    pub(crate) fn since(&self, mark: usize) -> Option<Reported> {
        (self.0.len() > mark).then_some(Reported(()))
    }

    /// A copy of every finding pushed after `mark`, in that order.
    pub(crate) fn copied_since(&self, mark: usize) -> Vec<Finding> {
        self.0[mark..].to_vec()
    }

    /// Pushes `findings` in order.
    pub(crate) fn extend_from(&mut self, findings: &[Finding]) {
        self.0.extend_from_slice(findings);
    }

    /// Gives every finding pushed after `mark` this hint.
    pub(crate) fn rehint_since(&mut self, mark: usize, hint: &str) {
        for finding in &mut self.0[mark..] {
            finding.hint = hint.to_string();
        }
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
