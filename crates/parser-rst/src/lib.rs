//! First-party reStructuredText parser plugin.
//!
//! Parses reStructuredText with `lsdocs-rst` (a port of docutils' parser
//! and transforms) and converts docutils' tree into the shared shape (see
//! "Document shape" in `spec/wire.md`).
//!
//! - **Sections** nest as written. As in Sphinx, a lone top-level title is
//!   not promoted to a document title: it stays a section.
//! - **Metadata:** a field list at the very top of the file (docutils'
//!   bibliographic fields) becomes the document's metadata: `author`,
//!   `date` and the like as strings, `authors` as a list, any other field
//!   under its name in lower case. A `title` field names the page. `meta` directives go
//!   under `meta`.
//! - **Ids:** labels (`.. _name:`) and `:name:` options become ids (made
//!   as docutils makes them: `My Label` is `my-label`). A section without
//!   a label gets docutils' id only if something in the document links to
//!   it; the host gives the others ids.
//! - **Links:** references resolve as docutils resolves them. Two roles
//!   from Sphinx link across the site: `` :ref:`label` `` (or
//!   `` :ref:`text <label>` ``) points to a label anywhere, and
//!   `` :doc:`path` `` to another reStructuredText document (relative to
//!   this one, or to the source directory with a leading `/`; `.rst` is
//!   added). Without text, `transform-links` fills in the target's title.
//! - **Core kinds:** admonitions, footnotes and citations (as footnotes),
//!   math, figures (a table's title makes it a figure's caption),
//!   definition lists (field and option lists too), line blocks
//!   (paragraphs with line breaks).
//! - **Custom kinds:** topics (`topic`, `contents`, `dedication`,
//!   `abstract`), `sidebar`, `rubric`, `container`, `legend`, `header`
//!   and `footer`. A directive with no implementation becomes a custom
//!   node of its name, with `argument`, `options` and `content`
//!   attributes, for transformers to handle.
//! - **Diagnostics:** docutils' warnings and errors become warnings, its
//!   severe errors errors; info messages are dropped.
//! - **Files:** `include`, `raw` and `csv-table` read files from the
//!   source directory (option `file_access`, on by default). Results are
//!   not cached while it is on, since they depend on other files.
//!
//! Options:
//!
//! | Option | Default | Meaning |
//! |---|---|---|
//! | `file_access` | `true` | Let directives read files. |

mod convert;

use lsdocs_plugin_sdk::doctree::Value;
use lsdocs_plugin_sdk::protocol::{InitializeParams, ParseParams, ParseResult};
use lsdocs_plugin_sdk::{Context, Parser, Plugin, PluginResult};
use lsdocs_rst::nodes::{Element, Kind, Node, Text};
use lsdocs_rst::{DirectiveError, Role, RoleContext};

pub use convert::convert;

/// The plugin, as registered with the host.
pub fn plugin() -> Plugin {
    Plugin::new("parser-rst", env!("CARGO_PKG_VERSION"))
        .parser(Rst)
        .cache_key(|init: &InitializeParams| (!file_access(&init.options)).then(String::new))
}

/// The reStructuredText parser.
#[derive(Debug, Clone, Copy, Default)]
pub struct Rst;

fn file_access(options: &Value) -> bool {
    options
        .get("file_access")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

impl Parser for Rst {
    fn extensions(&self) -> Vec<String> {
        vec!["rst".into()]
    }

    async fn parse(&self, params: ParseParams, ctx: &Context) -> PluginResult<ParseResult> {
        let source_dir = ctx
            .site()
            .source_dir
            .clone()
            .filter(|_| file_access(ctx.options()));
        let (document, diagnostics) = parse(&params.source, &params.text, source_dir.as_deref());
        Ok(ParseResult {
            document,
            diagnostics,
        })
    }
}

/// The `lsdocs-rst` parser the plugin uses: docutils' transforms without
/// the document title, unknown directives kept, and the `ref` and `doc`
/// roles; with `source_dir`, files are read from it.
pub fn rst_parser(source: &str, source_dir: Option<&str>) -> lsdocs_rst::Parser {
    let mut parser = lsdocs_rst::Parser::new()
        .source(source)
        .transforms(true)
        .doctitle(false)
        .keep_unknown_directives(true)
        .role("ref", RefRole)
        .role("doc", DocRole);
    if let Some(dir) = source_dir {
        parser = parser.file_loader(lsdocs_rst::DirectoryLoader::new(dir));
    }
    parser
}

/// Parses reStructuredText into a document, plus the problems docutils
/// reported. `source` is the file's path relative to the source
/// directory; with `source_dir`, directives may read files from it.
pub fn parse(
    source: &str,
    text: &str,
    source_dir: Option<&str>,
) -> (
    lsdocs_plugin_sdk::doctree::Document,
    Vec<lsdocs_plugin_sdk::protocol::Diagnostic>,
) {
    let (tree, messages) = rst_parser(source, source_dir).parse_with_messages(text);
    convert(source, text, &tree, &messages)
}

/// Element kinds the roles make, converted to links.
pub(crate) const REF: &str = "lsdocs-ref";
pub(crate) const DOC: &str = "lsdocs-doc";

/// `text <target>`: the text (if any) and the target.
fn split_title(text: &str) -> (Option<String>, String) {
    let text = text.trim();
    if let Some(rest) = text.strip_suffix('>')
        && let Some(i) = rest.rfind('<')
        && i > 0
        && rest[..i].ends_with(char::is_whitespace)
    {
        let title = rest[..i].trim();
        return (Some(title.to_owned()), rest[i + 1..].trim().to_owned());
    }
    (None, text.to_owned())
}

fn role_node(kind: &str, attr: &str, cx: &RoleContext<'_>) -> Result<Vec<Node>, DirectiveError> {
    let (title, target) = split_title(&cx.text());
    if target.is_empty() {
        return Err(DirectiveError::error(format!(
            "The \"{}\" role needs a target.",
            cx.name()
        )));
    }
    let mut node = Element::new(Kind::Other(kind.into())).with_attr(attr, target);
    node.span = cx.span();
    if let Some(title) = title {
        node.children.push(Text::new(title, cx.span()).into());
    }
    Ok(vec![node.into()])
}

/// `` :ref:`label` ``: a link to a label anywhere in the site.
struct RefRole;

impl Role for RefRole {
    fn run(&self, cx: &mut RoleContext<'_>) -> Result<Vec<Node>, DirectiveError> {
        role_node(REF, "label", cx)
    }
}

/// `` :doc:`path` ``: a link to another reStructuredText document.
struct DocRole;

impl Role for DocRole {
    fn run(&self, cx: &mut RoleContext<'_>) -> Result<Vec<Node>, DirectiveError> {
        role_node(DOC, "path", cx)
    }
}

#[cfg(test)]
mod tests;
