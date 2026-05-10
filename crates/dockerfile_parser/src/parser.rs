//! The main parser implementation.
//!
//! Responsibilities:
//!
//! - Walk the token stream produced by [`dockerfile_lexer`].
//! - Build a [`rowan::GreenNode`] tree shaped according to [`SyntaxKind`].
//! - Recognize instruction keywords case-insensitively and dispatch to
//!   per-instruction sub-parsers that add structural sub-nodes (flags,
//!   key-value pairs, exec form, `AS` stage names, etc.).
//! - Wrap leading parser-directive comments in [`SyntaxKind::PARSER_DIRECTIVE`]
//!   nodes (the directive locations are taken from the pre-pass result).
//!
//! # Trivia handling
//!
//! Trivia (whitespace, newlines, comments, line continuations) that sits
//! *between* instructions stays at the [`SyntaxKind::DOCKERFILE`] level.
//! Trivia *within* an instruction's logical line — including any
//! [`SyntaxKind::LINE_CONTINUATION`] tokens — is captured inside the
//! instruction's node. The terminating newline of an instruction is *not*
//! consumed by the instruction node; it remains at the document level so
//! every instruction node's text equals the instruction's logical-line body.

use dockerfile_diagnostics::{Diagnostic, range as drange};
use dockerfile_lexer::{Lexer, Token};
use dockerfile_syntax::SyntaxKind;
use rowan::{GreenNode, GreenNodeBuilder};

use crate::directive::ParserDirective;

/// Internal parser state.
pub(crate) struct Parser<'a> {
    src: &'a str,
    tokens: Vec<Token>,
    pos: usize,
    builder: GreenNodeBuilder<'static>,
    diagnostics: Vec<Diagnostic>,
    directives: Vec<ParserDirective>,
}

impl<'a> Parser<'a> {
    pub(crate) fn new(src: &'a str, escape: char, directives: Vec<ParserDirective>) -> Self {
        let tokens: Vec<Token> = Lexer::with_escape(src, escape).collect();
        Self {
            src,
            tokens,
            pos: 0,
            builder: GreenNodeBuilder::new(),
            diagnostics: Vec::new(),
            directives,
        }
    }

    pub(crate) fn parse(mut self) -> (GreenNode, Vec<Diagnostic>) {
        self.builder.start_node(SyntaxKind::DOCKERFILE.into());
        self.parse_directive_block();
        while !self.is_eof() {
            self.parse_top_level_item();
        }
        self.builder.finish_node();
        (self.builder.finish(), self.diagnostics)
    }

    // ------------------------------------------------------------------
    // Token cursor helpers
    // ------------------------------------------------------------------

    fn is_eof(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn peek(&self) -> Option<Token> {
        self.tokens.get(self.pos).copied()
    }

    /// Peek the next *non-trivia* token without advancing the cursor.
    fn peek_significant(&self) -> Option<Token> {
        let mut idx = self.pos;
        while let Some(tok) = self.tokens.get(idx) {
            if !tok.kind.is_trivia() {
                return Some(*tok);
            }
            idx += 1;
        }
        None
    }

    fn token_text(&self, tok: Token) -> &'a str {
        tok.text(self.src)
    }

    /// Push the current token into the builder (whatever level is open) and
    /// advance.
    fn bump(&mut self) {
        let tok = self.tokens[self.pos];
        let text = self.token_text(tok);
        self.builder.token(tok.kind.into(), text);
        self.pos += 1;
    }

    /// Bump consecutive trivia tokens into the currently open node.
    #[allow(dead_code)]
    fn bump_trivia(&mut self) {
        while let Some(tok) = self.peek() {
            if !tok.kind.is_trivia() {
                break;
            }
            self.bump();
        }
    }

    /// Bump *non-newline* trivia (whitespace, comments, line continuations)
    /// without crossing a logical-line boundary.
    fn bump_intraline_trivia(&mut self) {
        while let Some(tok) = self.peek() {
            match tok.kind {
                SyntaxKind::WHITESPACE | SyntaxKind::LINE_CONTINUATION | SyntaxKind::COMMENT => {
                    self.bump();
                }
                _ => break,
            }
        }
    }

