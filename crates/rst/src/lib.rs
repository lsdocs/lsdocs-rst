//! A reStructuredText parser.
//!
//! Parses reStructuredText into a document tree modeled on
//! [docutils](https://docutils.sourceforge.io/)' doctree: the same element
//! kinds and attributes, so the tree prints as the same pseudo-XML docutils
//! produces. Unlike docutils, every node also records the byte span of the
//! text it came from.
//!
//! ```
//! let doc = lsdocs_rst::Parser::new().source("example").parse("Hello, world.\n");
//! assert_eq!(
//!     doc.pformat(),
//!     "<document source=\"example\">\n    <paragraph>\n        Hello, world.\n"
//! );
//! ```
//!
//! The crate stands alone: it depends on nothing from lsdocs, and lsdocs'
//! `parser-rst` plugin converts its tree into lsdocs' own.
//!
//! **Status:** early. The parser handles the block structure (sections,
//! lists, tables, explicit markup and the rest), inline markup with the
//! standard roles, and most standard directives; the table directives
//! and docutils' transforms are being added, each checked against
//! docutils' own output (see `tests/conformance.rs`).

pub mod lines;
pub mod nodes;
mod parser;
mod patterns;
mod pformat;
mod tables;
mod util;

pub use nodes::{AttrValue, Attributes, Element, Kind, Node, Span, Text};
pub use parser::Parser;

/// Parses `text` with the default settings. See [`Parser::parse`].
pub fn parse(text: &str) -> Element {
    Parser::new().parse(text)
}
