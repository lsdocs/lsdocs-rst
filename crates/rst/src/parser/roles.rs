//! Interpreted text roles: docutils' standard roles and their English
//! names (`docutils.parsers.rst.roles`, `languages.en`), and custom roles
//! made with the `role` directive.

use super::directives::options::{Opt, OptValue, python_int};
use super::document::{At, Document, ERROR, INFO, fixed};
use super::inline::{Inliner, element};
use crate::nodes::{Element, Kind, Node, Span, Text};
use crate::util::{restore_backslashes, unescape};
use std::sync::Arc;

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

/// Whether `canonical` names a role in the registry.
fn registered(canonical: &str) -> bool {
    generic(canonical).is_some()
        || matches!(
            canonical,
            "code" | "math" | "pep-reference" | "rfc-reference" | "raw"
        )
        || UNIMPLEMENTED.contains(&canonical)
}

/// A role registered by the caller.
#[derive(Clone)]
pub(crate) struct UserRole(pub(crate) Arc<dyn crate::extension::Role>);

impl std::fmt::Debug for UserRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UserRole")
    }
}

/// A role: a standard one by canonical name, a registered one, or a
/// custom one (from the `role` directive) with the options it was made
/// with.
#[derive(Debug, Clone)]
pub(crate) struct Role {
    /// The canonical name of the base role; empty for docutils'
    /// `generic_custom_role` (an `inline` element) and registered roles.
    canonical: String,
    /// Options given by the `role` directive.
    options: Vec<(String, OptValue)>,
    user: Option<UserRole>,
}

/// What a registered role's context reads.
pub(crate) struct RoleRun<'a> {
    pub(crate) name: &'a str,
    /// The text, escapes as nulls.
    pub(crate) text: &'a str,
    pub(crate) rawsource: &'a str,
    pub(crate) options: &'a [(String, OptValue)],
    pub(crate) lineno: usize,
    pub(crate) span: Option<Span>,
}

impl RoleRun<'_> {
    pub(crate) fn options(&self) -> Vec<(String, crate::extension::OptionValue)> {
        self.options
            .iter()
            .map(|(k, v)| (k.clone(), super::directives::public_value(v)))
            .collect()
    }
}

impl Role {
    fn standard(canonical: &str) -> Self {
        Role {
            canonical: canonical.to_owned(),
            options: Vec::new(),
            user: None,
        }
    }

    /// The base of a custom role made without one.
    pub(crate) fn generic_custom() -> Self {
        Self::standard("")
    }

    /// The spec of a custom role based on this one: its options (any, for
    /// a registered role), no arguments or content.
    pub(crate) fn spec(&self) -> super::directives::Spec {
        let options: &'static [(&'static str, Opt)] = match self.canonical.as_str() {
            "code" => &[("language", Opt::Unchanged), ("class", Opt::Class)],
            "raw" => &[("format", Opt::Unchanged), ("class", Opt::Class)],
            _ => &[("class", Opt::Class)],
        };
        let mut spec = super::directives::Spec::new();
        spec.options = Some(options);
        if self.user.is_some() {
            spec.extra = super::directives::Extra::Any;
        }
        spec
    }

    /// A custom role on this base with `options` (docutils' `CustomRole`).
    pub(crate) fn customized(&self, options: Vec<(String, OptValue)>) -> Self {
        let mut merged = self.options.clone();
        for (k, v) in options {
            match merged.iter_mut().find(|(n, _)| *n == k) {
                Some(slot) => slot.1 = v,
                None => merged.push((k, v)),
            }
        }
        Role {
            canonical: self.canonical.clone(),
            options: merged,
            user: self.user.clone(),
        }
    }

    fn option(&self, name: &str) -> Option<&OptValue> {
        self.options.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// Sets the options on a role's element (`classes` from `class`).
    fn apply_options(&self, e: &mut Element) {
        for (name, value) in &self.options {
            match (name.as_str(), value) {
                ("class", v) => {
                    for c in v.as_list() {
                        e.attributes.push("classes", c);
                    }
                }
                ("language", _) => {}
                (n, OptValue::Str(s)) => e.attributes.set(n, s.clone()),
                (n, OptValue::Int(i)) => e.attributes.set(n, *i),
                _ => {}
            }
        }
    }
}

/// Finds a role by name (docutils' `roles.role`): a custom role, a
/// standard role under its English name, or one by canonical name, with
/// a message. Found roles are remembered for the document.
pub(crate) fn lookup(
    doc: &mut Document,
    name: &str,
    lineno: usize,
) -> (Option<Role>, Vec<Element>) {
    let normname = name.to_lowercase();
    if let Some(role) = doc.roles.get(&normname) {
        return (Some(role.clone()), Vec::new());
    }
    if let Some(user) = doc.extensions.roles.get(&normname).cloned() {
        let role = Role {
            canonical: String::new(),
            options: Vec::new(),
            user: Some(UserRole(user)),
        };
        doc.roles.insert(normname, role.clone());
        return (Some(role), Vec::new());
    }
    let mut messages = Vec::new();
    let canonical_name = if name.is_empty() {
        "title-reference".to_owned()
    } else if let Some(c) = canonical(&normname) {
        c.to_owned()
    } else {
        messages.push(doc.message(
            INFO,
            &format!(
                "No role entry for \"{name}\" in module \
                 \"docutils.parsers.rst.languages.en\".\n\
                 Trying \"{name}\" as canonical role name."
            ),
            Vec::new(),
            At::Top(Some(lineno)),
        ));
        normname.clone()
    };
    if registered(&canonical_name) {
        let role = Role::standard(&canonical_name);
        if !name.is_empty() {
            doc.roles.insert(normname, role.clone());
        }
        return (Some(role), messages);
    }
    (None, messages)
}

