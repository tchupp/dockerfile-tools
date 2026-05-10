//! Typed AST wrappers around the [`dockerfile_syntax`] rowan tree.
//!
//! This crate provides a strongly-typed view over the lossless concrete
//! syntax tree produced by [`dockerfile_parser`]. Each Dockerfile instruction
//! has a corresponding `*Stmt` type whose methods extract semantic
//! information (image references, key-value pairs, flags, exec/shell form,
//! etc.) without losing the underlying syntax tree.
//!
//! The wrappers are *cheap*: they hold a single [`SyntaxNode`] and are zero
//! cost to clone. Source text is borrowed from the tree on demand — no
//! eager-parsing into owned `String`s — which keeps mutations efficient
//! through rowan's green-tree edits.
//!
//! # Example
//!
//! ```
//! use dockerfile_parser::parse;
//! use dockerfile_ast::{AstNode, Dockerfile, Stmt};
//!
//! let parse = parse("FROM rust:1.80 AS builder\nRUN cargo build\n");
//! let dockerfile = Dockerfile::cast(parse.syntax()).unwrap();
//! for stmt in dockerfile.statements() {
//!     match stmt {
//!         Stmt::From(from) => {
//!             assert_eq!(from.image_ref().unwrap().text(), "rust:1.80");
//!             assert_eq!(from.stage_name().as_deref(), Some("builder"));
//!         }
//!         Stmt::Run(run) => {
//!             assert!(run.is_shell_form());
//!         }
//!         _ => {}
//!     }
//! }
//! ```
//!
//! [`dockerfile_parser`]: https://docs.rs/dockerfile_parser

use dockerfile_syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

/// Trait implemented by every typed AST node. Allows safe casting from a
/// raw [`SyntaxNode`] of the appropriate [`SyntaxKind`].
pub trait AstNode: Sized {
    fn can_cast(kind: SyntaxKind) -> bool;
    fn cast(node: SyntaxNode) -> Option<Self>;
    fn syntax(&self) -> &SyntaxNode;

    /// Source text covered by this node (round-trips exactly).
    fn text(&self) -> String {
        self.syntax().text().to_string()
    }
}

/// Macro: define a typed wrapper around a [`SyntaxNode`] of a specific kind.
///
/// Generates `Debug`, `Clone`, and an [`AstNode`] impl.
macro_rules! ast_node {
    ($name:ident, $kind:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(SyntaxNode);

        impl AstNode for $name {
            fn can_cast(kind: SyntaxKind) -> bool {
                kind == $kind
            }
            fn cast(node: SyntaxNode) -> Option<Self> {
                if Self::can_cast(node.kind()) {
                    Some(Self(node))
                } else {
                    None
                }
            }
            fn syntax(&self) -> &SyntaxNode {
                &self.0
            }
        }
    };
}

// ----------------------------------------------------------------------
// Document root
// ----------------------------------------------------------------------

ast_node!(Dockerfile, SyntaxKind::DOCKERFILE);

impl Dockerfile {
    /// Iterate over recognized parser directives at the top of the file.
    pub fn parser_directives(&self) -> impl Iterator<Item = ParserDirective> + '_ {
        self.0.children().filter_map(ParserDirective::cast)
    }

    /// Iterate over the statements in the Dockerfile, in source order.
    pub fn statements(&self) -> impl Iterator<Item = Stmt> + '_ {
        self.0.children().filter_map(Stmt::cast_node)
    }
}

ast_node!(ParserDirective, SyntaxKind::PARSER_DIRECTIVE);

impl ParserDirective {
    /// Returns `(name, value)` parsed from the directive's COMMENT text.
    /// The name is lower-cased to match the `BuildKit` convention.
    pub fn name_and_value(&self) -> Option<(String, String)> {
        let raw = self.text();
        // Strip the leading `#`.
        let body = raw.strip_prefix('#')?;
        let (name, value) = body.split_once('=')?;
        Some((name.trim().to_ascii_lowercase(), value.trim().to_string()))
    }

    pub fn name(&self) -> Option<String> {
        self.name_and_value().map(|(n, _)| n)
    }

    pub fn value(&self) -> Option<String> {
        self.name_and_value().map(|(_, v)| v)
    }
}

// ----------------------------------------------------------------------
// Statement (instruction) enum
// ----------------------------------------------------------------------

