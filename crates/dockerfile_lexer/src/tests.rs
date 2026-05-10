//! Lexer tests, including snapshot-based fixtures via `insta`.

use super::*;

/// Render a token stream as a debug-friendly multi-line string. Each line is
/// `KIND@start..end "literal-text"` with newlines/tabs escaped — this format
/// is intentionally compact so insta snapshots stay readable.
fn render(src: &str, tokens: &[Token]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for tok in tokens {
        let text = tok.text(src);
        let escaped: String = text
            .chars()
            .map(|c| match c {
                '\n' => "\\n".to_string(),
                '\r' => "\\r".to_string(),
                '\t' => "\\t".to_string(),
                _ => c.to_string(),
            })
            .collect();
        let _ = writeln!(
            out,
            "{:?}@{}..{} {:?}",
            tok.kind,
            u32::from(tok.range.start()),
            u32::from(tok.range.end()),
            escaped
        );
    }
    out
}

fn snapshot(src: &str) -> String {
    render(src, &lex(src))
}

#[test]
fn empty_input() {
    assert!(lex("").is_empty());
}

#[test]
fn covers_entire_source() {
    let src = "FROM rust:1.80 AS build
RUN cargo build
";
    let tokens = lex(src);
    let total: u32 = tokens.iter().map(|t| u32::from(t.range.len())).sum();
    assert_eq!(usize::try_from(total).unwrap(), src.len());
    assert_eq!(u32::from(tokens[0].range.start()), 0);
    assert_eq!(
        usize::try_from(u32::from(tokens.last().unwrap().range.end())).unwrap(),
        src.len()
    );
}

#[test]
fn comment_only_at_line_start() {
    // The `#` after `FROM` must be part of the WORD, not a new comment.
    let src = "FROM repo#tag";
    let tokens = lex(src);
    let kinds: Vec<_> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![SyntaxKind::WORD, SyntaxKind::WHITESPACE, SyntaxKind::WORD]
    );
    assert_eq!(tokens[2].text(src), "repo#tag");
}

#[test]
fn line_continuation_emitted_as_trivia() {
    let src = r"RUN echo \
foo";
    let tokens = lex(src);
    let kinds: Vec<_> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            SyntaxKind::WORD,              // RUN
            SyntaxKind::WHITESPACE,        //
            SyntaxKind::WORD,              // echo
            SyntaxKind::WHITESPACE,        //
            SyntaxKind::LINE_CONTINUATION, // backslash + newline
            SyntaxKind::WORD,              // foo
        ]
    );
}

#[test]
fn custom_escape_char_backtick() {
    let src = "RUN echo `
foo";
    let tokens = lex_with_escape(src, '`');
    assert!(
        tokens
            .iter()
            .any(|t| t.kind == SyntaxKind::LINE_CONTINUATION)
    );
}

#[test]
fn unterminated_string_is_error() {
    let src = r#"LABEL key="unterminated"#;
    let tokens = lex(src);
    assert!(tokens.iter().any(|t| t.kind == SyntaxKind::ERROR));
}

#[test]
fn crlf_newlines_are_recognized() {
    // CRLF can't be expressed as a literal newline in source, so this case
    // keeps the explicit `\r\n` escapes.
    let src = "FROM x\r\nRUN y\r\n";
    let tokens = lex(src);
    let newlines: Vec<&Token> = tokens
        .iter()
        .filter(|t| t.kind == SyntaxKind::NEWLINE)
        .collect();
    assert_eq!(newlines.len(), 2);
    assert_eq!(newlines[0].text(src), "\r\n");
}

#[test]
fn shell_body_consumes_to_end_of_logical_line() {
    let src = r"RUN echo a \
  echo b
";
    let mut lexer = Lexer::new(src);
    // Consume `RUN`.
    let run = lexer.next_token().unwrap();
    assert_eq!(run.kind, SyntaxKind::WORD);
    // Consume the whitespace after `RUN`.
    let ws = lexer.next_token().unwrap();
    assert_eq!(ws.kind, SyntaxKind::WHITESPACE);
    // Now lex the rest of the logical line as a shell body.
    let body = lexer.lex_shell_body().unwrap();
    assert_eq!(body.kind, SyntaxKind::SHELL_BODY);
    assert_eq!(
        body.text(src),
        r"echo a \
  echo b"
    );
    // The trailing newline must remain to be lexed as a NEWLINE trivia token.
    let nl = lexer.next_token().unwrap();
    assert_eq!(nl.kind, SyntaxKind::NEWLINE);
}

#[test]
fn shell_body_returns_none_at_eol() {
    let mut lexer = Lexer::new("\n");
    assert!(lexer.lex_shell_body().is_none());
}

#[test]
fn heredoc_body_terminates_on_delimiter_line() {
    let src = "cat <<EOF
hello
world
EOF
FROM x
";
    // Skip past the heredoc marker manually to set up the cursor.
    let prefix_len = "cat <<EOF\n".len();
    let mut lexer = Lexer {
        src,
        pos: prefix_len,
        escape: b'\\',
        at_line_start: true,
    };
    let body = lexer.lex_heredoc_body("EOF", false);
    assert_eq!(body.kind, SyntaxKind::HEREDOC_BODY);
    assert_eq!(
        body.text(src),
        "hello
world
EOF
"
    );
    // After the heredoc, the next token is `FROM`.
    let next = lexer.next_token().unwrap();
    assert_eq!(next.kind, SyntaxKind::WORD);
    assert_eq!(next.text(src), "FROM");
}

#[test]
fn heredoc_dash_form_tolerates_leading_tabs() {
    let src = "cat <<-EOF
\thello
\tEOF
";
    let prefix_len = "cat <<-EOF\n".len();
    let mut lexer = Lexer {
        src,
        pos: prefix_len,
        escape: b'\\',
        at_line_start: true,
    };
    let body = lexer.lex_heredoc_body("EOF", true);
    assert_eq!(
        body.text(src),
        "\thello
\tEOF
"
    );
}

// -------- snapshot tests --------

#[test]
fn snapshot_simple_dockerfile() {
    insta::assert_snapshot!(snapshot(
        "FROM rust:1.80-alpine AS builder
WORKDIR /app
COPY . .
RUN cargo build --release
"
    ));
}

#[test]
fn snapshot_with_continuations_and_comments() {
    insta::assert_snapshot!(snapshot(
        r"# syntax=docker/dockerfile:1
FROM debian:bookworm
RUN apt-get update \
    && apt-get install -y curl \
    && rm -rf /var/lib/apt/lists/*
"
    ));
}

#[test]
fn snapshot_quoted_strings_and_env() {
    insta::assert_snapshot!(snapshot(
        r#"ENV NAME="world" GREETING='hello, world'
LABEL maintainer="ada@example.com"
"#
    ));
}
