//! The reStructuredText parser: a port of docutils' state machine
//! (`docutils.parsers.rst.states`).
//!
//! Supported so far: the block structure (sections, transitions,
//! paragraphs, block quotes, bullet, enumerated, definition, field and
//! option lists, literal, line and doctest blocks, footnotes, citations,
//! hyperlink targets, substitution definitions, comments and tables),
//! inline markup (docutils' `Inliner`, with the standard roles), and the
//! standard directives except the table directives.

mod body;
mod directives;
mod document;
mod explicit;
mod inline;
mod machine;
mod roles;
mod table;
mod text;

use crate::lines::lines;
use crate::nodes::{Element, Kind, Span};

/// The longest line docutils parses (its `line_length_limit` setting).
const LINE_LENGTH_LIMIT: usize = 10_000;

/// Parses reStructuredText. Settings such as the source name are set on the
/// parser; [`crate::parse`] uses the defaults.
#[derive(Debug, Clone, Default)]
pub struct Parser {
    source: Option<String>,
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
        let mut root = doc.finish();
        root.span = Some(Span::new(0, text.len()));
        root
    }
}