/// Sum type over every recognized instruction.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Stmt {
    From(FromStmt),
    Run(RunStmt),
    Cmd(CmdStmt),
    Label(LabelStmt),
    Maintainer(MaintainerStmt),
    Expose(ExposeStmt),
    Env(EnvStmt),
    Add(AddStmt),
    Copy(CopyStmt),
    Entrypoint(EntrypointStmt),
    Volume(VolumeStmt),
    User(UserStmt),
    Workdir(WorkdirStmt),
    Arg(ArgStmt),
    Onbuild(OnbuildStmt),
    Stopsignal(StopsignalStmt),
    Healthcheck(HealthcheckStmt),
    Shell(ShellStmt),
    /// An instruction that the parser couldn't recognize as a known keyword.
    Unknown(UnknownStmt),
}

impl Stmt {
    pub(crate) fn cast_node(node: SyntaxNode) -> Option<Self> {
        Some(match node.kind() {
            SyntaxKind::FROM_INSTR => Self::From(FromStmt(node)),
            SyntaxKind::RUN_INSTR => Self::Run(RunStmt(node)),
            SyntaxKind::CMD_INSTR => Self::Cmd(CmdStmt(node)),
            SyntaxKind::LABEL_INSTR => Self::Label(LabelStmt(node)),
            SyntaxKind::MAINTAINER_INSTR => Self::Maintainer(MaintainerStmt(node)),
            SyntaxKind::EXPOSE_INSTR => Self::Expose(ExposeStmt(node)),
            SyntaxKind::ENV_INSTR => Self::Env(EnvStmt(node)),
            SyntaxKind::ADD_INSTR => Self::Add(AddStmt(node)),
            SyntaxKind::COPY_INSTR => Self::Copy(CopyStmt(node)),
            SyntaxKind::ENTRYPOINT_INSTR => Self::Entrypoint(EntrypointStmt(node)),
            SyntaxKind::VOLUME_INSTR => Self::Volume(VolumeStmt(node)),
            SyntaxKind::USER_INSTR => Self::User(UserStmt(node)),
            SyntaxKind::WORKDIR_INSTR => Self::Workdir(WorkdirStmt(node)),
            SyntaxKind::ARG_INSTR => Self::Arg(ArgStmt(node)),
            SyntaxKind::ONBUILD_INSTR => Self::Onbuild(OnbuildStmt(node)),
            SyntaxKind::STOPSIGNAL_INSTR => Self::Stopsignal(StopsignalStmt(node)),
            SyntaxKind::HEALTHCHECK_INSTR => Self::Healthcheck(HealthcheckStmt(node)),
            SyntaxKind::SHELL_INSTR => Self::Shell(ShellStmt(node)),
            SyntaxKind::INSTRUCTION => Self::Unknown(UnknownStmt(node)),
            _ => return None,
        })
    }

    pub fn syntax(&self) -> &SyntaxNode {
        match self {
            Self::From(s) => s.syntax(),
            Self::Run(s) => s.syntax(),
            Self::Cmd(s) => s.syntax(),
            Self::Label(s) => s.syntax(),
            Self::Maintainer(s) => s.syntax(),
            Self::Expose(s) => s.syntax(),
            Self::Env(s) => s.syntax(),
            Self::Add(s) => s.syntax(),
            Self::Copy(s) => s.syntax(),
            Self::Entrypoint(s) => s.syntax(),
            Self::Volume(s) => s.syntax(),
            Self::User(s) => s.syntax(),
            Self::Workdir(s) => s.syntax(),
            Self::Arg(s) => s.syntax(),
            Self::Onbuild(s) => s.syntax(),
            Self::Stopsignal(s) => s.syntax(),
            Self::Healthcheck(s) => s.syntax(),
            Self::Shell(s) => s.syntax(),
            Self::Unknown(s) => s.syntax(),
        }
    }
}

// ----------------------------------------------------------------------
// One typed node per instruction kind
// ----------------------------------------------------------------------

ast_node!(FromStmt, SyntaxKind::FROM_INSTR);
ast_node!(RunStmt, SyntaxKind::RUN_INSTR);
ast_node!(CmdStmt, SyntaxKind::CMD_INSTR);
ast_node!(LabelStmt, SyntaxKind::LABEL_INSTR);
ast_node!(MaintainerStmt, SyntaxKind::MAINTAINER_INSTR);
ast_node!(ExposeStmt, SyntaxKind::EXPOSE_INSTR);
ast_node!(EnvStmt, SyntaxKind::ENV_INSTR);
ast_node!(AddStmt, SyntaxKind::ADD_INSTR);
ast_node!(CopyStmt, SyntaxKind::COPY_INSTR);
ast_node!(EntrypointStmt, SyntaxKind::ENTRYPOINT_INSTR);
ast_node!(VolumeStmt, SyntaxKind::VOLUME_INSTR);
ast_node!(UserStmt, SyntaxKind::USER_INSTR);
ast_node!(WorkdirStmt, SyntaxKind::WORKDIR_INSTR);
ast_node!(ArgStmt, SyntaxKind::ARG_INSTR);
ast_node!(OnbuildStmt, SyntaxKind::ONBUILD_INSTR);
ast_node!(StopsignalStmt, SyntaxKind::STOPSIGNAL_INSTR);
ast_node!(HealthcheckStmt, SyntaxKind::HEALTHCHECK_INSTR);
ast_node!(ShellStmt, SyntaxKind::SHELL_INSTR);
ast_node!(UnknownStmt, SyntaxKind::INSTRUCTION);

