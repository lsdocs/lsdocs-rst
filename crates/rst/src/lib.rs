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
//! The parser handles the block structure (sections, lists, tables,
//! explicit markup and the rest), inline markup with the standard roles,
//! the standard directives and, optionally, docutils' transforms
//! ([`Parser::transforms`]), each checked against docutils' own output
//! (see `tests/conformance.rs`). Beyond that:
//!
//! - [`settings`]: the docutils settings that change the tree, by their
//!   docutils names.
//! - [`extension`]: directives and roles of your own.
//! - [`files`]: letting directives such as `include` read files.
//! - [`nodes`]: the tree, and with the `serde` feature its serialized form.
//! - [`xml`]: Docutils XML output, as `docutils --writer=xml` writes it.
//!
//! **Features:** `serde` derives `Serialize` and `Deserialize` for the tree,
//! [`Settings`] and [`XmlOptions`].

pub mod extension;
pub mod files;
pub mod lines;
pub mod nodes;
mod parser;
mod patterns;
mod pformat;
pub mod settings;
mod tables;
mod util;
pub mod xml;

pub use extension::{
    Directive, DirectiveContext, DirectiveError, DirectiveSpec, OptionKind, OptionValue, Role,
    RoleContext,
};
pub use files::{DirectoryLoader, FileLoader};
pub use nodes::{AttrValue, Attributes, Descendants, Element, Kind, Node, Span, Text};
pub use parser::Parser;
pub use settings::{Settings, TocBacklinks};
pub use xml::XmlOptions;

/// `name` as docutils makes ids of names (`docutils.nodes.make_id`):
/// lowercase ASCII letters and digits, with every run of anything else a
/// single hyphen, and no hyphen or digit first.
pub fn make_id(name: &str) -> String {
    util::make_id(name)
}

/// Parses `text` with the default settings. See [`Parser::parse`].
pub fn parse(text: &str) -> Element {
    Parser::new().parse(text)
}