    // ------------------------------------------------------------------
    // Phase 1: leading directive block
    // ------------------------------------------------------------------

    fn parse_directive_block(&mut self) {
        if self.directives.is_empty() {
            return;
        }
        let directive_ranges: Vec<text_size::TextRange> =
            self.directives.iter().map(|d| d.range).collect();
        let mut iter = directive_ranges.into_iter().peekable();
        while let Some(range) = iter.next() {
            // Bump tokens until we reach the COMMENT covering this directive.
            while let Some(tok) = self.peek() {
                if tok.kind == SyntaxKind::COMMENT && tok.range == range {
                    self.builder.start_node(SyntaxKind::PARSER_DIRECTIVE.into());
                    self.bump();
                    self.builder.finish_node();
                    break;
                }
                if tok.kind == SyntaxKind::COMMENT {
                    // A different comment — shouldn't happen if directives are
                    // contiguous, but bail to be safe.
                    return;
                }
                self.bump();
            }
            // Stop bumping once the directive is consumed; the loop will pick
            // up trivia for the next directive.
            if iter.peek().is_none() {
                return;
            }
        }
    }

    // ------------------------------------------------------------------
    // Phase 2: top-level items
    // ------------------------------------------------------------------

    fn parse_top_level_item(&mut self) {
        let Some(tok) = self.peek() else {
            return;
        };
        if tok.kind.is_trivia() {
            self.bump();
            return;
        }
        if tok.kind == SyntaxKind::WORD {
            self.parse_instruction();
            return;
        }
        // Stray significant token at top level (e.g. a `[`). Wrap the
        // remainder of the logical line as a generic INSTRUCTION node with
        // a diagnostic so consumers can still see the bytes.
        self.diagnostics.push(
            Diagnostic::error(
                format!("unexpected token at top level: {:?}", self.token_text(tok)),
                tok.range,
            )
            .with_code("DF000"),
        );
        self.recover_to_newline(SyntaxKind::INSTRUCTION);
    }

    fn parse_instruction(&mut self) {
        let keyword_tok = self.peek().expect("caller verified WORD ahead");
        let kind = recognize_instruction(self.token_text(keyword_tok));
        let node_kind = kind.unwrap_or(SyntaxKind::INSTRUCTION);
        if kind.is_none() {
            self.diagnostics.push(
                Diagnostic::warning(
                    format!(
                        "unknown Dockerfile instruction: {:?}",
                        self.token_text(keyword_tok)
                    ),
                    keyword_tok.range,
                )
                .with_code("DF001"),
            );
        }
        self.builder.start_node(node_kind.into());
        self.bump(); // keyword

        match node_kind {
            SyntaxKind::FROM_INSTR => self.parse_from_body(),
            SyntaxKind::ENV_INSTR | SyntaxKind::LABEL_INSTR => self.parse_kv_body(),
            SyntaxKind::ARG_INSTR => self.parse_arg_body(),
            SyntaxKind::RUN_INSTR | SyntaxKind::CMD_INSTR | SyntaxKind::ENTRYPOINT_INSTR => {
                self.parse_runlike_body();
            }
            SyntaxKind::COPY_INSTR | SyntaxKind::ADD_INSTR => self.parse_copy_body(),
            _ => self.parse_generic_body(),
        }

        self.builder.finish_node();
    }

    /// Generic body parser: bump everything up to (but not including) the
    /// next [`NEWLINE`] token. [`LINE_CONTINUATION`] tokens are *not* line
    /// terminators.
    ///
    /// [`NEWLINE`]: SyntaxKind::NEWLINE
    /// [`LINE_CONTINUATION`]: SyntaxKind::LINE_CONTINUATION
    fn parse_generic_body(&mut self) {
        while let Some(tok) = self.peek() {
            if tok.kind == SyntaxKind::NEWLINE {
                break;
            }
            if tok.kind == SyntaxKind::WORD && is_flag_word(self.token_text(tok)) {
                self.parse_flag();
                continue;
            }
            self.bump();
        }
    }