// Sub-nodes:
ast_node!(Flag, SyntaxKind::FLAG);
ast_node!(KeyValue, SyntaxKind::KEY_VALUE);
ast_node!(ExecForm, SyntaxKind::EXEC_FORM);
ast_node!(ShellForm, SyntaxKind::SHELL_FORM);
ast_node!(ImageRef, SyntaxKind::IMAGE_REF);
ast_node!(StageName, SyntaxKind::STAGE_NAME);

// ----------------------------------------------------------------------
// Helpers shared across instruction wrappers
// ----------------------------------------------------------------------

fn first_child<T: AstNode>(node: &SyntaxNode) -> Option<T> {
    node.children().find_map(T::cast)
}

fn children<'a, T: AstNode + 'a>(node: &'a SyntaxNode) -> impl Iterator<Item = T> + 'a {
    node.children().filter_map(T::cast)
}

/// Find the first significant token (skipping trivia) whose kind matches one
/// of `kinds`.
fn find_token(node: &SyntaxNode, kinds: &[SyntaxKind]) -> Option<SyntaxToken> {
    node.children_with_tokens().find_map(|el| {
        let tok = el.into_token()?;
        if !tok.kind().is_trivia() && kinds.contains(&tok.kind()) {
            Some(tok)
        } else {
            None
        }
    })
}

// ----------------------------------------------------------------------
// FROM
// ----------------------------------------------------------------------

impl FromStmt {
    /// `--platform=<p>` etc.
    pub fn flags(&self) -> impl Iterator<Item = Flag> + '_ {
        children(&self.0)
    }

    /// The `<image>[:<tag>][@<digest>]` clause.
    pub fn image_ref(&self) -> Option<ImageRef> {
        first_child(&self.0)
    }

    /// The `<name>` of an `AS <name>` clause, if any.
    pub fn stage_name(&self) -> Option<String> {
        first_child::<StageName>(&self.0).map(|s| s.text())
    }
}

impl ImageRef {
    /// The full text of the image reference, e.g. `rust:1.80-alpine`.
    pub fn full(&self) -> String {
        self.text()
    }

    /// Split the reference into `(name, tag, digest)`. The name is required;
    /// tag and digest are optional.
    pub fn parts(&self) -> ImageRefParts {
        let raw = self.full();
        let (head, digest) = match raw.split_once('@') {
            Some((h, d)) => (h.to_string(), Some(d.to_string())),
            None => (raw, None),
        };
        // The colon for the tag is the *last* colon (registry hosts may
        // contain a `:port`, but the tag can't).
        let (name, tag) = match head.rsplit_once(':') {
            Some((n, t)) if !n.is_empty() && !t.contains('/') => {
                (n.to_string(), Some(t.to_string()))
            }
            _ => (head, None),
        };
        ImageRefParts { name, tag, digest }
    }
}

/// Parsed components of an [`ImageRef`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRefParts {
    pub name: String,
    pub tag: Option<String>,
    pub digest: Option<String>,
}

// ----------------------------------------------------------------------
// RUN / CMD / ENTRYPOINT — shared "exec or shell" interface
// ----------------------------------------------------------------------

/// Form of a `RUN`/`CMD`/`ENTRYPOINT` body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandForm {
    /// `["a", "b"]`. The vector contains the parsed string arguments.
    Exec(Vec<String>),
    /// Free-form shell text.
    Shell(String),
    /// No body present (or only flags) — diagnostic-worthy.
    Empty,
}