/// Interpreted text `text` with role `role_name` (empty for the default
/// role).
pub(crate) fn interpreted(
    inliner: &mut Inliner<'_>,
    rawsource: &str,
    text: &str,
    role_name: &str,
    span: Option<Span>,
) -> (Vec<Node>, Vec<Element>) {
    let lineno = inliner.lineno;
    let (role, mut messages) = lookup(inliner.doc, role_name, lineno);
    let Some(role) = role else {
        let mut msg = inliner.doc.message(
            ERROR,
            &format!("Unknown interpreted text role \"{role_name}\"."),
            Vec::new(),
            At::Top(Some(lineno)),
        );
        let prb = inliner.problematic(rawsource, &mut msg, span);
        messages.push(msg);
        return (vec![prb.into()], messages);
    };
    let (nodes, more) = apply(inliner, &role, role_name, rawsource, text, span);
    messages.extend(more);
    (nodes, messages)
}

/// Runs a role function.
fn apply(
    inliner: &mut Inliner<'_>,
    role: &Role,
    role_name: &str,
    rawsource: &str,
    text: &str,
    span: Option<Span>,
) -> (Vec<Node>, Vec<Element>) {
    let lineno = inliner.lineno;
    let error = |inliner: &mut Inliner<'_>, message: &str| -> (Vec<Node>, Vec<Element>) {
        let mut msg = inliner
            .doc
            .message(ERROR, message, Vec::new(), At::Top(Some(lineno)));
        let prb = inliner.problematic(rawsource, &mut msg, span);
        (vec![prb.into()], vec![msg])
    };
    if let Some(UserRole(user)) = &role.user {
        let mut cx = crate::extension::RoleContext {
            run: RoleRun {
                name: role_name,
                text,
                rawsource,
                options: &role.options,
                lineno,
                span,
            },
        };
        return match user.run(&mut cx) {
            Ok(nodes) => (nodes, Vec::new()),
            Err(e) => {
                let mut msg =
                    inliner
                        .doc
                        .message(e.level, &e.message, Vec::new(), At::Top(Some(lineno)));
                let prb = inliner.problematic(rawsource, &mut msg, span);
                (vec![prb.into()], vec![msg])
            }
        };
    }
    let mut node = if role.canonical.is_empty() {
        element(Kind::Inline, text, span)
    } else if let Some(kind) = generic(&role.canonical) {
        element(kind, text, span)
    } else {
        match role.canonical.as_str() {
            "code" => {
                let mut language = if role_name.eq_ignore_ascii_case("code") {
                    String::new()
                } else {
                    role_name.to_owned()
                };
                if let Some(OptValue::Str(l)) = role.option("language") {
                    language = l.clone();
                }
                if language.eq_ignore_ascii_case("none") {
                    language.clear();
                }
                let mut node = Element::new(Kind::Literal);
                node.span = span;
                node.attributes.push("classes", "code");
                role.apply_options(&mut node);
                if !language.is_empty() && !node.attributes.get_list("classes").contains(&language)
                {
                    node.attributes.push("classes", language);
                }
                node.children
                    .push(Text::new(restore_backslashes(text), span).into());
                return (vec![node.into()], Vec::new());
            }
            "math" => element(Kind::Math, &restore_backslashes(text), span),
            "pep-reference" => {
                let number = python_int(Some(&unescape(text)))
                    .ok()
                    .filter(|n| (0..=9999).contains(n));
                let Some(n) = number else {
                    return error(
                        inliner,
                        &format!(
                            "PEP number must be a number from 0 to 9999; \"{text}\" is invalid."
                        ),
                    );
                };
                let mut r = element(Kind::Reference, &format!("PEP {text}"), span);
                r.attributes
                    .set("refuri", format!("https://peps.python.org/pep-{n:04}"));
                r
            }
            "rfc-reference" => {
                let plain = unescape(text);
                let (number, section) = match plain.split_once('#') {
                    Some((n, s)) => (n.to_owned(), Some(s.to_owned())),
                    None => (plain.clone(), None),
                };
                let number = python_int(Some(&number)).ok().filter(|&n| n >= 1);
                let Some(n) = number else {
                    return error(
                        inliner,
                        &format!(
                            "RFC number must be a number greater than or equal to 1; \
                             \"{text}\" is invalid."
                        ),
                    );
                };
                let mut uri = format!("https://tools.ietf.org/html/rfc{n}.html");
                if let Some(section) = section {
                    uri.push('#');
                    uri.push_str(&section);
                }
                let mut r = element(Kind::Reference, &format!("RFC {n}"), span);
                r.attributes.set("refuri", uri);
                r
            }
            "raw" => {
                if role.option("format").is_none() {
                    return error(
                        inliner,
                        &format!(
                            "No format (Writer name) is associated with this role: \"{role_name}\".\n\
                             The \"raw\" role cannot be used directly.\n\
                             Instead, use the \"role\" directive to create a new role with an \
                             associated format."
                        ),
                    );
                }
                let mut node = fixed(Kind::Raw, &restore_backslashes(text), span);
                role.apply_options(&mut node);
                return (vec![node.into()], Vec::new());
            }
            _ => {
                return error(
                    inliner,
                    &format!("Interpreted text role \"{role_name}\" not implemented."),
                );
            }
        }
    };
    role.apply_options(&mut node);
    (vec![node.into()], Vec::new())
}