    /// Recovery: bump everything to (but not including) the next newline as
    /// children of a node of `kind`.
    fn recover_to_newline(&mut self, kind: SyntaxKind) {
        self.builder.start_node(kind.into());
        while let Some(tok) = self.peek() {
            if tok.kind == SyntaxKind::NEWLINE {
                break;
            }
            self.bump();
        }
        self.builder.finish_node();
    }

    // ------------------------------------------------------------------
    // Per-instruction sub-parsers
    // ------------------------------------------------------------------

    /// `FROM [--platform=<p>] <image>[:<tag>][@<digest>] [AS <name>]`
    fn parse_from_body(&mut self) {
        // Optional flags.
        loop {
            self.bump_intraline_trivia();
            match self.peek() {
                Some(tok) if tok.kind == SyntaxKind::WORD && is_flag_word(self.token_text(tok)) => {
                    self.parse_flag();
                }
                _ => break,
            }
        }
        // Image reference.
        self.bump_intraline_trivia();
        if matches!(self.peek().map(|t| t.kind), Some(k) if k != SyntaxKind::NEWLINE) {
            self.builder.start_node(SyntaxKind::IMAGE_REF.into());
            // The image reference is one or more tokens up to whitespace; in
            // practice the lexer emits it as a single WORD (since we don't
            // split on `:` or `@`).
            if let Some(tok) = self.peek()
                && tok.kind != SyntaxKind::NEWLINE
            {
                self.bump();
            }
            self.builder.finish_node();
        }
        // Optional `AS <name>` clause.
        self.bump_intraline_trivia();
        if let Some(tok) = self.peek()
            && tok.kind == SyntaxKind::WORD
            && self.token_text(tok).eq_ignore_ascii_case("AS")
        {
            // Bump the `AS` keyword.
            self.bump();
            self.bump_intraline_trivia();
            if let Some(name_tok) = self.peek()
                && name_tok.kind != SyntaxKind::NEWLINE
            {
                self.builder.start_node(SyntaxKind::STAGE_NAME.into());
                self.bump();
                self.builder.finish_node();
            } else {
                self.diagnostics.push(
                    Diagnostic::error("expected stage name after `AS`", self.tail_range_at_pos())
                        .with_code("DF010"),
                );
            }
        }
        // Anything trailing up to newline: bump as-is.
        self.parse_generic_body();
    }

    /// `ENV key=value ...` or legacy `ENV key value` (single pair).
    /// `LABEL key=value ...`.
    fn parse_kv_body(&mut self) {
        loop {
            self.bump_intraline_trivia();
            let Some(tok) = self.peek() else {
                break;
            };
            if tok.kind == SyntaxKind::NEWLINE {
                break;
            }
            if tok.kind == SyntaxKind::WORD {
                self.parse_kv_pair();
            } else {
                // Quoted string or other: just bump.
                self.bump();
            }
        }
    }

    /// `ARG name[=default]`. Only one argument per ARG instruction.
    fn parse_arg_body(&mut self) {
        self.bump_intraline_trivia();
        if let Some(tok) = self.peek()
            && tok.kind == SyntaxKind::WORD
        {
            self.parse_kv_pair();
        }
        self.parse_generic_body();
    }

