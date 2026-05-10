//! Tests for the typed AST wrappers.

#![allow(clippy::manual_let_else)]

use crate::{AstNode, CommandForm, Dockerfile, Stmt};
use dockerfile_parser::parse;

fn dockerfile(src: &str) -> Dockerfile {
    let parse = parse(src);
    Dockerfile::cast(parse.syntax()).expect("DOCKERFILE root")
}

#[test]
fn iterates_statements_in_order() {
    let df = dockerfile(
        "
FROM rust
RUN cargo build
WORKDIR /app
",
    );
    let kinds: Vec<&'static str> = df
        .statements()
        .map(|s| match s {
            Stmt::From(_) => "from",
            Stmt::Run(_) => "run",
            Stmt::Workdir(_) => "workdir",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, vec!["from", "run", "workdir"]);
}

#[test]
fn from_image_ref_and_stage_name() {
    let df = dockerfile("FROM rust:1.80-alpine AS builder\n");
    let from = match df.statements().next().unwrap() {
        Stmt::From(f) => f,
        _ => unreachable!(),
    };
    let img = from.image_ref().unwrap();
    assert_eq!(img.full(), "rust:1.80-alpine");
    let parts = img.parts();
    assert_eq!(parts.name, "rust");
    assert_eq!(parts.tag.as_deref(), Some("1.80-alpine"));
    assert_eq!(parts.digest, None);
    assert_eq!(from.stage_name().as_deref(), Some("builder"));
}

#[test]
fn image_ref_with_digest() {
    let df = dockerfile("FROM rust@sha256:abc123\n");
    let from = match df.statements().next().unwrap() {
        Stmt::From(f) => f,
        _ => unreachable!(),
    };
    let parts = from.image_ref().unwrap().parts();
    assert_eq!(parts.name, "rust");
    assert_eq!(parts.tag, None);
    assert_eq!(parts.digest.as_deref(), Some("sha256:abc123"));
}

#[test]
fn image_ref_with_registry_port_does_not_confuse_tag_split() {
    let df = dockerfile("FROM registry.example.com:5000/myimage\n");
    let from = match df.statements().next().unwrap() {
        Stmt::From(f) => f,
        _ => unreachable!(),
    };
    let parts = from.image_ref().unwrap().parts();
    assert_eq!(parts.name, "registry.example.com:5000/myimage");
    assert_eq!(parts.tag, None);
}

#[test]
fn run_shell_form_body() {
    let df = dockerfile("RUN echo hello && echo world\n");
    let run = match df.statements().next().unwrap() {
        Stmt::Run(r) => r,
        _ => unreachable!(),
    };
    assert!(run.is_shell_form());
    match run.body() {
        CommandForm::Shell(s) => assert_eq!(s, "echo hello && echo world"),
        other => panic!("expected shell form, got {other:?}"),
    }
}

#[test]
fn run_exec_form_body() {
    let df = dockerfile(
        r#"RUN ["echo", "hello, world"]
"#,
    );
    let run = match df.statements().next().unwrap() {
        Stmt::Run(r) => r,
        _ => unreachable!(),
    };
    assert!(run.is_exec_form());
    match run.body() {
        CommandForm::Exec(args) => assert_eq!(args, vec!["echo", "hello, world"]),
        other => panic!("expected exec form, got {other:?}"),
    }
}

#[test]
fn env_pairs_split_by_kv() {
    let df = dockerfile(
        r#"ENV A=1 B="two" C=three
"#,
    );
    let env = match df.statements().next().unwrap() {
        Stmt::Env(e) => e,
        _ => unreachable!(),
    };
    let pairs: Vec<(String, Option<String>)> =
        env.pairs().map(|p| (p.key().unwrap(), p.value())).collect();
    assert_eq!(
        pairs,
        vec![
            ("A".to_string(), Some("1".to_string())),
            ("B".to_string(), Some("two".to_string())),
            ("C".to_string(), Some("three".to_string())),
        ]
    );
}

#[test]
fn label_pairs_with_quoted_values() {
    let df = dockerfile(
        r#"LABEL maintainer="ada@example.com" version="1.0"
"#,
    );
    let label = match df.statements().next().unwrap() {
        Stmt::Label(l) => l,
        _ => unreachable!(),
    };
    let pairs: Vec<_> = label
        .pairs()
        .map(|p| (p.key().unwrap(), p.value().unwrap()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("maintainer".to_string(), "ada@example.com".to_string()),
            ("version".to_string(), "1.0".to_string()),
        ]
    );
}

#[test]
fn arg_with_default() {
    let df = dockerfile("ARG VERSION=1.0\n");
    let arg = match df.statements().next().unwrap() {
        Stmt::Arg(a) => a,
        _ => unreachable!(),
    };
    assert_eq!(arg.name().as_deref(), Some("VERSION"));
    assert_eq!(arg.default_value().as_deref(), Some("1.0"));
}

#[test]
fn arg_without_default() {
    let df = dockerfile("ARG VERSION\n");
    let arg = match df.statements().next().unwrap() {
        Stmt::Arg(a) => a,
        _ => unreachable!(),
    };
    assert_eq!(arg.name().as_deref(), Some("VERSION"));
    assert_eq!(arg.default_value(), None);
}

#[test]
fn copy_sources_and_dest() {
    let df = dockerfile("COPY --from=builder src1 src2 /dest\n");
    let copy = match df.statements().next().unwrap() {
        Stmt::Copy(c) => c,
        _ => unreachable!(),
    };
    assert_eq!(copy.from().as_deref(), Some("builder"));
    assert_eq!(copy.sources(), vec!["src1".to_string(), "src2".to_string()]);
    assert_eq!(copy.dest().as_deref(), Some("/dest"));
}

#[test]
fn workdir_path() {
    let df = dockerfile("WORKDIR /app\n");
    let wd = match df.statements().next().unwrap() {
        Stmt::Workdir(w) => w,
        _ => unreachable!(),
    };
    assert_eq!(wd.path().as_deref(), Some("/app"));
}

#[test]
fn expose_ports() {
    let df = dockerfile("EXPOSE 80 443/tcp 53/udp\n");
    let exp = match df.statements().next().unwrap() {
        Stmt::Expose(e) => e,
        _ => unreachable!(),
    };
    assert_eq!(
        exp.ports(),
        vec![
            "80".to_string(),
            "443/tcp".to_string(),
            "53/udp".to_string()
        ]
    );
}

#[test]
fn parser_directive_extraction() {
    let df = dockerfile(
        "# syntax=docker/dockerfile:1
# escape=`
FROM rust
",
    );
    let directives: Vec<_> = df
        .parser_directives()
        .map(|d| d.name_and_value().unwrap())
        .collect();
    assert_eq!(
        directives,
        vec![
            ("syntax".to_string(), "docker/dockerfile:1".to_string()),
            ("escape".to_string(), "`".to_string()),
        ]
    );
}

#[test]
fn flag_value_extraction() {
    let df = dockerfile("RUN --mount=type=cache,target=/root/.cargo cargo build\n");
    let run = match df.statements().next().unwrap() {
        Stmt::Run(r) => r,
        _ => unreachable!(),
    };
    let flag = run.flags().next().expect("RUN has a flag");
    assert_eq!(flag.name().as_deref(), Some("mount"));
    assert_eq!(
        flag.value().as_deref(),
        Some("type=cache,target=/root/.cargo")
    );
}
