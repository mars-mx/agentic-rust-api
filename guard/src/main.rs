use std::io::Write;
use std::process::ExitCode;

use ra_ap_rustc_lexer::{FrontmatterAllowed, TokenKind, strip_shebang, tokenize};

#[derive(Debug, PartialEq, Eq)]
enum Finding {
    Comment,
    DocComment,
    DocAttribute,
    LintSuppression,
}

impl Finding {
    fn label(&self) -> &'static str {
        match self {
            Finding::Comment => "comment",
            Finding::DocComment => "doc comment",
            Finding::DocAttribute => "#[doc] attribute",
            Finding::LintSuppression => "lint suppression",
        }
    }
}

fn findings(source: &str) -> Option<Vec<(usize, Finding)>> {
    let start = strip_shebang(source).unwrap_or(0);
    let body = source.get(start..)?;
    let mut tokens = Vec::new();
    let mut offset = start;
    for token in tokenize(body, FrontmatterAllowed::No) {
        let end = offset + usize::try_from(token.len).ok()?;
        if token.kind != TokenKind::Whitespace {
            tokens.push((offset, token.kind, source.get(offset..end)?));
        }
        offset = end;
    }

    let line_of = |at: usize| {
        source
            .bytes()
            .take(at)
            .filter(|byte| *byte == b'\n')
            .count()
            + 1
    };
    let mut found = Vec::new();
    for (index, (at, kind, _)) in tokens.iter().enumerate() {
        match kind {
            TokenKind::LineComment { doc_style } | TokenKind::BlockComment { doc_style, .. } => {
                let finding = if doc_style.is_some() {
                    Finding::DocComment
                } else {
                    Finding::Comment
                };
                found.push((line_of(*at), finding));
            }
            TokenKind::Pound => {
                if let Some(finding) = attribute_finding(&tokens[index + 1..]) {
                    found.push((line_of(*at), finding));
                }
            }
            _ => {}
        }
    }
    Some(found)
}

fn attribute_finding(rest: &[(usize, TokenKind, &str)]) -> Option<Finding> {
    let rest = match rest.first() {
        Some((_, TokenKind::Bang, _)) => &rest[1..],
        _ => rest,
    };
    let [
        (_, TokenKind::OpenBracket, _),
        (_, TokenKind::Ident, name),
        body @ ..,
    ] = rest
    else {
        return None;
    };
    if *name != "cfg_attr" {
        return finding_for(name);
    }
    let mut depth = 1usize;
    for (_, kind, text) in body {
        match kind {
            TokenKind::OpenBracket => depth += 1,
            TokenKind::CloseBracket if depth == 1 => return None,
            TokenKind::CloseBracket => depth -= 1,
            TokenKind::Ident => {
                if let Some(finding) = finding_for(text) {
                    return Some(finding);
                }
            }
            _ => {}
        }
    }
    None
}

fn finding_for(name: &str) -> Option<Finding> {
    match name {
        "doc" => Some(Finding::DocAttribute),
        "allow" | "expect" | "warn" => Some(Finding::LintSuppression),
        _ => None,
    }
}

fn check(paths: impl Iterator<Item = String>, out: &mut impl Write) -> Result<bool, String> {
    let mut clean = true;
    for path in paths {
        let source = std::fs::read_to_string(&path)
            .map_err(|error| format!("{path}: cannot read: {error}"))?;
        let found = findings(&source).ok_or_else(|| format!("{path}: cannot tokenize"))?;
        for (line, finding) in found {
            clean = false;
            writeln!(out, "{path}:{line}: {}", finding.label())
                .map_err(|error| format!("cannot write the report: {error}"))?;
        }
    }
    Ok(clean)
}

fn main() -> ExitCode {
    match check(std::env::args().skip(1), &mut std::io::stdout().lock()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(message) => {
            let reported = writeln!(std::io::stderr().lock(), "{message}");
            ExitCode::from(if reported.is_ok() { 2 } else { 3 })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Finding;

    fn findings(source: &str) -> Vec<(usize, Finding)> {
        super::findings(source).unwrap()
    }

    #[test]
    fn plain_code_is_clean() {
        let source = "fn main() {\n    let url = \"https://example.com/a//b\";\n    let c = '/';\n    let s = r#\"/* not */ // either\"#;\n    let b = b\"//\";\n}\n";
        assert_eq!(findings(source), vec![]);
    }

    #[test]
    fn line_and_block_comments_are_found() {
        let source = "fn a() {}\n// line\nfn b() { /* block /* nested */ still */ }\n";
        assert_eq!(
            findings(source),
            vec![(2, Finding::Comment), (3, Finding::Comment)]
        );
    }

    #[test]
    fn doc_comments_are_found() {
        let source = "//! crate docs\n/// item docs\n/** block docs */\nfn a() {}\n";
        assert_eq!(
            findings(source),
            vec![
                (1, Finding::DocComment),
                (2, Finding::DocComment),
                (3, Finding::DocComment)
            ]
        );
    }

    #[test]
    fn doc_attributes_are_found() {
        let source = "#![doc = \"crate\"]\n#[doc = \"item\"]\n# [ doc = \"spaced\" ]\n#[derive(Debug)]\nstruct A;\n";
        assert_eq!(
            findings(source),
            vec![
                (1, Finding::DocAttribute),
                (2, Finding::DocAttribute),
                (3, Finding::DocAttribute)
            ]
        );
    }

    #[test]
    fn lint_suppressions_are_found() {
        let source = "#![allow(dead_code)]\n#[expect(clippy::unwrap_used, reason = \"x\")]\nfn a() {}\n#[warn(missing_docs)]\nfn b() {}\n#[cfg_attr(test, allow(unused))]\nfn c() {}\n#[cfg_attr(docsrs, doc = \"x\")]\nfn d() {}\n";
        assert_eq!(
            findings(source),
            vec![
                (1, Finding::LintSuppression),
                (2, Finding::LintSuppression),
                (4, Finding::LintSuppression),
                (6, Finding::LintSuppression),
                (8, Finding::DocAttribute)
            ]
        );
    }

    #[test]
    fn ordinary_attributes_are_clean() {
        let source = "#[derive(Debug)]\n#[serde(default)]\n#[cfg_attr(test, derive(Clone))]\n#[cfg(test)]\nstruct A;\n";
        assert_eq!(findings(source), vec![]);
    }

    #[test]
    fn shebang_is_not_a_comment() {
        assert_eq!(
            findings("#!/usr/bin/env run-cargo-script\nfn main() {}\n"),
            vec![]
        );
    }

    #[test]
    fn lifetimes_do_not_confuse_the_lexer() {
        assert_eq!(findings("fn a<'b>(c: &'b str) -> &'b str { c }\n"), vec![]);
    }
}