    /// Parse a single `key=value` (or just `key`) pair as a [`KEY_VALUE`]
    /// node. The lexer hands us the whole `key=value` as a single WORD, so
    /// we re-tokenize here to split it.
    ///
    /// [`KEY_VALUE`]: SyntaxKind::KEY_VALUE
    fn parse_kv_pair(&mut self) {
        let tok = self.peek().expect("caller verified WORD ahead");
        let text = self.token_text(tok);
        self.builder.start_node(SyntaxKind::KEY_VALUE.into());
        if let Some(eq_idx) = text.find('=') {
            let start: usize = tok.range.start().into();
            let key = &text[..eq_idx];
            let eq = &text[eq_idx..=eq_idx];
            let after = &text[eq_idx + 1..];
            self.builder.token(SyntaxKind::WORD.into(), key);
            self.builder.token(SyntaxKind::EQUALS.into(), eq);
            if !after.is_empty() {
                self.builder.token(SyntaxKind::WORD.into(), after);
            }
            // Manually advance past the original WORD token.
            let _ = start; // not used; bookkeeping doc for the read above
            self.pos += 1;
            // After the WORD: optional STRING value if `key="..."` happens to
            // have been split because the quote is the boundary.
            if let Some(next) = self.peek()
                && next.kind == SyntaxKind::STRING
                && eq_idx == text.len() - 1
            {
                // The `=` ended the WORD because of the quote — the string is
                // the value.
                self.bump();
            }
        } else {
            // Either a bare key (legacy ENV/ARG) or a value-less LABEL.
            self.bump();
            // Legacy `ENV KEY VALUE` form: the tokens after are the value.
            self.bump_intraline_trivia();
            while let Some(next) = self.peek() {
                if next.kind == SyntaxKind::NEWLINE {
                    break;
                }
                self.bump();
            }
        }
        self.builder.finish_node();
    }

    /// `RUN`/`CMD`/`ENTRYPOINT` body. Recognizes flags, then either an
    /// exec-form JSON array or a shell-form free-text body.
    fn parse_runlike_body(&mut self) {
        // Optional flags.
        loop {
            self.bump_intraline_trivia();
            match self.peek() {
                Some(tok) if tok.kind == SyntaxKind::WORD && is_flag_word(self.token_text(tok)) => {
                    self.parse_flag();
                }
                _ => break,
            }
        }
        self.bump_intraline_trivia();
        match self.peek_significant() {
            Some(tok) if tok.kind == SyntaxKind::WORD && self.token_text(tok).starts_with('[') => {
                self.parse_exec_form();
            }
            Some(tok) if tok.kind != SyntaxKind::NEWLINE => {
                self.parse_shell_form();
            }
            _ => {
                // Empty body — emit a diagnostic; instruction allows zero
                // args is unusual (only HEALTHCHECK NONE etc.).
                self.diagnostics.push(
                    Diagnostic::warning("instruction has no body", self.tail_range_at_pos())
                        .with_code("DF020"),
                );
            }
        }
    }

    /// `COPY`/`ADD` body: flags + sources + dest.
    fn parse_copy_body(&mut self) {
        loop {
            self.bump_intraline_trivia();
            match self.peek() {
                Some(tok) if tok.kind == SyntaxKind::WORD && is_flag_word(self.token_text(tok)) => {
                    self.parse_flag();
                }
                _ => break,
            }
        }
        // Sources and dest: also support the exec-form-style array (yes,
        // `COPY ["a", "b"]` is valid).
        self.bump_intraline_trivia();
        if let Some(tok) = self.peek_significant()
            && tok.kind == SyntaxKind::WORD
            && self.token_text(tok).starts_with('[')
        {
            self.parse_exec_form();
        } else {
            self.parse_generic_body();
        }
    }

    /// Parse a `--flag` or `--flag=value` argument as a [`FLAG`] node.
    ///
    /// [`FLAG`]: SyntaxKind::FLAG
    fn parse_flag(&mut self) {
        let tok = self.peek().expect("caller verified WORD ahead");
        let text = self.token_text(tok);
        self.builder.start_node(SyntaxKind::FLAG.into());
        if let Some(eq_idx) = text.find('=') {
            let key = &text[..eq_idx];
            let eq = &text[eq_idx..=eq_idx];
            let value = &text[eq_idx + 1..];
            self.builder.token(SyntaxKind::WORD.into(), key);
            self.builder.token(SyntaxKind::EQUALS.into(), eq);
            if !value.is_empty() {
                self.builder.token(SyntaxKind::WORD.into(), value);
            }
            self.pos += 1;
            // If the value was quoted, the lexer split the STRING off.
            if let Some(next) = self.peek()
                && next.kind == SyntaxKind::STRING
                && eq_idx == text.len() - 1
            {
                self.bump();
            }
        } else {
            self.bump();
        }
        self.builder.finish_node();
    }

