//! The [`SyntaxKind`] enum.

/// Every kind of token (leaf) and node (interior) that can appear in a
/// Dockerfile syntax tree.
///
/// The discriminants are explicit so that the value can be safely round-tripped
/// through [`rowan::SyntaxKind`] (which is a `u16`).
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
// Note: we deliberately use a `__LAST` sentinel rather than `#[non_exhaustive]`
// because we need a compile-time count of the variants for `from_u16` and the
// `ALL_KINDS` lookup table.
#[allow(clippy::manual_non_exhaustive)]
pub enum SyntaxKind {
    // ------------------------------------------------------------------
    // Trivia tokens (whitespace, comments, continuations)
    // ------------------------------------------------------------------
    /// Spaces and tabs (no newlines).
    WHITESPACE = 0,
    /// A single `\n` or `\r\n`.
    NEWLINE,
    /// A `# ...` comment that is *not* a parser directive.
    COMMENT,
    /// A backslash (or configured escape char) immediately followed by a
    /// newline. Joins logical lines while remaining trivia in the tree.
    LINE_CONTINUATION,

    // ------------------------------------------------------------------
    // Leaf tokens
    // ------------------------------------------------------------------
    /// A bareword: instruction name, image name, flag value fragment, etc.
    /// The lexer does not distinguish instruction keywords from other words at
    /// the token level — the parser interprets the first word of each logical
    /// line as the instruction name.
    WORD,
    /// A double- or single-quoted string, including the surrounding quotes.
    STRING,
    /// An integer literal (as it appears in EXPOSE, HEALTHCHECK, etc.).
    INT,
    /// `=`
    EQUALS,
    /// `:`
    COLON,
    /// `,`
    COMMA,
    /// `[`
    L_BRACKET,
    /// `]`
    R_BRACKET,
    /// `{`
    L_BRACE,
    /// `}`
    R_BRACE,
    /// `@` (used in image digests: `image@sha256:...`).
    AT,
    /// `--` introducing a flag (e.g. `--mount`, `--from`).
    DASH_DASH,
    /// `<<` or `<<-` introducing a heredoc.
    HEREDOC_START,
    /// The body of a heredoc, up to and including the closing delimiter line.
    HEREDOC_BODY,
    /// The opaque shell-form body of a `RUN`/`CMD`/`ENTRYPOINT`. Preserved as
    /// a single token so consumers (e.g. an `shfmt` integration) can treat it
    /// as raw shell.
    SHELL_BODY,
    /// A `${...}` or `$NAME` build-arg/environment expansion. (Tracked as a
    /// single token; structural parsing of the expansion is not yet done.)
    DOLLAR_EXPANSION,
    /// Lexical error token. Always accompanied by a diagnostic.
    ERROR,

    // ------------------------------------------------------------------
    // Node kinds
    // ------------------------------------------------------------------
    /// Top-level node: the whole Dockerfile.
    DOCKERFILE,

    /// A parser directive (`# directive=value`) appearing in the leading
    /// directive block.
    PARSER_DIRECTIVE,

    /// Wrapper for any single instruction; the first child indicates which
    /// concrete instruction node follows.
    INSTRUCTION,

    // -- one node kind per Dockerfile instruction --
    FROM_INSTR,
    RUN_INSTR,
    CMD_INSTR,
    LABEL_INSTR,
    MAINTAINER_INSTR,
    EXPOSE_INSTR,
    ENV_INSTR,
    ADD_INSTR,
    COPY_INSTR,
    ENTRYPOINT_INSTR,
    VOLUME_INSTR,
    USER_INSTR,
    WORKDIR_INSTR,
    ARG_INSTR,
    ONBUILD_INSTR,
    STOPSIGNAL_INSTR,
    HEALTHCHECK_INSTR,
    SHELL_INSTR,

    // -- structural sub-nodes used by multiple instructions --
    /// `--name` or `--name=value` flag.
    FLAG,
    /// `key=value` pair (used by ENV, LABEL, ARG, and flag values).
    KEY_VALUE,
    /// JSON-array exec form: `["a", "b"]`.
    EXEC_FORM,
    /// Shell-form body: free-form text up to end-of-(logical-)line.
    SHELL_FORM,
    /// A heredoc block (`<<EOF ... EOF`).
    HEREDOC,
    /// An image reference: `[registry/]name[:tag][@digest]`.
    IMAGE_REF,
    /// `AS <name>` clause on a `FROM`.
    STAGE_NAME,

    // ------------------------------------------------------------------
    // Sentinel — keep last.
    // ------------------------------------------------------------------
    #[doc(hidden)]
    __LAST,
}

impl SyntaxKind {
    /// Round-trip from a raw `u16` discriminant. Returns `None` for
    /// out-of-range values.
    pub fn from_u16(raw: u16) -> Option<Self> {
        if raw >= Self::__LAST as u16 {
            return None;
        }
        // SAFETY: `repr(u16)` enum with contiguous discriminants 0..__LAST.
        // We just bounds-checked above. We forbid `unsafe_code` at the crate
        // level, so use a match table instead.
        Some(ALL_KINDS[raw as usize])
    }