macro_rules! impl_runlike {
    ($ty:ident) => {
        impl $ty {
            /// All flags attached to the instruction (e.g. `--mount`).
            pub fn flags(&self) -> impl Iterator<Item = Flag> + '_ {
                children(&self.0)
            }

            /// Whether the body is in shell form.
            pub fn is_shell_form(&self) -> bool {
                first_child::<ShellForm>(&self.0).is_some()
            }

            /// Whether the body is in exec form.
            pub fn is_exec_form(&self) -> bool {
                first_child::<ExecForm>(&self.0).is_some()
            }

            /// Parse the body into a [`CommandForm`].
            pub fn body(&self) -> CommandForm {
                if let Some(exec) = first_child::<ExecForm>(&self.0) {
                    CommandForm::Exec(exec.parsed_args())
                } else if let Some(shell) = first_child::<ShellForm>(&self.0) {
                    CommandForm::Shell(shell.text())
                } else {
                    CommandForm::Empty
                }
            }
        }
    };
}

impl_runlike!(RunStmt);
impl_runlike!(CmdStmt);
impl_runlike!(EntrypointStmt);

impl ExecForm {
    /// Best-effort parse of the exec-form arguments. Quoted strings are
    /// unescaped; bare words are returned as-is. `[`, `]`, and `,` separators
    /// are skipped.
    pub fn parsed_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        for tok in self
            .0
            .children_with_tokens()
            .filter_map(rowan::NodeOrToken::into_token)
        {
            match tok.kind() {
                SyntaxKind::STRING => {
                    args.push(unquote(tok.text()));
                }
                SyntaxKind::WORD => {
                    let text = tok.text();
                    let stripped: &str = text
                        .trim_start_matches(['[', ','])
                        .trim_end_matches([']', ',']);
                    if !stripped.is_empty() {
                        args.push(stripped.to_string());
                    }
                }
                _ => {}
            }
        }
        args
    }
}

fn unquote(s: &str) -> String {
    let bytes = s.as_bytes();
    if bytes.len() < 2 {
        return s.to_string();
    }
    let q = bytes[0];
    if (q == b'"' || q == b'\'') && bytes[bytes.len() - 1] == q {
        let inner = &s[1..s.len() - 1];
        if q == b'"' {
            // Unescape `\"` and `\\` minimally.
            let mut out = String::with_capacity(inner.len());
            let mut chars = inner.chars();
            while let Some(c) = chars.next() {
                if c == '\\'
                    && let Some(n) = chars.next()
                {
                    out.push(n);
                } else {
                    out.push(c);
                }
            }
            return out;
        }
        return inner.to_string();
    }
    s.to_string()
}

// ----------------------------------------------------------------------
// ENV / LABEL / ARG — key-value extraction
// ----------------------------------------------------------------------

impl KeyValue {
    /// The key portion of the pair.
    pub fn key(&self) -> Option<String> {
        let tok = self
            .0
            .children_with_tokens()
            .filter_map(rowan::NodeOrToken::into_token)
            .find(|t| t.kind() == SyntaxKind::WORD)?;
        Some(tok.text().to_string())
    }

    /// The value portion of the pair, with surrounding quotes stripped.
    /// Returns `None` if the pair has no value (e.g. legacy `ARG name`).
    pub fn value(&self) -> Option<String> {
        let mut saw_eq = false;
        for el in self.0.children_with_tokens() {
            let Some(tok) = el.into_token() else { continue };
            if tok.kind() == SyntaxKind::EQUALS {
                saw_eq = true;
                continue;
            }
            if !saw_eq {
                continue;
            }
            match tok.kind() {
                SyntaxKind::WORD => return Some(tok.text().to_string()),
                SyntaxKind::STRING => return Some(unquote(tok.text())),
                _ => {}
            }
        }
        None
    }
}

impl EnvStmt {
    pub fn pairs(&self) -> impl Iterator<Item = KeyValue> + '_ {
        children(&self.0)
    }
}

impl LabelStmt {
    pub fn pairs(&self) -> impl Iterator<Item = KeyValue> + '_ {
        children(&self.0)
    }
}

impl ArgStmt {
    /// `ARG` only allows one argument per instruction.
    pub fn pair(&self) -> Option<KeyValue> {
        first_child(&self.0)
    }

    pub fn name(&self) -> Option<String> {
        self.pair()?.key()
    }

    pub fn default_value(&self) -> Option<String> {
        self.pair()?.value()
    }
}

// ----------------------------------------------------------------------
// COPY / ADD
// ----------------------------------------------------------------------

