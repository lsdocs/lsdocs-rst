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
//!   under its name in lower case, with `true`, `false` and numbers as
//!   such (as in YAML front matter). A `title` field names the page. `meta` directives go
//!   under `meta`.
//! - **Ids:** labels (`.. _name:`) and `:name:` options become ids (made
//!   as docutils makes them: `My Label` is `my-label`). A section without
//!   a label gets docutils' id only if something in the document links to
//!   it; the host gives the others ids.
//! - **Links:** references resolve as docutils resolves them. Links
//!   across the site (Sphinx's `:ref:` and `:doc:`) are roles the
//!   `sphinx-addons` plugin declares.
//! - **Core kinds:** admonitions, footnotes and citations (as footnotes),
//!   math, figures (a table's title makes it a figure's caption),
//!   definition lists (field and option lists too), line blocks
//!   (paragraphs with line breaks).
//! - **Custom kinds:** topics (`topic`, `contents`, `dedication`,
//!   `abstract`), `sidebar`, `rubric`, `container`, `legend`, `header`
//!   and `footer`. A directive with no implementation becomes a custom
//!   node of its name, with `argument`, `options` and `content`
//!   attributes, for transformers to handle.
//! - **Syntax extensions:** directives and roles plugins declare (see
//!   "Syntax extensions" in `spec/wire.md`) are parsed by their
//!   declaration, into the custom nodes the spec describes: a
//!   directive's options are checked and its content parsed if it is
//!   markup; a role keeps its text.
//! - **Diagnostics:** docutils' warnings and errors become warnings, its
//!   severe errors errors; info messages are dropped.
//! - **Files:** `include`, `raw` and `csv-table` read files from the
//!   source directory (option `file_access`, on by default). The files
//!   read (or looked for) are the result's dependencies, so a cached
//!   result is reused only while they are unchanged.
//!
//! Options:
//!
//! | Option | Default | Meaning |
//! |---|---|---|
//! | `file_access` | `true` | Let directives read files. |

mod convert;

use std::sync::{Arc, Mutex, PoisonError};

use lsdocs_plugin_sdk::doctree::Value;
use lsdocs_plugin_sdk::protocol::{
    ArgumentSyntax, ContentSyntax, DirectiveSyntax, OptionSyntax, ParseParams, ParseResult, Syntax,
};
use lsdocs_plugin_sdk::{Context, Parser, Plugin, PluginResult};
use lsdocs_rst::nodes::{Element, Kind, Node, Text};
use lsdocs_rst::{
    Directive, DirectiveContext, DirectiveError, DirectiveSpec, OptionKind, OptionValue, Role,
    RoleContext,
};

pub use convert::convert;

/// The plugin, as registered with the host.
pub fn plugin() -> Plugin {
    Plugin::new("parser-rst", env!("CARGO_PKG_VERSION"))
        .parser(Rst)
        .cacheable()
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
        Ok(parse_with_dependencies(
            &params.source,
            &params.text,
            source_dir.as_deref(),
            &params.syntax,
        ))
    }
}

/// The `lsdocs-rst` parser the plugin uses: docutils' transforms without
/// the document title, unknown directives kept, and the directives and
/// roles of `syntax`; with `source_dir`, files are read from it.
pub fn rst_parser(source: &str, source_dir: Option<&str>, syntax: &Syntax) -> lsdocs_rst::Parser {
    rst_parser_recording(source, source_dir, syntax, None)
}

/// [`rst_parser`], noting every file directives read (or look for) in
/// `read`.
fn rst_parser_recording(
    source: &str,
    source_dir: Option<&str>,
    syntax: &Syntax,
    read: Option<Arc<Mutex<Vec<String>>>>,
) -> lsdocs_rst::Parser {
    let mut parser = lsdocs_rst::Parser::new()
        .source(source)
        .transforms(true)
        .doctitle(false)
        .keep_unknown_directives(true);
    for directive in &syntax.directives {
        parser = parser.directive(&directive.name, Declared(directive.clone()));
    }
    for role in &syntax.roles {
        parser = parser.role(&role.name, DeclaredRole(role.name.clone()));
    }
    if let Some(dir) = source_dir {
        parser = match read {
            Some(read) => parser.file_loader(Recording {
                inner: lsdocs_rst::DirectoryLoader::new(dir),
                dir: dir.to_owned(),
                read,
            }),
            None => parser.file_loader(lsdocs_rst::DirectoryLoader::new(dir)),
        };
    }
    parser
}

/// A loader noting the files it is asked for.
struct Recording {
    inner: lsdocs_rst::DirectoryLoader,
    dir: String,
    read: Arc<Mutex<Vec<String>>>,
}

impl lsdocs_rst::FileLoader for Recording {
    fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
        let full = std::path::Path::new(&self.dir).join(path);
        self.read
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(full.to_string_lossy().replace('\\', "/"));
        self.inner.read_file(path)
    }
}

