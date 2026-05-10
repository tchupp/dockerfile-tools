//! Parser tests.

use std::fmt::Write as _;

use crate::{assert_round_trips, parse};

fn snapshot(src: &str) -> String {
    let parse = parse(src);
    let mut out = parse.debug_tree();
    if !parse.diagnostics().is_empty() {
        out.push_str("\n\n--- diagnostics ---\n");
        for diag in parse.diagnostics() {
            writeln!(
                out,
                "{:?} @{}..{}: {}",
                diag.severity,
                u32::from(diag.range.start()),
                u32::from(diag.range.end()),
                diag.message,
            )
            .unwrap();
        }
    }
    out
}

#[test]
fn empty_input() {
    let parse = parse("");
    assert!(parse.diagnostics().is_empty());
    assert_round_trips("");
}

#[test]
fn round_trip_preserves_source_exactly() {
    let cases: &[&str] = &[
        "",
        "FROM rust\n",
        "FROM rust:1.80-alpine AS builder
RUN cargo build
",
        "# syntax=docker/dockerfile:1
# escape=`
FROM debian
RUN apt-get update `
  && apt-get install -y curl
",
        r#"ENV A=1 B=2 C="three"
"#,
        "COPY --from=builder /app/target/release/x /usr/local/bin/x
",
        r#"RUN ["echo", "hello"]
"#,
        r#"LABEL maintainer="ada@example.com"
LABEL desc="a test"
"#,
        // Trailing blank lines and missing final newline.
        "FROM rust


",
        "FROM rust",
    ];
    for src in cases {
        assert_round_trips(src);
    }
}

#[test]
fn unknown_instruction_warns_but_recovers() {
    let src = "FOOBAR something
FROM rust
";
    let parse = parse(src);
    assert!(parse.diagnostics().iter().any(|d| d.code == Some("DF001")));
    assert_round_trips(src);
}

#[test]
fn lower_case_instructions_recognized() {
    let parse = parse(
        "from rust:1
run echo hi
",
    );
    assert!(parse.diagnostics().is_empty());
    let kinds: Vec<_> = parse.syntax().children().map(|c| c.kind()).collect();
    assert!(kinds.contains(&dockerfile_syntax::SyntaxKind::FROM_INSTR));
    assert!(kinds.contains(&dockerfile_syntax::SyntaxKind::RUN_INSTR));
}

#[test]
fn from_with_as_clause_produces_stage_name() {
    let parse = parse("FROM rust:1.80 AS builder\n");
    let from = parse
        .syntax()
        .children()
        .find(|c| c.kind() == dockerfile_syntax::SyntaxKind::FROM_INSTR)
        .expect("FROM_INSTR present");
    let stage = from
        .children()
        .find(|c| c.kind() == dockerfile_syntax::SyntaxKind::STAGE_NAME)
        .expect("STAGE_NAME child");
    assert_eq!(stage.text().to_string(), "builder");
}

#[test]
fn from_with_platform_flag() {
    let parse = parse("FROM --platform=linux/amd64 rust:1.80\n");
    assert!(parse.diagnostics().is_empty());
    let from = parse
        .syntax()
        .children()
        .find(|c| c.kind() == dockerfile_syntax::SyntaxKind::FROM_INSTR)
        .unwrap();
    let flag = from
        .children()
        .find(|c| c.kind() == dockerfile_syntax::SyntaxKind::FLAG)
        .expect("FLAG child");
    assert!(flag.text().to_string().starts_with("--platform"));
}

#[test]
fn run_exec_form_wrapped_in_exec_form_node() {
    let parse = parse(
        r#"
RUN ["echo", "hello"]
"#,
    );
    let run = parse
        .syntax()
        .children()
        .find(|c| c.kind() == dockerfile_syntax::SyntaxKind::RUN_INSTR)
        .unwrap();
    assert!(
        run.children()
            .any(|c| c.kind() == dockerfile_syntax::SyntaxKind::EXEC_FORM)
    );
}

#[test]
fn run_shell_form_wrapped_in_shell_form_node() {
    let parse = parse("RUN echo hello && echo world\n");
    let run = parse
        .syntax()
        .children()
        .find(|c| c.kind() == dockerfile_syntax::SyntaxKind::RUN_INSTR)
        .unwrap();
    assert!(
        run.children()
            .any(|c| c.kind() == dockerfile_syntax::SyntaxKind::SHELL_FORM)
    );
}

#[test]
fn env_emits_key_value_node() {
    let parse = parse("ENV PATH=/usr/local/bin\n");
    let env = parse
        .syntax()
        .children()
        .find(|c| c.kind() == dockerfile_syntax::SyntaxKind::ENV_INSTR)
        .unwrap();
    let kv = env
        .children()
        .find(|c| c.kind() == dockerfile_syntax::SyntaxKind::KEY_VALUE)
        .expect("KEY_VALUE child");
    assert_eq!(kv.text().to_string(), "PATH=/usr/local/bin");
}

#[test]
fn parser_directive_node_emitted() {
    let parse = parse(
        "# syntax=docker/dockerfile:1
FROM rust
",
    );
    let directive = parse
        .syntax()
        .children()
        .find(|c| c.kind() == dockerfile_syntax::SyntaxKind::PARSER_DIRECTIVE)
        .expect("PARSER_DIRECTIVE child");
    assert_eq!(directive.text().to_string(), "# syntax=docker/dockerfile:1");
}

#[test]
fn escape_directive_propagates_to_lexer() {
    // With `escape=\``, the backtick before the newline is the line
    // continuation, not a literal character. The body should round-trip.
    let src = "# escape=`
RUN echo a `
  echo b
";
    assert_round_trips(src);
}

#[test]
fn label_with_quoted_value() {
    let src = r#"LABEL maintainer="ada@example.com"
"#;
    let parse = parse(src);
    assert!(parse.diagnostics().is_empty());
    assert_round_trips(src);
}

#[test]
fn copy_with_multiple_flags() {
    let src = "COPY --from=builder --chown=root:root /a /b\n";
    let parse = parse(src);
    assert!(parse.diagnostics().is_empty());
    let copy = parse
        .syntax()
        .children()
        .find(|c| c.kind() == dockerfile_syntax::SyntaxKind::COPY_INSTR)
        .unwrap();
    let flag_count = copy
        .children()
        .filter(|c| c.kind() == dockerfile_syntax::SyntaxKind::FLAG)
        .count();
    assert_eq!(flag_count, 2);
}

#[test]
fn line_continuations_keep_one_logical_line() {
    let src = r"RUN apt-get update \
    && apt-get install -y curl
";
    let parse = parse(src);
    assert!(parse.diagnostics().is_empty());
    let run_count = parse
        .syntax()
        .children()
        .filter(|c| c.kind() == dockerfile_syntax::SyntaxKind::RUN_INSTR)
        .count();
    assert_eq!(
        run_count, 1,
        "continuation must not start a new instruction"
    );
    assert_round_trips(src);
}

// -------- snapshot tests --------

#[test]
fn snapshot_minimal_dockerfile() {
    insta::assert_snapshot!(snapshot("FROM rust:1.80\n"));
}

#[test]
fn snapshot_multistage_with_directives() {
    insta::assert_snapshot!(snapshot(
        r#"# syntax=docker/dockerfile:1
# escape=\
FROM rust:1.80-alpine AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/myapp /usr/local/bin/myapp
ENTRYPOINT ["/usr/local/bin/myapp"]
"#
    ));
}

#[test]
fn snapshot_env_label_arg() {
    insta::assert_snapshot!(snapshot(
        r#"ARG VERSION=1.0
ENV PATH=/bin:/usr/bin LANG=en_US.UTF-8
LABEL maintainer="ada@example.com" version=$VERSION
"#
    ));
}

#[test]
fn snapshot_run_with_continuations() {
    insta::assert_snapshot!(snapshot(
        r"RUN apt-get update \
    && apt-get install -y curl \
    && rm -rf /var/lib/apt/lists/*
"
    ));
}

#[test]
fn snapshot_unknown_instruction_diagnostic() {
    insta::assert_snapshot!(snapshot(
        "FOOBAR what
FROM rust
"
    ));
}
