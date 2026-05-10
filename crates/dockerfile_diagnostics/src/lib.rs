//! Diagnostics for the Dockerfile parser pipeline.
//!
//! All errors and warnings produced by the lexer, parser, and analyzers
//! are reported through [`Diagnostic`].

use std::ops::Range;

use text_size::{TextRange, TextSize};

/// Severity of a [`Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    /// A hard error: the input could not be fully understood at this point.
    Error,
    /// A warning: the input is understandable but suspicious or non-idiomatic.
    Warning,
}

/// A diagnostic message attached to a span of source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub range: TextRange,
    /// Stable rule identifier (e.g. `"DF001"`) used for lints; `None` for
    /// generic parse errors.
    pub code: Option<&'static str>,
}

impl Diagnostic {
    pub fn error(message: impl Into<String>, range: TextRange) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            range,
            code: None,
        }
    }

    pub fn warning(message: impl Into<String>, range: TextRange) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
            range,
            code: None,
        }
    }

    #[must_use]
    pub fn with_code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }

    pub fn span(&self) -> Range<usize> {
        self.range.start().into()..self.range.end().into()
    }
}

/// Convenience for callers that produce diagnostics with `usize` offsets.
pub fn range(start: usize, end: usize) -> TextRange {
    TextRange::new(
        TextSize::try_from(start).expect("offset overflow"),
        TextSize::try_from(end).expect("offset overflow"),
    )
}
