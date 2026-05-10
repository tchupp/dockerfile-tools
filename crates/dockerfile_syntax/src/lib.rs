//! Concrete syntax tree definitions for Dockerfiles.
//!
//! This crate defines the [`SyntaxKind`] enum (every kind of token and node
//! that can appear in a Dockerfile syntax tree) and the [`DockerfileLanguage`]
//! type that ties [`SyntaxKind`] into [`rowan`]'s generic tree machinery.
//!
//! The tree itself is *lossless*: every byte of the original source — including
//! whitespace, comments, and line continuations — is preserved as a token in
//! the tree. This is what enables formatters, linters, and mutation tooling to
//! round-trip a Dockerfile exactly.

mod kind;

pub use kind::SyntaxKind;

/// Marker type implementing [`rowan::Language`] for Dockerfiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DockerfileLanguage {}

impl From<SyntaxKind> for rowan::SyntaxKind {
    fn from(kind: SyntaxKind) -> Self {
        rowan::SyntaxKind(kind as u16)
    }
}

impl rowan::Language for DockerfileLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> Self::Kind {
        SyntaxKind::from_u16(raw.0).expect("invalid SyntaxKind")
    }

    fn kind_to_raw(kind: Self::Kind) -> rowan::SyntaxKind {
        rowan::SyntaxKind(kind as u16)
    }
}

/// Type aliases over [`rowan`]'s generic tree types specialized for
/// Dockerfiles.
pub type SyntaxNode = rowan::SyntaxNode<DockerfileLanguage>;
pub type SyntaxToken = rowan::SyntaxToken<DockerfileLanguage>;
pub type SyntaxElement = rowan::SyntaxElement<DockerfileLanguage>;
pub type SyntaxNodeChildren = rowan::SyntaxNodeChildren<DockerfileLanguage>;
pub type SyntaxElementChildren = rowan::SyntaxElementChildren<DockerfileLanguage>;
pub type PreorderWithTokens = rowan::api::PreorderWithTokens<DockerfileLanguage>;