/// Parses as [`parse`] does, with the files directives read as the
/// result's dependencies.
pub fn parse_with_dependencies(
    source: &str,
    text: &str,
    source_dir: Option<&str>,
    syntax: &Syntax,
) -> ParseResult {
    let read = Arc::new(Mutex::new(Vec::new()));
    let (tree, messages) = rst_parser_recording(source, source_dir, syntax, Some(read.clone()))
        .parse_with_messages(text);
    let (document, diagnostics) = convert(source, text, &tree, &messages);
    let mut dependencies =
        std::mem::take(&mut *read.lock().unwrap_or_else(PoisonError::into_inner));
    dependencies.sort();
    dependencies.dedup();
    ParseResult {
        document,
        diagnostics,
        dependencies,
    }
}

/// Parses reStructuredText into a document, plus the problems docutils
/// reported. `source` is the file's path relative to the source
/// directory; with `source_dir`, directives may read files from it.
/// `syntax` holds the directives and roles plugins declared.
pub fn parse(
    source: &str,
    text: &str,
    source_dir: Option<&str>,
    syntax: &Syntax,
) -> (
    lsdocs_plugin_sdk::doctree::Document,
    Vec<lsdocs_plugin_sdk::protocol::Diagnostic>,
) {
    let (tree, messages) = rst_parser(source, source_dir, syntax).parse_with_messages(text);
    convert(source, text, &tree, &messages)
}

/// Element kinds declared directives and roles make, converted to custom
/// nodes.
pub(crate) const DIRECTIVE: &str = "lsdocs-directive";
pub(crate) const ARGUMENT: &str = "lsdocs-directive-argument";
pub(crate) const ROLE: &str = "lsdocs-role";

/// A directive a plugin declared: its parts, parsed as declared, in an
/// element for `convert` to turn into a custom node. Its `attrs`
/// attribute holds the custom node's attributes as JSON.
struct Declared(DirectiveSyntax);

impl Directive for Declared {
    fn spec(&self) -> DirectiveSpec {
        let d = &self.0;
        DirectiveSpec {
            required_arguments: d.required_arguments as usize,
            optional_arguments: d.optional_arguments as usize,
            final_argument_whitespace: d.final_argument_whitespace,
            options: d
                .options
                .iter()
                .map(|(name, kind)| {
                    let kind = match kind {
                        OptionSyntax::Flag => OptionKind::Flag,
                        OptionSyntax::Value => OptionKind::Text,
                        OptionSyntax::Required => OptionKind::RequiredText,
                    };
                    (name.clone(), kind)
                })
                .collect(),
            has_content: d.content != ContentSyntax::None,
        }
    }

    fn run(&self, cx: &mut DirectiveContext<'_>) -> Result<Vec<Node>, DirectiveError> {
        let d = &self.0;
        let mut attrs = serde_json::Map::new();
        let argument = cx.arguments().join(" ");
        if !argument.is_empty() {
            attrs.insert("argument".into(), Value::String(argument.clone()));
        }
        let mut options = serde_json::Map::new();
        for (name, value) in cx.options() {
            let value = match value {
                OptionValue::Flag => Value::Bool(true),
                OptionValue::Text(text) => Value::String(text),
                OptionValue::Int(n) => Value::String(n.to_string()),
                OptionValue::List(items) => Value::String(items.join(" ")),
                _ => continue,
            };
            options.insert(name, value);
        }
        if !options.is_empty() {
            attrs.insert("options".into(), Value::Object(options));
        }
        let mut element =
            Element::new(Kind::Other(DIRECTIVE.into())).with_attr("name", d.name.as_str());
        element.span = cx.span();
        match d.content {
            ContentSyntax::Text => {
                let content = cx.content();
                if !content.is_empty() {
                    attrs.insert("content".into(), Value::String(content.join("\n")));
                }
            }
            ContentSyntax::Blocks => element = cx.parse_content(element),
            ContentSyntax::None => {}
        }
        let mut out = Vec::new();
        if d.argument == ArgumentSyntax::Inline && !argument.is_empty() {
            let (nodes, messages) = cx.parse_inline(&argument);
            let mut child = Element::new(Kind::Other(ARGUMENT.into())).with_children(nodes);
            child.span = cx.span();
            element.children.insert(0, child.into());
            out.extend(messages.into_iter().map(Node::from));
        }
        let attrs = serde_json::to_string(&attrs).unwrap_or_default();
        element = element.with_attr("attrs", attrs.as_str());
        out.insert(0, element.into());
        Ok(out)
    }
}

/// A role a plugin declared: its name and text, for `convert` to turn
/// into a custom node.
struct DeclaredRole(String);

impl Role for DeclaredRole {
    fn run(&self, cx: &mut RoleContext<'_>) -> Result<Vec<Node>, DirectiveError> {
        let text = cx.text();
        let mut node = Element::new(Kind::Other(ROLE.into()))
            .with_attr("name", self.0.as_str())
            .with_attr("text", text.as_str());
        node.span = cx.span();
        node.children.push(Text::new(text, cx.span()).into());
        Ok(vec![node.into()])
    }
}

#[cfg(test)]
mod tests;