    /// Wrap the rest of the logical line in an [`EXEC_FORM`] node.
    ///
    /// [`EXEC_FORM`]: SyntaxKind::EXEC_FORM
    fn parse_exec_form(&mut self) {
        self.builder.start_node(SyntaxKind::EXEC_FORM.into());
        while let Some(tok) = self.peek() {
            if tok.kind == SyntaxKind::NEWLINE {
                break;
            }
            self.bump();
        }
        self.builder.finish_node();
    }

    /// Wrap the rest of the logical line in a [`SHELL_FORM`] node.
    ///
    /// [`SHELL_FORM`]: SyntaxKind::SHELL_FORM
    fn parse_shell_form(&mut self) {
        self.builder.start_node(SyntaxKind::SHELL_FORM.into());
        while let Some(tok) = self.peek() {
            if tok.kind == SyntaxKind::NEWLINE {
                break;
            }
            self.bump();
        }
        self.builder.finish_node();
    }

    fn tail_range_at_pos(&self) -> text_size::TextRange {
        if let Some(tok) = self.peek() {
            tok.range
        } else if let Some(tok) = self.tokens.last() {
            tok.range
        } else {
            drange(0, 0)
        }
    }
}

/// Recognize a Dockerfile instruction keyword. Returns the corresponding
/// instruction node kind, or `None` if `text` is not a known keyword.
///
/// Recognition is case-insensitive — Docker accepts `from`, `FROM`, `From`,
/// etc. The canonical form is upper-case.
pub fn recognize_instruction(text: &str) -> Option<SyntaxKind> {
    // Avoid an allocation by uppercasing into a stack buffer.
    let mut buf = [0u8; 16];
    if text.len() > buf.len() {
        return None;
    }
    for (i, b) in text.bytes().enumerate() {
        buf[i] = b.to_ascii_uppercase();
    }
    let upper = &buf[..text.len()];
    Some(match upper {
        b"FROM" => SyntaxKind::FROM_INSTR,
        b"RUN" => SyntaxKind::RUN_INSTR,
        b"CMD" => SyntaxKind::CMD_INSTR,
        b"LABEL" => SyntaxKind::LABEL_INSTR,
        b"MAINTAINER" => SyntaxKind::MAINTAINER_INSTR,
        b"EXPOSE" => SyntaxKind::EXPOSE_INSTR,
        b"ENV" => SyntaxKind::ENV_INSTR,
        b"ADD" => SyntaxKind::ADD_INSTR,
        b"COPY" => SyntaxKind::COPY_INSTR,
        b"ENTRYPOINT" => SyntaxKind::ENTRYPOINT_INSTR,
        b"VOLUME" => SyntaxKind::VOLUME_INSTR,
        b"USER" => SyntaxKind::USER_INSTR,
        b"WORKDIR" => SyntaxKind::WORKDIR_INSTR,
        b"ARG" => SyntaxKind::ARG_INSTR,
        b"ONBUILD" => SyntaxKind::ONBUILD_INSTR,
        b"STOPSIGNAL" => SyntaxKind::STOPSIGNAL_INSTR,
        b"HEALTHCHECK" => SyntaxKind::HEALTHCHECK_INSTR,
        b"SHELL" => SyntaxKind::SHELL_INSTR,
        _ => return None,
    })
}

/// `true` if a [`WORD`] token's text is a `--flag` argument.
///
/// We also treat a bare `--` as a flag introducer to match real Dockerfile
/// usage. Single-dash `-foo` is *not* a flag in Dockerfile syntax.
///
/// [`WORD`]: SyntaxKind::WORD
fn is_flag_word(text: &str) -> bool {
    text.starts_with("--") && text.len() >= 2
}
