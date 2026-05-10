//! Parser for Dockerfiles.
//!
//! Produces a lossless rowan green tree alongside a list of [`Diagnostic`]s.
//! See [`parse`] for the entry point.
//!
//! # Pipeline
//!
//! 1. [`directive`] runs a pre-pass over the source to identify any leading
//!    `# directive=value` lines and (most importantly) honor a `# escape=`
//!    directive that changes the line-continuation character.
//! 2. The [`dockerfile_lexer`] is constructed with that escape character and
//!    its token stream is collected.
//! 3. [`Parser`] walks the tokens, delegating to per-instruction routines and
//!    pushing tokens/nodes into a [`rowan::GreenNodeBuilder`].
//!
//! The grammar is intentionally permissive: unknown instructions are still
//! captured as [`SyntaxKind::INSTRUCTION`] nodes with a diagnostic, and badly
//! formed lines are recovered by skipping to the next newline. This matches
//! the goals of a hadolint-style linter, where producing a useful tree even
//! for broken input is more important than rejecting it outright.

mod directive;
mod parser;

pub use directive::{ParserDirective, scan_directives};

use dockerfile_diagnostics::Diagnostic;
use dockerfile_syntax::SyntaxNode;
use rowan::GreenNode;

/// Result of parsing a Dockerfile source.
#[derive(Debug, Clone)]
pub struct Parse {
    green: GreenNode,
    diagnostics: Vec<Diagnostic>,
}

impl Parse {
    /// The lossless syntax tree.
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    /// Diagnostics produced during parsing.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Underlying green node, useful for sharing trees cheaply across
    /// analysis passes.
    pub fn green(&self) -> &GreenNode {
        &self.green
    }

    /// Render the tree text — should equal the input source byte-for-byte.
    pub fn debug_tree(&self) -> String {
        let syntax = self.syntax();
        let mut out = format!("{syntax:#?}");
        // Remove the trailing newline `format!("{:#?}")` adds for readability.
        if out.ends_with('\n') {
            out.pop();
        }
        out
    }
}

/// Parse a complete Dockerfile source string.
pub fn parse(src: &str) -> Parse {
    let directives = scan_directives(src);
    let escape = directives.escape_char();
    let parser = parser::Parser::new(src, escape, directives.directives);
    let (green, diagnostics) = parser.parse();
    Parse { green, diagnostics }
}

/// Convenience for tests/debugging: parse and check that the tree fully
/// covers the input.
#[doc(hidden)]
pub fn assert_round_trips(src: &str) {
    let parse = parse(src);
    let text = parse.syntax().text().to_string();
    assert_eq!(text, src, "tree text must equal source");
}

#[cfg(test)]
mod tests;

// Re-exports used in tests.
#[doc(hidden)]
pub use dockerfile_syntax as syntax;
