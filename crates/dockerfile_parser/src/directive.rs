//! Pre-pass that scans the leading parser-directive block of a Dockerfile.
//!
//! According to the `BuildKit` specification, parser directives appear at the
//! very top of the file as a contiguous block of `# directive=value`
//! comments. Once *any* non-directive content is encountered (including a
//! blank line, a regular comment, or an instruction), no further directives
//! are recognized.
//!
//! The directives we care about most are:
//!
//! - `# escape=<char>` — sets the line-continuation character for the rest of
//!   the file. This must be known before tokenization, hence the pre-pass.
//! - `# syntax=<image>` — names the `BuildKit` frontend image.
//! - `# check=<settings>` — configures the `BuildKit` linter.
//!
//! Other directives are preserved verbatim but uninterpreted.

use text_size::{TextRange, TextSize};

/// A single parser directive recognized in the leading block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParserDirective {
    /// Lower-cased directive name (e.g. `"escape"`).
    pub name: String,
    /// Raw directive value (with surrounding whitespace trimmed).
    pub value: String,
    /// Byte range of the entire directive line (the `#...` text only — the
    /// trailing newline is *not* included).
    pub range: TextRange,
}

/// Output of the directive pre-pass.
#[derive(Debug, Clone, Default)]
pub struct DirectiveScan {
    pub directives: Vec<ParserDirective>,
    /// Resolved escape character. Defaults to `\` if no `# escape=` directive
    /// is present (or if the directive is malformed).
    pub escape: char,
}

impl DirectiveScan {
    /// Effective escape character for this scan.
    pub fn escape_char(&self) -> char {
        self.escape
    }
}

/// Scan parser directives from the start of `src`.
///
/// Returns every recognized directive plus the resolved escape character.
/// Whitespace before the `#`, around the `=`, and at end of line is tolerated.
pub fn scan_directives(src: &str) -> DirectiveScan {
    let mut directives = Vec::new();
    let mut escape = '\\';
    let bytes = src.as_bytes();
    let mut pos = 0;

    while pos < bytes.len() {
        let line_start = pos;
        // Optional leading horizontal whitespace.
        let mut cursor = line_start;
        while cursor < bytes.len() && matches!(bytes[cursor], b' ' | b'\t') {
            cursor += 1;
        }
        // Must start with `#`.
        if cursor >= bytes.len() || bytes[cursor] != b'#' {
            break;
        }
        // Find end of line.
        let line_end = find_newline(bytes, cursor);
        let after_hash = cursor + 1;
        // Parse `name = value` from the comment body.
        let body = &src[after_hash..line_end];
        let Some(directive) = parse_directive_body(body) else {
            break;
        };
        if directive.name == "escape"
            && let Some(ch) = directive.value.chars().next()
            && (ch == '\\' || ch == '`')
        {
            escape = ch;
        }
        directives.push(ParserDirective {
            name: directive.name,
            value: directive.value,
            range: TextRange::new(
                TextSize::try_from(line_start).expect("offset overflow"),
                TextSize::try_from(line_end).expect("offset overflow"),
            ),
        });
        // Advance past the newline.
        pos = line_end;
        if pos < bytes.len() && bytes[pos] == b'\r' {
            pos += 1;
        }
        if pos < bytes.len() && bytes[pos] == b'\n' {
            pos += 1;
        }
    }

    DirectiveScan { directives, escape }
}

fn find_newline(bytes: &[u8], from: usize) -> usize {
    let mut i = from;
    while i < bytes.len() && !matches!(bytes[i], b'\n' | b'\r') {
        i += 1;
    }
    i
}

struct ParsedDirective {
    name: String,
    value: String,
}

fn parse_directive_body(body: &str) -> Option<ParsedDirective> {
    let trimmed = body.trim();
    let (name, value) = trimmed.split_once('=')?;
    let name = name.trim();
    let value = value.trim();
    if name.is_empty() || value.is_empty() {
        return None;
    }
    if !is_valid_directive_name(name) {
        return None;
    }
    Some(ParsedDirective {
        name: name.to_ascii_lowercase(),
        value: value.to_string(),
    })
}

fn is_valid_directive_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_directives() {
        let scan = scan_directives("FROM rust\n");
        assert!(scan.directives.is_empty());
        assert_eq!(scan.escape, '\\');
    }

    #[test]
    fn single_syntax_directive() {
        let scan = scan_directives("# syntax=docker/dockerfile:1\nFROM rust\n");
        assert_eq!(scan.directives.len(), 1);
        assert_eq!(scan.directives[0].name, "syntax");
        assert_eq!(scan.directives[0].value, "docker/dockerfile:1");
    }

    #[test]
    fn escape_directive_changes_escape_char() {
        let scan = scan_directives("# escape=`\nFROM rust\n");
        assert_eq!(scan.escape, '`');
    }

    #[test]
    fn invalid_escape_value_is_ignored() {
        let scan = scan_directives("# escape=xyz\nFROM rust\n");
        assert_eq!(scan.escape, '\\');
    }

    #[test]
    fn directive_block_ends_at_first_non_directive() {
        let scan = scan_directives(
            "# syntax=docker/dockerfile:1\n# regular comment\n# escape=`\nFROM rust\n",
        );
        // The "regular comment" lacks `=`, so the block ends there.
        assert_eq!(scan.directives.len(), 1);
        assert_eq!(scan.escape, '\\');
    }

    #[test]
    fn directive_names_lowercased() {
        let scan = scan_directives("# Syntax=docker/dockerfile:1\n");
        assert_eq!(scan.directives[0].name, "syntax");
    }

    #[test]
    fn whitespace_around_equals_tolerated() {
        let scan = scan_directives("#   syntax  =  foo \n");
        assert_eq!(scan.directives.len(), 1);
        assert_eq!(scan.directives[0].name, "syntax");
        assert_eq!(scan.directives[0].value, "foo");
    }

    #[test]
    fn ranges_cover_directive_line() {
        let src = "# syntax=docker/dockerfile:1\nFROM rust\n";
        let scan = scan_directives(src);
        let r = scan.directives[0].range;
        assert_eq!(&src[r], "# syntax=docker/dockerfile:1");
    }
}