macro_rules! impl_copylike {
    ($ty:ident) => {
        impl $ty {
            pub fn flags(&self) -> impl Iterator<Item = Flag> + '_ {
                children(&self.0)
            }

            /// Source paths (everything but the last positional WORD).
            pub fn sources(&self) -> Vec<String> {
                let words: Vec<String> = self
                    .0
                    .children_with_tokens()
                    .filter_map(rowan::NodeOrToken::into_token)
                    .filter(|t| t.kind() == SyntaxKind::WORD)
                    .map(|t| t.text().to_string())
                    .collect();
                // The first WORD is the instruction keyword itself.
                if words.len() < 3 {
                    return Vec::new();
                }
                words[1..words.len() - 1].to_vec()
            }

            /// Destination path (the last positional WORD).
            pub fn dest(&self) -> Option<String> {
                let words: Vec<String> = self
                    .0
                    .children_with_tokens()
                    .filter_map(rowan::NodeOrToken::into_token)
                    .filter(|t| t.kind() == SyntaxKind::WORD)
                    .map(|t| t.text().to_string())
                    .collect();
                if words.len() < 2 {
                    None
                } else {
                    Some(words.last().cloned().unwrap())
                }
            }

            /// The value of the `--from=` flag, if present.
            pub fn from(&self) -> Option<String> {
                self.flags().find_map(|f| {
                    if f.name().as_deref() == Some("from") {
                        f.value()
                    } else {
                        None
                    }
                })
            }
        }
    };
}

impl_copylike!(CopyStmt);
impl_copylike!(AddStmt);

// ----------------------------------------------------------------------
// Flag
// ----------------------------------------------------------------------

impl Flag {
    /// `--mount` -> `"mount"`. Returns `None` if the flag text is malformed.
    pub fn name(&self) -> Option<String> {
        let tok = self
            .0
            .children_with_tokens()
            .filter_map(rowan::NodeOrToken::into_token)
            .find(|t| t.kind() == SyntaxKind::WORD)?;
        Some(tok.text().trim_start_matches('-').to_string())
    }

    /// `--mount=type=cache,target=/foo` -> `"type=cache,target=/foo"`.
    /// Returns `None` for value-less flags.
    pub fn value(&self) -> Option<String> {
        let mut saw_eq = false;
        for el in self.0.children_with_tokens() {
            let Some(tok) = el.into_token() else { continue };
            if tok.kind() == SyntaxKind::EQUALS {
                saw_eq = true;
                continue;
            }
            if !saw_eq {
                continue;
            }
            match tok.kind() {
                SyntaxKind::WORD => return Some(tok.text().to_string()),
                SyntaxKind::STRING => return Some(unquote(tok.text())),
                _ => {}
            }
        }
        None
    }
}

// ----------------------------------------------------------------------
// Misc — instructions that accept a single positional argument.
// ----------------------------------------------------------------------

impl WorkdirStmt {
    pub fn path(&self) -> Option<String> {
        first_positional(&self.0)
    }
}

impl UserStmt {
    pub fn user(&self) -> Option<String> {
        first_positional(&self.0)
    }
}

impl ExposeStmt {
    /// All ports listed in the `EXPOSE` instruction (e.g. `80/tcp`,
    /// `443/udp`, or just `8080`).
    pub fn ports(&self) -> Vec<String> {
        positional_args(&self.0)
    }
}

impl VolumeStmt {
    pub fn volumes(&self) -> Vec<String> {
        positional_args(&self.0)
    }
}

impl StopsignalStmt {
    pub fn signal(&self) -> Option<String> {
        first_positional(&self.0)
    }
}

impl MaintainerStmt {
    pub fn name(&self) -> Option<String> {
        // MAINTAINER takes free-form text — return everything after the
        // keyword, trimmed.
        let text = self.text();
        let trimmed = text.trim_start();
        let (_, rest) = trimmed.split_once(char::is_whitespace)?;
        Some(rest.trim().to_string())
    }
}

/// Return all WORD tokens after the leading instruction keyword. Used by
/// instructions that take one or more positional arguments.
fn positional_args(node: &SyntaxNode) -> Vec<String> {
    let mut words: Vec<String> = node
        .children_with_tokens()
        .filter_map(rowan::NodeOrToken::into_token)
        .filter(|t| t.kind() == SyntaxKind::WORD)
        .map(|t| t.text().to_string())
        .collect();
    if !words.is_empty() {
        words.remove(0);
    }
    words
}

fn first_positional(node: &SyntaxNode) -> Option<String> {
    positional_args(node).into_iter().next()
}

// Suppress unused-import warning for the helper that's only referenced
// inside a few macros.
#[allow(dead_code)]
fn _silence(node: &SyntaxNode) -> Option<SyntaxToken> {
    find_token(node, &[])
}

#[cfg(test)]
mod tests;
