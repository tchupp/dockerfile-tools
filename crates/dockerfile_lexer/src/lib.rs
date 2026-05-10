//! Hand-rolled lexer for Dockerfiles.
//!
//! # Design
//!
//! The lexer produces a coarse stream of tokens — trivia (whitespace, newline,
//! comment, line continuation), quoted strings, and `WORD` (any run of
//! non-whitespace, non-quote bytes). Internal punctuation such as `=`, `:`,
//! `[`, `]` is *not* split out at the lexer level: those characters are
//! contextual (an `=` inside a `RUN` body has nothing to do with an `=` in
//! `ENV key=value`), so the parser re-tokenizes within a `WORD` when it
//! actually needs structure.
//!
//! Dockerfile-specific rules implemented here:
//!
//! - `#` only starts a comment when it is the first non-whitespace byte on a
//!   *logical* line. A `#` mid-instruction is just part of a `WORD`.
//! - `\<newline>` (or, if a `# escape=` directive overrode it, `` `<newline> ``)
//!   is emitted as a [`LINE_CONTINUATION`] trivia token. The lexer does *not*
//!   collapse the bytes — they remain in the source slice — but the parser
//!   treats the continuation as glue between two physical lines of the same
//!   logical instruction.
//! - The escape character is configurable through [`Lexer::with_escape`] so
//!   the parser can re-lex a file once it has scanned the directive block.
//!
//! Two specialized lex modes are provided in addition to the general
//! [`Lexer::next_token`] loop:
//!
//! - [`Lexer::lex_shell_body`] consumes from the cursor to the end of the
//!   current logical line and emits a single [`SHELL_BODY`] token, used for
//!   the bodies of shell-form `RUN`/`CMD`/`ENTRYPOINT`.
//! - [`Lexer::lex_heredoc_body`] consumes a heredoc body up through the
//!   matching delimiter line.
//!
//! [`LINE_CONTINUATION`]: dockerfile_syntax::SyntaxKind::LINE_CONTINUATION
//! [`SHELL_BODY`]: dockerfile_syntax::SyntaxKind::SHELL_BODY

use dockerfile_syntax::SyntaxKind;
use text_size::{TextRange, TextSize};

/// The default Dockerfile escape character.
pub const DEFAULT_ESCAPE: char = '\\';

/// A single lexed token: a [`SyntaxKind`] plus the byte range it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: SyntaxKind,
    pub range: TextRange,
}

impl Token {
    /// Get the source text covered by this token.
    pub fn text<'a>(&self, src: &'a str) -> &'a str {
        &src[self.range]
    }
}

/// Stateful cursor over a Dockerfile source string.
#[derive(Debug, Clone)]
pub struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    escape: u8,
    /// `true` when the cursor is at the start of a *logical* line — i.e. only
    /// whitespace tokens (or no tokens at all) have been emitted since the
    /// last [`NEWLINE`]. A `#` is only treated as a comment in this state.
    ///
    /// [`NEWLINE`]: SyntaxKind::NEWLINE
    at_line_start: bool,
}

impl<'a> Lexer<'a> {
    /// Construct a lexer using the default escape character (`\`).
    pub fn new(src: &'a str) -> Self {
        Self::with_escape(src, DEFAULT_ESCAPE)
    }

    /// Construct a lexer with a custom escape character. Dockerfile only
    /// permits ASCII escape characters (`\` or `` ` ``); a multi-byte char
    /// will fall back to `\`.
    pub fn with_escape(src: &'a str, escape: char) -> Self {
        let escape = u8::try_from(u32::from(escape)).unwrap_or(b'\\');
        Self {
            src,
            pos: 0,
            escape,
            at_line_start: true,
        }
    }

    /// Total length of the source.
    pub fn source_len(&self) -> usize {
        self.src.len()
    }

    /// Current cursor position.
    pub fn position(&self) -> TextSize {
        text_size_of(self.pos)
    }

