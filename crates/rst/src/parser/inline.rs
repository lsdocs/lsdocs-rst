//! Inline text. Inline markup is not parsed yet: text is kept as is, with
//! backslash escapes removed.

use crate::nodes::{Node, Span, Text};
use crate::util::{escape2null, unescape};

/// The nodes for inline `text` (docutils' `Inliner.parse`); none for empty
/// text.
pub(crate) fn inline_text(text: &str, span: Option<Span>) -> Vec<Node> {
    if text.is_empty() {
        return Vec::new();
    }
    vec![Text::new(unescape(&escape2null(text)), span).into()]
}