    /// Whether this kind is a *trivia* token (whitespace, newline, comment,
    /// line continuation). Trivia tokens are skipped by the parser when
    /// looking for the "next real token", but are preserved in the tree.
    #[inline]
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            Self::WHITESPACE | Self::NEWLINE | Self::COMMENT | Self::LINE_CONTINUATION
        )
    }

    /// Whether this kind is a token (leaf), as opposed to a node (interior).
    #[inline]
    pub fn is_token(self) -> bool {
        (self as u16) < Self::DOCKERFILE as u16
    }

    /// Whether this kind is a node (interior).
    #[inline]
    pub fn is_node(self) -> bool {
        !self.is_token()
    }
}

/// Lookup table that maps each discriminant to its [`SyntaxKind`] value, used
/// by [`SyntaxKind::from_u16`] without needing `unsafe`.
const ALL_KINDS: &[SyntaxKind] = &[
    SyntaxKind::WHITESPACE,
    SyntaxKind::NEWLINE,
    SyntaxKind::COMMENT,
    SyntaxKind::LINE_CONTINUATION,
    SyntaxKind::WORD,
    SyntaxKind::STRING,
    SyntaxKind::INT,
    SyntaxKind::EQUALS,
    SyntaxKind::COLON,
    SyntaxKind::COMMA,
    SyntaxKind::L_BRACKET,
    SyntaxKind::R_BRACKET,
    SyntaxKind::L_BRACE,
    SyntaxKind::R_BRACE,
    SyntaxKind::AT,
    SyntaxKind::DASH_DASH,
    SyntaxKind::HEREDOC_START,
    SyntaxKind::HEREDOC_BODY,
    SyntaxKind::SHELL_BODY,
    SyntaxKind::DOLLAR_EXPANSION,
    SyntaxKind::ERROR,
    SyntaxKind::DOCKERFILE,
    SyntaxKind::PARSER_DIRECTIVE,
    SyntaxKind::INSTRUCTION,
    SyntaxKind::FROM_INSTR,
    SyntaxKind::RUN_INSTR,
    SyntaxKind::CMD_INSTR,
    SyntaxKind::LABEL_INSTR,
    SyntaxKind::MAINTAINER_INSTR,
    SyntaxKind::EXPOSE_INSTR,
    SyntaxKind::ENV_INSTR,
    SyntaxKind::ADD_INSTR,
    SyntaxKind::COPY_INSTR,
    SyntaxKind::ENTRYPOINT_INSTR,
    SyntaxKind::VOLUME_INSTR,
    SyntaxKind::USER_INSTR,
    SyntaxKind::WORKDIR_INSTR,
    SyntaxKind::ARG_INSTR,
    SyntaxKind::ONBUILD_INSTR,
    SyntaxKind::STOPSIGNAL_INSTR,
    SyntaxKind::HEALTHCHECK_INSTR,
    SyntaxKind::SHELL_INSTR,
    SyntaxKind::FLAG,
    SyntaxKind::KEY_VALUE,
    SyntaxKind::EXEC_FORM,
    SyntaxKind::SHELL_FORM,
    SyntaxKind::HEREDOC,
    SyntaxKind::IMAGE_REF,
    SyntaxKind::STAGE_NAME,
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The lookup table must mirror the enum exactly, including order.
    #[test]
    fn all_kinds_table_matches_enum_layout() {
        assert_eq!(ALL_KINDS.len(), SyntaxKind::__LAST as usize);
        for (i, kind) in ALL_KINDS.iter().enumerate() {
            assert_eq!(*kind as usize, i, "ALL_KINDS[{i}] is misordered");
        }
    }

    #[test]
    fn round_trip_u16() {
        for kind in ALL_KINDS {
            assert_eq!(SyntaxKind::from_u16(*kind as u16), Some(*kind));
        }
        assert_eq!(SyntaxKind::from_u16(SyntaxKind::__LAST as u16), None);
    }

    #[test]
    fn trivia_classification() {
        assert!(SyntaxKind::WHITESPACE.is_trivia());
        assert!(SyntaxKind::NEWLINE.is_trivia());
        assert!(SyntaxKind::COMMENT.is_trivia());
        assert!(SyntaxKind::LINE_CONTINUATION.is_trivia());
        assert!(!SyntaxKind::WORD.is_trivia());
        assert!(!SyntaxKind::FROM_INSTR.is_trivia());
    }

    #[test]
    fn token_vs_node_classification() {
        assert!(SyntaxKind::WORD.is_token());
        assert!(!SyntaxKind::WORD.is_node());
        assert!(SyntaxKind::FROM_INSTR.is_node());
        assert!(!SyntaxKind::FROM_INSTR.is_token());
    }
}