    /// `true` once the cursor has consumed all input.
    pub fn is_eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    /// Returns the next token, or `None` at end of input.
    pub fn next_token(&mut self) -> Option<Token> {
        if self.is_eof() {
            return None;
        }
        let start = self.pos;
        let kind = self.scan_one();
        Some(Token {
            kind,
            range: TextRange::new(text_size_of(start), text_size_of(self.pos)),
        })
    }

    /// Lex from the current cursor through the end of the current *logical*
    /// line as a single [`SHELL_BODY`] token. The trailing newline (if any) is
    /// **not** consumed; the caller can take it with the next [`next_token`]
    /// call.
    ///
    /// Inside a shell body, `escape<newline>` sequences are part of the token
    /// text (rather than separate trivia) — the shell itself uses the same
    /// continuation convention, and consumers like an `shfmt` integration
    /// expect the raw shell text.
    ///
    /// Returns `None` if the cursor is already at end-of-line / EOF.
    ///
    /// [`SHELL_BODY`]: SyntaxKind::SHELL_BODY
    /// [`next_token`]: Self::next_token
    pub fn lex_shell_body(&mut self) -> Option<Token> {
        if self.is_eof() {
            return None;
        }
        let bytes = self.src.as_bytes();
        if matches!(bytes[self.pos], b'\n' | b'\r') {
            return None;
        }
        let start = self.pos;
        while self.pos < bytes.len() {
            let b = bytes[self.pos];
            if b == b'\n' || b == b'\r' {
                break;
            }
            if b == self.escape && self.is_newline_at(self.pos + 1) {
                // Consume escape + the entire (possibly CRLF) newline as part
                // of the shell body; we are continuing onto the next physical
                // line of the same logical line.
                self.pos += 1; // escape
                self.consume_newline();
                continue;
            }
            self.pos += 1;
        }
        self.at_line_start = false;
        Some(Token {
            kind: SyntaxKind::SHELL_BODY,
            range: TextRange::new(text_size_of(start), text_size_of(self.pos)),
        })
    }

    /// Lex a heredoc body up through (and including) the line that matches
    /// `delimiter`. If the delimiter is never found, consumes to EOF and
    /// returns the entire range — the parser is expected to surface a
    /// diagnostic in that case.
    ///
    /// `strip_tabs` corresponds to the `BuildKit` `<<-` form: leading tabs on
    /// each body line are tolerated when matching the delimiter, but they are
    /// **not** stripped from the token text (round-trip preservation is the
    /// lexer's job; semantic interpretation belongs to the AST layer).
    pub fn lex_heredoc_body(&mut self, delimiter: &str, strip_tabs: bool) -> Token {
        let bytes = self.src.as_bytes();
        let start = self.pos;
        // The body begins on the line *after* the heredoc-start token. The
        // caller is responsible for ensuring the cursor sits at the first
        // character of that line.
        loop {
            if self.pos >= bytes.len() {
                break;
            }
            let line_start = self.pos;
            // Optionally allow leading tabs when matching the delimiter.
            let mut match_cursor = line_start;
            if strip_tabs {
                while match_cursor < bytes.len() && bytes[match_cursor] == b'\t' {
                    match_cursor += 1;
                }
            }
            let line_end = next_newline(bytes, line_start);
            let line_content = &self.src[match_cursor..line_end];
            if line_content == delimiter {
                // Consume the delimiter line including its trailing newline so
                // the heredoc body token covers everything up to (but not
                // past) the next logical content.
                self.pos = line_end;
                self.consume_newline();
                break;
            }
            // Not the closing delimiter: advance past this body line.
            self.pos = line_end;
            self.consume_newline();
        }
        self.at_line_start = true;
        Token {
            kind: SyntaxKind::HEREDOC_BODY,
            range: TextRange::new(text_size_of(start), text_size_of(self.pos)),
        }
    }

    // ------------------------------------------------------------------
    // Internal scanning helpers
    // ------------------------------------------------------------------

