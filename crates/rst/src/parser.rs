//! The reStructuredText parser.
//!
//! So far (Phase 8a) it only splits the document into paragraphs: runs of
//! non-blank lines separated by blank lines. The block constructs and
//! inline markup follow in later steps.

use crate::lines::{Line, lines};
use crate::nodes::{Element, Kind, Span, Text};

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
        let mut document = Element::new(Kind::Document).with_span(Span::new(0, text.len()));
        if let Some(source) = &self.source {
            document.attributes.set("source", source.as_str());
        }
        let lines = lines(text);
        let mut i = 0;
        while i < lines.len() {
            if lines[i].is_blank() {
                i += 1;
                continue;
            }
            let end = lines[i..]
                .iter()
                .position(Line::is_blank)
                .map_or(lines.len(), |n| i + n);
            document.children.push(paragraph(&lines[i..end]).into());
            i = end;
        }
        document
    }
}

/// A paragraph of `lines`, joined with line breaks.
fn paragraph(lines: &[Line]) -> Element {
    let span = lines[0].span.to(lines[lines.len() - 1].span);
    let text: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
    Element::new(Kind::Paragraph)
        .with_span(span)
        .with_child(Text::new(text.join("\n"), Some(span)))
}
