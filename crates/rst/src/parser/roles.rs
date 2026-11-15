//! Interpreted text roles: docutils' standard roles and their English
//! names (`docutils.parsers.rst.roles`, `languages.en`).
//!
//! Roles with options (`code` with a language, `raw` with a format) come
//! from the `role` directive, which arrives with the directive registry;
//! used directly they behave as in docutils.

use super::document::{ERROR, INFO};
use super::inline::{Inliner, element};
use crate::nodes::{Element, Kind, Node, Span, Text};
use crate::util::{restore_backslashes, unescape};

/// The canonical role for an English role name.
fn canonical(name: &str) -> Option<&'static str> {
    Some(match name {
        "abbreviation" | "ab" => "abbreviation",
        "acronym" | "ac" => "acronym",
        "code" => "code",
        "emphasis" => "emphasis",
        "literal" => "literal",
        "math" => "math",
        "pep-reference" | "pep" => "pep-reference",
        "rfc-reference" | "rfc" => "rfc-reference",
        "strong" => "strong",
        "subscript" | "sub" => "subscript",
        "superscript" | "sup" => "superscript",
        "title-reference" | "title" | "t" => "title-reference",
        "raw" => "raw",
        "index" | "i" => "index",
        "anonymous-reference" => "anonymous-reference",
        "citation-reference" => "citation-reference",
        "footnote-reference" => "footnote-reference",
        "named-reference" => "named-reference",
        "substitution-reference" => "substitution-reference",
        "uri-reference" | "uri" | "url" => "uri-reference",
        "target" => "target",
        _ => return None,
    })
}

/// Canonical roles that exist but are not implemented.
const UNIMPLEMENTED: &[&str] = &[
    "index",
    "named-reference",
    "anonymous-reference",
    "uri-reference",
    "footnote-reference",
    "citation-reference",
    "substitution-reference",
    "target",
    "restructuredtext-unimplemented-role",
];

/// The element kind of a role that wraps its text.
fn generic(canonical: &str) -> Option<Kind> {
    Some(match canonical {
        "abbreviation" => Kind::Abbreviation,
        "acronym" => Kind::Acronym,
        "emphasis" => Kind::Emphasis,
        "literal" => Kind::Literal,
        "strong" => Kind::Strong,
        "subscript" => Kind::Subscript,
        "superscript" => Kind::Superscript,
        "title-reference" => Kind::TitleReference,
        _ => return None,
    })
}

/// Parses an integer the way Python's `int()` does for plain decimal text.
fn python_int(text: &str) -> Option<i64> {
    let t = text.trim();
    let digits = t.strip_prefix(['+', '-']).unwrap_or(t);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit() || c == '_') {
        return None;
    }
    if digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") {
        return None;
    }
    t.replace('_', "").parse().ok()
}

/// Interpreted text `text` with `role` (empty for the default role).
pub(crate) fn interpreted(
    inliner: &mut Inliner<'_>,
    rawsource: &str,
    text: &str,
    role: &str,
    span: Option<Span>,
) -> (Vec<Node>, Vec<Element>) {
    let mut messages = Vec::new();
    let normname = role.to_lowercase();
    let canonical_name = if role.is_empty() {
        "title-reference".to_owned()
    } else if let Some(c) = canonical(&normname) {
        c.to_owned()
    } else {
        messages.push(inliner.doc.message(
            INFO,
            &format!(
                "No role entry for \"{role}\" in module \
                 \"docutils.parsers.rst.languages.en\".\n\
                 Trying \"{role}\" as canonical role name."
            ),
            Vec::new(),
            super::document::At::Top(Some(inliner.lineno)),
        ));
        normname
    };
    let error =
        |inliner: &mut Inliner<'_>, level: i64, message: &str| -> (Vec<Node>, Vec<Element>) {
            let mut msg = inliner.doc.message(
                level,
                message,
                Vec::new(),
                super::document::At::Top(Some(inliner.lineno)),
            );
            let prb = inliner.problematic(rawsource, &mut msg, span);
            (vec![prb.into()], vec![msg])
        };
    let (nodes, more) = if let Some(kind) = generic(&canonical_name) {
        (vec![element(kind, text, span).into()], Vec::new())
    } else {
        match canonical_name.as_str() {
            "code" => {
                let mut node = Element::new(Kind::Literal);
                node.span = span;
                node.attributes.push("classes", "code");
                node.children
                    .push(Text::new(restore_backslashes(text), span).into());
                (vec![node.into()], Vec::new())
            }
            "math" => (
                vec![element(Kind::Math, &restore_backslashes(text), span).into()],
                Vec::new(),
            ),
            "pep-reference" => match python_int(&unescape(text)).filter(|n| (0..=9999).contains(n))
            {
                Some(n) => {
                    let mut r = element(Kind::Reference, &format!("PEP {text}"), span);
                    r.attributes
                        .set("refuri", format!("https://peps.python.org/pep-{n:04}"));
                    (vec![r.into()], Vec::new())
                }
                None => error(
                    inliner,
                    ERROR,
                    &format!("PEP number must be a number from 0 to 9999; \"{text}\" is invalid."),
                ),
            },
            "rfc-reference" => {
                let plain = unescape(text);
                let (number, section) = match plain.split_once('#') {
                    Some((n, s)) => (n.to_owned(), Some(s.to_owned())),
                    None => (plain.clone(), None),
                };
                match python_int(&number).filter(|&n| n >= 1) {
                    Some(n) => {
                        let mut uri = format!("https://tools.ietf.org/html/rfc{n}.html");
                        if let Some(section) = section {
                            uri.push('#');
                            uri.push_str(&section);
                        }
                        let mut r = element(Kind::Reference, &format!("RFC {n}"), span);
                        r.attributes.set("refuri", uri);
                        (vec![r.into()], Vec::new())
                    }
                    None => error(
                        inliner,
                        ERROR,
                        &format!(
                            "RFC number must be a number greater than or equal to 1; \
                             \"{text}\" is invalid."
                        ),
                    ),
                }
            }
            "raw" => error(
                inliner,
                ERROR,
                &format!(
                    "No format (Writer name) is associated with this role: \"{role}\".\n\
                     The \"raw\" role cannot be used directly.\n\
                     Instead, use the \"role\" directive to create a new role with an \
                     associated format."
                ),
            ),
            c if UNIMPLEMENTED.contains(&c) => error(
                inliner,
                ERROR,
                &format!("Interpreted text role \"{role}\" not implemented."),
            ),
            _ => {
                let mut msg = inliner.doc.message(
                    ERROR,
                    &format!("Unknown interpreted text role \"{role}\"."),
                    Vec::new(),
                    super::document::At::Top(Some(inliner.lineno)),
                );
                let prb = inliner.problematic(rawsource, &mut msg, span);
                messages.push(msg);
                return (vec![prb.into()], messages);
            }
        }
    };
    messages.extend(more);
    (nodes, messages)
}
