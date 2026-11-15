//! The reStructuredText parser: a port of docutils' state machine
//! (`docutils.parsers.rst.states`).
//!
//! Supported so far: the block structure (sections, transitions,
//! paragraphs, block quotes, bullet, enumerated, definition, field and
//! option lists, literal, line and doctest blocks, footnotes, citations,
//! hyperlink targets, substitution definitions, comments and tables),
//! inline markup (docutils' `Inliner`, with the standard roles), and the
//! standard directives, plus directives and roles the caller registers.

mod body;
mod directives;
mod document;
mod explicit;
mod inline;
mod machine;
mod roles;
mod table;
mod text;
mod transforms;

use std::collections::HashMap;
use std::sync::Arc;

pub(crate) use directives::DirectiveRun;
pub(crate) use roles::RoleRun;

use crate::extension::{Directive, Role};
use crate::lines::lines;
use crate::nodes::{Element, Kind, Span};

/// Directives and roles the caller added, and settings for them.
#[derive(Clone, Default)]
pub(crate) struct Extensions {
    /// By lower-case name.
    pub(crate) directives: HashMap<String, Arc<dyn Directive>>,
    /// By lower-case name.
    pub(crate) roles: HashMap<String, Arc<dyn Role>>,
    /// Keep unknown directives as generic nodes instead of reporting them.
    pub(crate) keep_unknown: bool,
    /// Run docutils' transforms after parsing.
    pub(crate) transforms: bool,
}

/// The longest line docutils parses (its `line_length_limit` setting).
const LINE_LENGTH_LIMIT: usize = 10_000;

/// Parses reStructuredText. Settings such as the source name are set on the
/// parser; [`crate::parse`] uses the defaults.
#[derive(Clone, Default)]
pub struct Parser {
    source: Option<String>,
    extensions: Arc<Extensions>,
}

impl std::fmt::Debug for Parser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut directives: Vec<&String> = self.extensions.directives.keys().collect();
        let mut roles: Vec<&String> = self.extensions.roles.keys().collect();
        directives.sort();
        roles.sort();
        f.debug_struct("Parser")
            .field("source", &self.source)
            .field("directives", &directives)
            .field("roles", &roles)
            .field("keep_unknown_directives", &self.extensions.keep_unknown)
            .finish()
    }
}

impl Parser {
    /// A parser with the default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Names the input; the document's `source` attribute, used in
    /// messages.
    pub fn source(mut self, name: impl Into<String>) -> Self {
        self.source = Some(name.into());
        self
    }

    /// Adds a directive under `name` (case-insensitive). It takes
    /// precedence over a standard directive of the same name.
    pub fn directive(mut self, name: &str, directive: impl Directive + 'static) -> Self {
        Arc::make_mut(&mut self.extensions)
            .directives
            .insert(name.to_lowercase(), Arc::new(directive));
        self
    }

    /// Adds an interpreted text role under `name` (case-insensitive). It
    /// takes precedence over a standard role of the same name.
    pub fn role(mut self, name: &str, role: impl Role + 'static) -> Self {
        Arc::make_mut(&mut self.extensions)
            .roles
            .insert(name.to_lowercase(), Arc::new(role));
        self
    }

    /// With `keep`, a directive without an implementation becomes an
    /// element of kind `directive` (its `name` and `argument` as
    /// attributes, its options as a field list, its content as a literal
    /// block) for the caller to handle. Without (the default), it is
    /// reported as docutils reports it: an INFO and an ERROR message.
    pub fn keep_unknown_directives(mut self, keep: bool) -> Self {
        Arc::make_mut(&mut self.extensions).keep_unknown = keep;
        self
    }

    /// With `transforms`, the tree is also transformed the way docutils'
    /// `publish_doctree` does it: references resolved, footnotes numbered,
    /// substitutions replaced, `pending` elements processed. Without (the
    /// default), it is the parser's tree, as docutils' parser returns it.
    pub fn transforms(mut self, transforms: bool) -> Self {
        Arc::make_mut(&mut self.extensions).transforms = transforms;
        self
    }

    /// Parses `text` into a document: an element of kind
    /// [`Kind::Document`]. Problems are reported as `system_message`
    /// elements in the tree; parsing itself never fails.
    pub fn parse(&self, text: &str) -> Element {
        let mut root = Element::new(Kind::Document).with_span(Span::new(0, text.len()));
        if let Some(source) = &self.source {
            root.attributes.set("source", source.as_str());
        }
        let lines = lines(text);
        let mut doc = document::Document::new(root, self.source.clone(), lines.len());
        doc.extensions = self.extensions.clone();
        // docutils refuses to parse input with very long lines.
        if let Some(i) = lines
            .iter()
            .position(|l| l.text.chars().count() > LINE_LENGTH_LIMIT)
        {
            let msg = doc.message(
                document::ERROR,
                &format!("Line {} exceeds the line-length-limit.", i + 1),
                Vec::new(),
                document::At::Exact(None),
            );
            doc.append(msg);
        } else {
            let mut top = machine::Machine::top(lines);
            top.run(&mut doc, machine::State::Body);
        }
        let mut root = doc.finish(self.extensions.transforms);
        root.span = Some(Span::new(0, text.len()));
        root
    }
}