    fn scan_one(&mut self) -> SyntaxKind {
        let bytes = self.src.as_bytes();
        let b = bytes[self.pos];

        // Line continuation: `<escape><newline>`.
        if b == self.escape && self.is_newline_at(self.pos + 1) {
            self.pos += 1; // escape
            self.consume_newline();
            self.at_line_start = false;
            return SyntaxKind::LINE_CONTINUATION;
        }

        match b {
            b' ' | b'\t' => {
                while self.pos < bytes.len() && matches!(bytes[self.pos], b' ' | b'\t') {
                    self.pos += 1;
                }
                SyntaxKind::WHITESPACE
            }
            b'\n' | b'\r' => {
                self.consume_newline();
                self.at_line_start = true;
                SyntaxKind::NEWLINE
            }
            b'#' if self.at_line_start => {
                while self.pos < bytes.len() && !matches!(bytes[self.pos], b'\n' | b'\r') {
                    self.pos += 1;
                }
                self.at_line_start = false;
                SyntaxKind::COMMENT
            }
            b'"' | b'\'' => self.scan_string(b),
            _ => self.scan_word(),
        }
    }

    fn scan_string(&mut self, quote: u8) -> SyntaxKind {
        let bytes = self.src.as_bytes();
        self.pos += 1; // opening quote
        let mut terminated = false;
        while self.pos < bytes.len() {
            let b = bytes[self.pos];
            if b == quote {
                self.pos += 1;
                terminated = true;
                break;
            }
            if b == self.escape && self.pos + 1 < bytes.len() {
                // Skip the escape char and the escaped byte, regardless of
                // what it is (preserves round-tripping).
                self.pos += 2;
                continue;
            }
            // Newlines inside a quoted string are tolerated (the BuildKit
            // frontend treats double-quoted strings as multi-line).
            self.pos += 1;
        }
        self.at_line_start = false;
        if terminated {
            SyntaxKind::STRING
        } else {
            SyntaxKind::ERROR
        }
    }

    fn scan_word(&mut self) -> SyntaxKind {
        let bytes = self.src.as_bytes();
        while self.pos < bytes.len() {
            let b = bytes[self.pos];
            // Word boundary characters.
            if matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'"' | b'\'') {
                break;
            }
            // Stop *before* a line continuation; the next call will emit it
            // as trivia. A bare escape char that is *not* followed by a
            // newline is a literal byte in the word.
            if b == self.escape && self.is_newline_at(self.pos + 1) {
                break;
            }
            self.pos += 1;
        }
        self.at_line_start = false;
        SyntaxKind::WORD
    }

    fn is_newline_at(&self, idx: usize) -> bool {
        let bytes = self.src.as_bytes();
        matches!(bytes.get(idx), Some(&b'\n' | &b'\r'))
    }

    fn consume_newline(&mut self) {
        let bytes = self.src.as_bytes();
        match bytes.get(self.pos) {
            Some(&b'\r') => {
                self.pos += 1;
                if bytes.get(self.pos) == Some(&b'\n') {
                    self.pos += 1;
                }
            }
            Some(&b'\n') => {
                self.pos += 1;
            }
            _ => {}
        }
    }
}

impl Iterator for Lexer<'_> {
    type Item = Token;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_token()
    }
}

/// Tokenize an entire source string into a `Vec<Token>` using default
/// settings. Convenience over building a [`Lexer`] manually.
pub fn lex(src: &str) -> Vec<Token> {
    Lexer::new(src).collect()
}

/// Tokenize using a custom escape character.
pub fn lex_with_escape(src: &str, escape: char) -> Vec<Token> {
    Lexer::with_escape(src, escape).collect()
}

// ----------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------

fn text_size_of(idx: usize) -> TextSize {
    TextSize::try_from(idx).expect("source larger than u32::MAX bytes")
}

fn next_newline(bytes: &[u8], from: usize) -> usize {
    let mut i = from;
    while i < bytes.len() && !matches!(bytes[i], b'\n' | b'\r') {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests;
