//! docutils' standard directives (`docutils.parsers.rst.directives`).

use super::options::{Opt, OptValue, class_option, py_repr, unicode_code};
use super::{Call, Failure, Run, Spec, failure};
use crate::nodes::{AttrValue, Element, Kind, Node, Text};
use crate::parser::document::{
    At, Document, ERROR, INFO, LINE, LINE_INDENT, MsgNode, RAW, WARNING, fixed,
};
use crate::parser::inline::{Origin, astext, inline_text};
use crate::parser::machine::{Machine, State};
use crate::util::{make_id, normalize_name, split_whitespace, strip, whitespace_normalize_name};

/// Every directive implemented here, by canonical name.
pub(super) const NAMES: &[&str] = &[
    "attention",
    "caution",
    "danger",
    "error",
    "hint",
    "important",
    "note",
    "tip",
    "warning",
    "admonition",
    "sidebar",
    "topic",
    "line-block",
    "parsed-literal",
    "code",
    "math",
    "rubric",
    "epigraph",
    "highlights",
    "pull-quote",
    "compound",
    "container",
    "image",
    "figure",
    "raw",
    "include",
    "replace",
    "unicode",
    "class",
    "role",
    "default-role",
    "title",
    "meta",
    "date",
    "contents",
    "sectnum",
    "header",
    "footer",
    "target-notes",
    "restructuredtext-test-directive",
    "table",
    "csv-table",
    "list-table",
];

const CLASS_NAME: &[(&str, Opt)] = &[("class", Opt::Class), ("name", Opt::Unchanged)];

const ALIGN: &[&str] = &["top", "middle", "bottom", "left", "center", "right"];
const ALIGN_H: &[&str] = &["left", "center", "right"];
const ALIGN_V: &[&str] = &["top", "middle", "bottom"];

const IMAGE_OPTIONS: &[(&str, Opt)] = &[
    ("alt", Opt::Unchanged),
    ("height", Opt::LengthOrUnitless),
    ("width", Opt::LengthOrPercentageOrUnitless),
    ("scale", Opt::Percentage),
    ("align", Opt::Choice(ALIGN)),
    ("target", Opt::UnchangedRequired),
    ("loading", Opt::Choice(&["embed", "link", "lazy"])),
    ("class", Opt::Class),
    ("name", Opt::Unchanged),
];

const FIGURE_OPTIONS: &[(&str, Opt)] = &[
    ("alt", Opt::Unchanged),
    ("height", Opt::LengthOrUnitless),
    ("width", Opt::LengthOrPercentageOrUnitless),
    ("scale", Opt::Percentage),
    ("align", Opt::Choice(ALIGN_H)),
    ("target", Opt::UnchangedRequired),
    ("loading", Opt::Choice(&["embed", "link", "lazy"])),
    ("class", Opt::Class),
    ("name", Opt::Unchanged),
    ("figwidth", Opt::FigWidth),
    ("figclass", Opt::Class),
    ("figname", Opt::Unchanged),
];

pub(super) fn lookup(name: &str) -> Option<(Spec, Run)> {
    let content = Spec {
        has_content: true,
        ..Spec::new()
    };
    let titled = Spec {
        final_whitespace: true,
        options: Some(CLASS_NAME),
        has_content: true,
        ..Spec::new()
    };
    Some(match name {
        "attention" | "caution" | "danger" | "error" | "hint" | "important" | "note" | "tip"
        | "warning" => (titled, admonition as Run),
        "admonition" => (
            Spec {
                required: 1,
                ..titled
            },
            admonition,
        ),
        "topic" => (
            Spec {
                required: 1,
                ..titled
            },
            topic,
        ),
        "sidebar" => (
            Spec {
                optional: 1,
                options: Some(&[
                    ("class", Opt::Class),
                    ("name", Opt::Unchanged),
                    ("subtitle", Opt::UnchangedRequired),
                ]),
                ..titled
            },
            topic,
        ),
        "line-block" => (
            Spec {
                options: Some(CLASS_NAME),
                ..content
            },
            line_block,
        ),
        "parsed-literal" => (
            Spec {
                options: Some(CLASS_NAME),
                ..content
            },
            parsed_literal,
        ),
        "code" => (
            Spec {
                optional: 1,
                options: Some(&[
                    ("class", Opt::Class),
                    ("name", Opt::Unchanged),
                    ("number-lines", Opt::IntOrNone),
                ]),
                ..content
            },
            code,
        ),
        "math" => (
            Spec {
                options: Some(CLASS_NAME),
                ..content
            },
            math,
        ),
        "rubric" => (
            Spec {
                required: 1,
                final_whitespace: true,
                options: Some(CLASS_NAME),
                ..Spec::new()
            },
            rubric,
        ),
        "epigraph" | "highlights" | "pull-quote" => (content, block_quote),
        "compound" => (
            Spec {
                options: Some(CLASS_NAME),
                ..content
            },
            compound,
        ),
        "container" => (
            Spec {
                optional: 1,
                final_whitespace: true,
                options: Some(&[("name", Opt::Unchanged)]),
                ..content
            },
            container,
        ),
        "image" => (
            Spec {
                required: 1,
                final_whitespace: true,
                options: Some(IMAGE_OPTIONS),
                ..Spec::new()
            },
            image,
        ),
        "figure" => (
            Spec {
                required: 1,
                final_whitespace: true,
                options: Some(FIGURE_OPTIONS),
                has_content: true,
                ..Spec::new()
            },
            figure,
        ),
        "raw" => (
            Spec {
                required: 1,
                final_whitespace: true,
                options: Some(&[
                    ("file", Opt::Path),
                    ("url", Opt::Uri),
                    ("encoding", Opt::Encoding),
                    ("class", Opt::Class),
                ]),
                has_content: true,
                ..Spec::new()
            },
            raw,
        ),
        "include" => (
            Spec {
                required: 1,
                final_whitespace: true,
                options: Some(&[
                    ("literal", Opt::Flag),
                    ("code", Opt::Unchanged),
                    ("encoding", Opt::Encoding),
                    ("parser", Opt::ParserName),
                    ("tab-width", Opt::Int),
                    ("start-line", Opt::Int),
                    ("end-line", Opt::Int),
                    ("start-after", Opt::Unchanged),
                    ("end-before", Opt::Unchanged),
                    ("number-lines", Opt::IntOrNone),
                    ("class", Opt::Class),
                    ("name", Opt::Unchanged),
                ]),
                ..Spec::new()
            },
            include,
        ),
        "replace" => (content, replace),
        "unicode" => (
            Spec {
                required: 1,
                final_whitespace: true,
                options: Some(&[
                    ("trim", Opt::Flag),
                    ("ltrim", Opt::Flag),
                    ("rtrim", Opt::Flag),
                ]),
                ..Spec::new()
            },
            unicode,
        ),
        "class" => (
            Spec {
                required: 1,
                final_whitespace: true,
                has_content: true,
                ..Spec::new()
            },
            class,
        ),
        "role" => (content, role),
        "default-role" => (
            Spec {
                optional: 1,
                ..Spec::new()
            },
            default_role,
        ),
        "title" => (
            Spec {
                required: 1,
                final_whitespace: true,
                ..Spec::new()
            },
            title,
        ),
        "meta" => (content, meta),
        "date" => (content, date),
        "contents" => (
            Spec {
                optional: 1,
                final_whitespace: true,
                options: Some(&[
                    ("depth", Opt::NonNegativeInt),
                    ("local", Opt::Flag),
                    ("backlinks", Opt::Backlinks),
                    ("class", Opt::Class),
                ]),
                ..Spec::new()
            },
            contents,
        ),
        "sectnum" => (
            Spec {
                options: Some(&[
                    ("depth", Opt::Int),
                    ("start", Opt::Int),
                    ("prefix", Opt::UnchangedRequired),
                    ("suffix", Opt::UnchangedRequired),
                ]),
                ..Spec::new()
            },
            sectnum,
        ),
        "header" | "footer" => (content, decoration),
        "target-notes" => (
            Spec {
                options: Some(&[("class", Opt::Class)]),
                ..Spec::new()
            },
            target_notes,
        ),
        "table" | "csv-table" | "list-table" => {
            let (options, run): (&'static [(&'static str, Opt)], Run) = match name {
                "table" => (super::tables::TABLE_OPTIONS, super::tables::table),
                "csv-table" => (super::tables::CSV_OPTIONS, super::tables::csv_table),
                _ => (super::tables::LIST_OPTIONS, super::tables::list_table),
            };
            (
                Spec {
                    optional: 1,
                    final_whitespace: true,
                    options: Some(options),
                    has_content: true,
                    ..Spec::new()
                },
                run,
            )
        }
        "restructuredtext-test-directive" => (
            Spec {
                optional: 1,
                final_whitespace: true,
                options: Some(&[("option", Opt::UnchangedRequired)]),
                has_content: true,
                ..Spec::new()
            },
            test_directive,
        ),
        _ => return None,
    })
}

// -- helpers ----------------------------------------------------------------

fn assert_has_content(call: &Call) -> Result<(), Failure> {
    if call.content.is_empty() {
        return Err(failure(
            ERROR,
            format!(
                "Content block expected for the \"{}\" directive; none found.",
                call.name
            ),
        ));
    }
    Ok(())
}

/// Sets the `classes` from the `class` option (docutils'
/// `normalize_options`).
pub(super) fn set_classes(node: &mut Element, call: &Call) {
    for class in call.classes() {
        node.attributes.push("classes", class);
    }
}

/// The `name` option becomes a target name of `node` (docutils'
/// `add_name`).
pub(super) fn add_name(doc: &mut Document, call: &mut Call, node: &mut Element) {
    if let Some(name) = call.take("name") {
        let name = normalize_name(name.as_str().unwrap_or(""));
        node.attributes.remove("name");
        node.attributes.push("names", name);
        doc.note_explicit_target(node, Some(MsgNode::Itself), Some(call.lineno));
    }
}

/// Inline text on the directive's line.
fn inline(doc: &mut Document, call: &Call, text: &str) -> (Vec<Node>, Vec<Element>) {
    inline_text(doc, text, call.lineno, &Origin::default())
}

/// Parses the content into `node`.
pub(super) fn parse_content(
    m: &mut Machine,
    doc: &mut Document,
    call: &Call,
    node: Element,
) -> Element {
    doc.open(node, false);
    m.nested_parse(doc, call.content.clone(), call.content_offset);
    doc.close().unwrap()
}

/// The nodes parsed from the content, in an anonymous container.
pub(super) fn parse_into_container(m: &mut Machine, doc: &mut Document, call: &Call) -> Vec<Node> {
    parse_content(m, doc, call, Element::new(Kind::Other("Element".into()))).children
}

/// A `pending` element: work for a transform, with its details.
fn pending(transform: &str, details: Vec<(String, AttrValue)>) -> Element {
    let mut p = Element::new(Kind::Pending).with_attr(".transform", transform);
    for (key, value) in details {
        p.attributes.set(format!(".details.{key}"), value);
    }
    p
}

/// An option value as a pending detail.
fn detail(value: &OptValue) -> AttrValue {
    match value {
        OptValue::None => AttrValue::None,
        OptValue::Str(s) => AttrValue::Str(s.clone()),
        OptValue::Int(n) => AttrValue::Int(*n),
        OptValue::List(l) => AttrValue::List(l.clone()),
        OptValue::IntList(l) => AttrValue::List(l.iter().map(i64::to_string).collect()),
    }
}

fn not_in_substitution(call: &Call) -> Result<(), Failure> {
    if call.in_substitution {
        Ok(())
    } else {
        Err(failure(
            ERROR,
            format!(
                "Invalid context: the \"{}\" directive can only be used within a substitution definition.",
                call.name
            ),
        ))
    }
}

/// Whether a topic or sidebar may not go into an element of `kind`.
fn invalid_parent(kind: &Kind) -> bool {
    !matches!(
        kind,
        Kind::Document | Kind::Section | Kind::Sidebar | Kind::Other(_)
    )
}

// -- directives ---------------------------------------------------------------

fn admonition(m: &mut Machine, doc: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    assert_has_content(call)?;
    let kind = Kind::from_tag(&call.name.to_lowercase());
    let kind = if matches!(kind, Kind::Other(_)) {
        Kind::Admonition
    } else {
        kind
    };
    let mut node = Element::new(kind.clone());
    node.span = call.span;
    set_classes(&mut node, call);
    add_name(doc, call, &mut node);
    if kind == Kind::Admonition {
        let title_text = call.arguments[0].clone();
        let (nodes, messages) = inline(doc, call, &title_text);
        node.children
            .push(Element::new(Kind::Title).with_children(nodes).into());
        node.children.extend(messages.into_iter().map(Node::from));
        if !call.has("class") {
            node.attributes
                .push("classes", format!("admonition-{}", make_id(&title_text)));
        }
    }
    Ok(vec![parse_content(m, doc, call, node).into()])
}

fn topic(m: &mut Machine, doc: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    let sidebar = call.name.eq_ignore_ascii_case("sidebar");
    let parent = doc.top().kind.clone();
    if sidebar {
        if parent == Kind::Sidebar {
            return Err(failure(
                ERROR,
                format!(
                    "The \"{}\" directive may not be used within a sidebar element.",
                    call.name
                ),
            ));
        }
        if call.has("subtitle") && call.arguments.is_empty() {
            return Err(failure(
                ERROR,
                "The \"subtitle\" option may not be used without a title.",
            ));
        }
    }
    if invalid_parent(&parent) {
        return Err(failure(
            ERROR,
            format!(
                "The \"{}\" directive may not be used within topics or body elements.",
                call.name
            ),
        ));
    }
    assert_has_content(call)?;
    let mut node = Element::new(if sidebar { Kind::Sidebar } else { Kind::Topic });
    node.span = call.span;
    if let Some(title_text) = call.arguments.first().cloned() {
        let (nodes, mut messages) = inline(doc, call, &title_text);
        node.children
            .push(Element::new(Kind::Title).with_children(nodes).into());
        if let Some(subtitle) = call
            .option("subtitle")
            .and_then(OptValue::as_str)
            .map(str::to_owned)
        {
            let (nodes, more) = inline(doc, call, &subtitle);
            node.children
                .push(Element::new(Kind::Subtitle).with_children(nodes).into());
            messages.extend(more);
        }
        node.children.extend(messages.into_iter().map(Node::from));
    }
    set_classes(&mut node, call);
    add_name(doc, call, &mut node);
    if !call.content_text().is_empty() {
        node = parse_content(m, doc, call, node);
    }
    Ok(vec![node.into()])
}

fn line_block(_: &mut Machine, doc: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    assert_has_content(call)?;
    let mut block = Element::new(Kind::LineBlock);
    block.span = call.span;
    set_classes(&mut block, call);
    add_name(doc, call, &mut block);
    let mut out = Vec::new();
    let mut lines = Vec::new();
    for (offset, line) in (call.content_offset..).zip(call.content.clone()) {
        let text = strip(&line.text).to_owned();
        // docutils adds the directive's line number and the content offset.
        let lineno = (call.lineno as isize + offset).max(0) as usize;
        let (nodes, messages) =
            inline_text(doc, &text, lineno, &Origin::of(std::slice::from_ref(&line)));
        let mut l = Element::new(Kind::Line).with_children(nodes);
        l.span = Some(line.span);
        if !text.is_empty() {
            let indent =
                line.text.chars().count() - crate::util::lstrip(&line.text).chars().count();
            l.attributes.set(LINE_INDENT, indent as i64);
        }
        lines.push(Node::from(l));
        out.extend(messages.into_iter().map(Node::from));
    }
    let mut previous = 0;
    for node in &mut lines {
        if let Node::Element(e) = node {
            match e.attributes.get(LINE_INDENT) {
                Some(AttrValue::Int(i)) => previous = *i,
                _ => e.attributes.set(LINE_INDENT, previous),
            }
        }
    }
    block.children = crate::parser::body::nest_line_block_segment(lines);
    out.insert(0, block.into());
    Ok(out)
}

fn parsed_literal(
    _: &mut Machine,
    doc: &mut Document,
    call: &mut Call,
) -> Result<Vec<Node>, Failure> {
    assert_has_content(call)?;
    let text = call.content_text();
    let (nodes, messages) = inline_text(doc, &text, call.lineno, &Origin::of(&call.content));
    let mut node = fixed(Kind::LiteralBlock, "", call.span).with_children(nodes);
    set_classes(&mut node, call);
    add_name(doc, call, &mut node);
    let mut out = vec![Node::from(node)];
    out.extend(messages.into_iter().map(Node::from));
    Ok(out)
}

fn code(_: &mut Machine, doc: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    assert_has_content(call)?;
    let language = call.arguments.first().cloned().unwrap_or_default();
    let text = call.content_text();
    let lines = call.content.len();
    Ok(vec![
        code_block(doc, call, &language, &text, lines, None).into(),
    ])
}

/// A `code` directive's literal block for `text` (`lines` lines long, as
/// the directive counts them), from file `source` if it was included.
pub(super) fn code_block(
    doc: &mut Document,
    call: &mut Call,
    language: &str,
    text: &str,
    lines: usize,
    source: Option<&str>,
) -> Element {
    let mut node = fixed(Kind::LiteralBlock, "", call.span);
    node.attributes.push("classes", "code");
    if !language.is_empty() {
        node.attributes.push("classes", language);
    }
    set_classes(&mut node, call);
    add_name(doc, call, &mut node);
    if let Some(source) = source {
        node.attributes.set("source", source);
    }
    match call.option("number-lines") {
        Some(start) => {
            let start = match start {
                OptValue::Int(n) => *n,
                _ => 1,
            };
            number_lines(&mut node, text, start, start + lines as i64);
        }
        None => node.children.push(Text::new(text, call.span).into()),
    }
    node
}

/// Adds `text` to `node` with line numbers from `start`, padded to the
/// width of `end` (docutils' `NumberLines`).
pub(super) fn number_lines(node: &mut Element, text: &str, start: i64, end: i64) {
    let width = end.to_string().len();
    let ln = |n: i64| -> Node {
        Element::new(Kind::Inline)
            .with_attr("classes", vec!["ln".to_owned()])
            .with_child(Text::new(format!("{n:>width$} "), None))
            .into()
    };
    let mut lineno = start;
    node.children.push(ln(lineno));
    let parts: Vec<&str> = text.split('\n').collect();
    for part in &parts[..parts.len() - 1] {
        node.children
            .push(Text::new(format!("{part}\n"), None).into());
        lineno += 1;
        node.children.push(ln(lineno));
    }
    node.children
        .push(Text::new(*parts.last().unwrap(), None).into());
}

fn math(_: &mut Machine, doc: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    assert_has_content(call)?;
    let mut out = Vec::new();
    for block in call.content_text().split("\n\n") {
        if block.is_empty() {
            continue;
        }
        let mut node = fixed(Kind::MathBlock, block, call.span);
        set_classes(&mut node, call);
        add_name(doc, call, &mut node);
        out.push(node.into());
    }
    Ok(out)
}

fn rubric(_: &mut Machine, doc: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    let text = call.arguments[0].clone();
    let (nodes, messages) = inline(doc, call, &text);
    let mut node = Element::new(Kind::Rubric).with_children(nodes);
    node.span = call.span;
    set_classes(&mut node, call);
    add_name(doc, call, &mut node);
    let mut out = vec![Node::from(node)];
    out.extend(messages.into_iter().map(Node::from));
    Ok(out)
}

fn block_quote(m: &mut Machine, doc: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    assert_has_content(call)?;
    let class = call.name.to_lowercase();
    let elements = m.block_quote(doc, call.content.clone(), call.content_offset);
    Ok(elements
        .into_iter()
        .map(|mut e| {
            if e.kind == Kind::BlockQuote {
                e.attributes.push("classes", class.clone());
            }
            e.into()
        })
        .collect())
}

fn compound(m: &mut Machine, doc: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    assert_has_content(call)?;
    let mut node = Element::new(Kind::Compound);
    node.span = call.span;
    set_classes(&mut node, call);
    add_name(doc, call, &mut node);
    Ok(vec![parse_content(m, doc, call, node).into()])
}

fn container(m: &mut Machine, doc: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    assert_has_content(call)?;
    let classes = match call.arguments.first() {
        Some(arg) => class_option(Some(arg)).map_err(|_| {
            failure(
                ERROR,
                format!(
                    "Invalid class attribute value for \"{}\" directive: \"{arg}\".",
                    call.name
                ),
            )
        })?,
        None => Vec::new(),
    };
    let mut node = Element::new(Kind::Container);
    node.span = call.span;
    for class in classes {
        node.attributes.push("classes", class);
    }
    add_name(doc, call, &mut node);
    Ok(vec![parse_content(m, doc, call, node).into()])
}

fn image(_: &mut Machine, doc: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    let figure = call.name.eq_ignore_ascii_case("figure");
    if !figure && let Some(align) = call.option("align").and_then(OptValue::as_str) {
        if call.in_substitution {
            if !ALIGN_V.contains(&align) {
                return Err(failure(
                    ERROR,
                    format!(
                        "Error in \"{}\" directive: \"{align}\" is not a valid value for the \"align\" \
                         option within a substitution definition.  Valid values for \"align\" are: \"{}\".",
                        call.name,
                        ALIGN_V.join("\", \"")
                    ),
                ));
            }
        } else if !ALIGN_H.contains(&align) {
            return Err(failure(
                ERROR,
                format!(
                    "Error in \"{}\" directive: \"{align}\" is not a valid value for the \"align\" \
                     option.  Valid values for \"align\" are: \"{}\".",
                    call.name,
                    ALIGN_H.join("\", \"")
                ),
            ));
        }
    }
    let uri = match super::options::convert(Opt::Uri, Some(&call.arguments[0])) {
        Ok(OptValue::Str(s)) => s,
        _ => String::new(),
    };
    let mut reference = None;
    if let Some(target) = call.take("target") {
        let block: Vec<String> =
            crate::lines::split_lines(&crate::util::escape2null(target.as_str().unwrap_or("")))
                .map(str::to_owned)
                .collect();
        let mut r = Element::new(Kind::Reference);
        match crate::parser::explicit::parse_target(&block) {
            crate::parser::explicit::TargetRef::Uri(uri) => r.attributes.set("refuri", uri),
            crate::parser::explicit::TargetRef::Name(name) => {
                r.attributes.set("name", whitespace_normalize_name(&name));
                r.attributes.set("refname", normalize_name(&name));
                doc.note_refname(&mut r);
            }
        }
        reference = Some(r);
    }
    let mut node = Element::new(Kind::Image);
    node.span = call.span;
    for (name, value) in call.options.clone() {
        match (name.as_str(), value) {
            ("class", v) => {
                for c in v.as_list() {
                    node.attributes.push("classes", c);
                }
            }
            ("scale", OptValue::Int(n)) => node.attributes.set("scale", n),
            ("name", _) => {}
            (n, OptValue::Str(s)) => node.attributes.set(n, s),
            (n, OptValue::Int(i)) => node.attributes.set(n, i),
            _ => {}
        }
    }
    node.attributes.set("uri", uri);
    add_name(doc, call, &mut node);
    Ok(vec![match reference {
        Some(r) => r.with_child(node).into(),
        None => node.into(),
    }])
}

fn figure(m: &mut Machine, doc: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    let figwidth = call.take("figwidth");
    let figclasses = call.take("figclass");
    let figname = call.take("figname");
    let align = call.take("align");
    let image_node = image(m, doc, call)?;
    let Some(Node::Element(image_node)) = image_node.into_iter().next() else {
        return Ok(Vec::new());
    };
    if image_node.kind == Kind::SystemMessage {
        return Ok(vec![image_node.into()]);
    }
    let mut figure = Element::new(Kind::Figure).with_child(image_node);
    figure.span = call.span;
    match &figwidth {
        // The image's own width, if it can be read (docutils needs PIL).
        Some(OptValue::Str(w)) if w == "image" => {
            let uri = figure
                .elements()
                .next()
                .and_then(|i| i.attributes.get_str("uri"))
                .map(str::to_owned);
            if let (Some(loader), Some(uri)) = (doc.extensions.loader.clone(), uri)
                && let Ok(bytes) = loader.read_file(&uri)
                && let Some(width) = crate::files::image_width(&bytes)
            {
                figure.attributes.set("width", format!("{width}px"));
            }
        }
        Some(OptValue::Str(w)) => figure.attributes.set("width", w.clone()),
        _ => {}
    }
    if let Some(classes) = figclasses {
        for c in classes.as_list() {
            figure.attributes.push("classes", c);
        }
    }
    if let Some(OptValue::Str(name)) = figname
        && !name.is_empty()
    {
        figure.attributes.push("names", normalize_name(&name));
        doc.note_explicit_target(&mut figure, Some(MsgNode::Itself), Some(call.lineno));
    }
    if let Some(OptValue::Str(align)) = align {
        figure.attributes.set("align", align);
    }
    if !call.content.is_empty() {
        let children = parse_into_container(m, doc, call);
        let mut i = 0;
        let count = children.len();
        let mut iter = children.into_iter().enumerate().peekable();
        let mut rest = Vec::new();
        while let Some((k, child)) = iter.next() {
            i = k;
            match child {
                Node::Element(e) if matches!(e.kind, Kind::Target | Kind::Pending) => {
                    figure.children.push(e.into());
                }
                Node::Element(e) if e.kind == Kind::Paragraph => {
                    let mut caption = Element::new(Kind::Caption);
                    caption.span = e.span;
                    caption.children = e.children;
                    figure.children.push(caption.into());
                    rest = iter.map(|(_, n)| n).collect();
                    break;
                }
                Node::Element(e) if e.kind == Kind::Comment && e.children.is_empty() => {
                    rest = iter.map(|(_, n)| n).collect();
                    break;
                }
                _ => {
                    let error = doc.message(
                        ERROR,
                        "Figure caption must be a paragraph or empty comment.",
                        vec![fixed(Kind::LiteralBlock, &call.block_text, call.span).into()],
                        At::Top(Some(call.lineno)),
                    );
                    return Ok(vec![figure.into(), error.into()]);
                }
            }
        }
        if count > i + 1 && !rest.is_empty() {
            figure
                .children
                .push(Element::new(Kind::Legend).with_children(rest).into());
        }
    }
    Ok(vec![figure.into()])
}

fn raw(_: &mut Machine, doc: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    let name = call.name.clone();
    let (file, url) = (call.has("file"), call.has("url"));
    if doc.extensions.loader.is_none() && (file || url) {
        return Err(failure(WARNING, format!("\"{name}\" directive disabled.")));
    }
    let format = split_whitespace(&call.arguments[0].to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    let mut source = None;
    let text = if !call.content.is_empty() {
        if file || url {
            return Err(failure(
                ERROR,
                format!(
                    "\"{name}\" directive may not both specify an external file and have content."
                ),
            ));
        }
        call.content_text()
    } else if let Some(OptValue::Str(path)) = call.option("file").cloned() {
        if url {
            return Err(failure(
                ERROR,
                format!(
                    "The \"file\" and \"url\" options may not be simultaneously specified for the \"{name}\" directive."
                ),
            ));
        }
        let path = super::include::resolve(doc, &path);
        let text = super::include::read_file(doc, &name, &path, call)?;
        source = Some(path);
        text
    } else if let Some(OptValue::Str(url)) = call.option("url").cloned() {
        let text = super::include::read_url(doc, &url, call).map_err(|e| {
            failure(
                ERROR,
                format!("Problems with \"{name}\" directive URL \"{url}\":\n{e}."),
            )
        })?;
        let text =
            text.map_err(|e| failure(ERROR, format!("Problem with \"{name}\" directive:\n{e}")))?;
        source = Some(url);
        text
    } else {
        assert_has_content(call)?;
        String::new()
    };
    let mut node = fixed(Kind::Raw, &text, call.span);
    set_classes(&mut node, call);
    node.attributes.set("format", format);
    if let Some(source) = source {
        node.attributes.set("source", source);
    }
    Ok(vec![node.into()])
}

fn include(m: &mut Machine, doc: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    super::include::include(m, doc, call)
}

fn replace(m: &mut Machine, doc: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    not_in_substitution(call)?;
    assert_has_content(call)?;
    let children = parse_into_container(m, doc, call);
    let mut paragraph: Option<Element> = None;
    let mut messages = Vec::new();
    for child in children {
        match child {
            Node::Element(e) if paragraph.is_none() && e.kind == Kind::Paragraph => {
                paragraph = Some(e)
            }
            Node::Element(mut e) if e.kind == Kind::SystemMessage => {
                e.attributes.set("backrefs", Vec::<String>::new());
                messages.push(Node::from(e));
            }
            _ => {
                let error = doc.message(
                    ERROR,
                    &format!(
                        "Error in \"{}\" directive: may contain a single paragraph only.",
                        call.name
                    ),
                    Vec::new(),
                    At::Top(Some(call.lineno)),
                );
                return Ok(vec![error.into()]);
            }
        }
    }
    if let Some(p) = paragraph {
        messages.extend(p.children);
    }
    Ok(messages)
}

fn unicode(_: &mut Machine, doc: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    not_in_substitution(call)?;
    let top = doc.top();
    if call.has("trim") {
        top.attributes.set("ltrim", 1i64);
        top.attributes.set("rtrim", 1i64);
    }
    if call.has("ltrim") {
        top.attributes.set("ltrim", 1i64);
    }
    if call.has("rtrim") {
        top.attributes.set("rtrim", 1i64);
    }
    // Text after ".. " is a comment.
    let arg = &call.arguments[0];
    let chars: Vec<char> = arg.chars().collect();
    let mut cut = chars.len();
    for i in 0..chars.len() {
        if chars[i..].starts_with(&['.', '.', ' '])
            && (i == 0 || chars[i - 1] == ' ' || chars[i - 1] == '\n')
        {
            cut = if i == 0 { 0 } else { i - 1 };
            break;
        }
    }
    let codes: String = chars[..cut].iter().collect();
    let mut out = Vec::new();
    for code in split_whitespace(&codes) {
        let decoded = unicode_code(code)
            .map_err(|e| failure(ERROR, format!("Invalid character code: {code}\n{e}")))?;
        out.push(Text::new(decoded, None).into());
    }
    Ok(out)
}

fn class(m: &mut Machine, doc: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    let classes = class_option(Some(&call.arguments[0])).map_err(|_| {
        failure(
            ERROR,
            format!(
                "Invalid class attribute value for \"{}\" directive: \"{}\".",
                call.name, call.arguments[0]
            ),
        )
    })?;
    if !call.content.is_empty() {
        let mut nodes = parse_into_container(m, doc, call);
        for node in &mut nodes {
            if let Node::Element(e) = node {
                for c in &classes {
                    e.attributes.push("classes", c.clone());
                }
            }
        }
        return Ok(nodes);
    }
    let mut p = pending(
        "docutils.transforms.misc.ClassAttribute",
        vec![
            ("class".into(), AttrValue::List(classes)),
            ("directive".into(), AttrValue::Str(call.name.clone())),
        ],
    );
    p.span = call.span;
    p.attributes.set(RAW, call.block_text.clone());
    if let Some(line) = doc.current_line
        && doc.in_document()
    {
        p.attributes.set(LINE, line as i64);
    }
    doc.note_pending(&mut p, 210);
    Ok(vec![p.into()])
}

fn role(m: &mut Machine, doc: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    if call.content_offset > call.lineno as isize || call.content.is_empty() {
        return Err(failure(
            ERROR,
            format!(
                "\"{}\" directive requires arguments on the first line.",
                call.name
            ),
        ));
    }
    let args = call.content[0].text.clone();
    let Some((new_name, base_name)) = parse_role_arguments(&args) else {
        return Err(failure(
            ERROR,
            format!(
                "\"{}\" directive arguments not valid role names: \"{args}\".",
                call.name
            ),
        ));
    };
    let literal = || -> Node { fixed(Kind::LiteralBlock, &call.block_text, call.span).into() };
    let mut messages: Vec<Node> = Vec::new();
    let base = match &base_name {
        Some(base) => {
            let (found, msgs) = crate::parser::roles::lookup(doc, base, call.lineno);
            messages.extend(msgs.into_iter().map(Node::from));
            match found {
                Some(b) => b,
                None => {
                    let error = doc.message(
                        ERROR,
                        &format!("Unknown interpreted text role \"{base}\"."),
                        vec![literal()],
                        At::Top(Some(call.lineno)),
                    );
                    messages.push(error.into());
                    return Ok(messages);
                }
            }
        }
        None => crate::parser::roles::Role::generic_custom(),
    };
    let spec = base.spec();
    let parsed = m.parse_directive_block(
        doc,
        call.content[1..].to_vec(),
        call.content_offset,
        &spec,
        Vec::new(),
    );
    let mut options = match parsed {
        Ok((_, options, _, _)) => options,
        Err(super::MarkupError(detail)) => {
            let error = doc.message(
                ERROR,
                &format!("Error in \"{}\" directive:\n{detail}.", call.name),
                vec![literal()],
                At::Top(Some(call.lineno)),
            );
            messages.push(error.into());
            return Ok(messages);
        }
    };
    if !options.iter().any(|(n, _)| n == "class") {
        match class_option(Some(&new_name)) {
            Ok(c) => options.push(("class".into(), OptValue::List(c))),
            Err(detail) => {
                let error = doc.message(
                    ERROR,
                    &format!(
                        "Invalid argument for \"{}\" directive:\n{detail}.",
                        call.name
                    ),
                    vec![literal()],
                    At::Top(Some(call.lineno)),
                );
                messages.push(error.into());
                return Ok(messages);
            }
        }
    }
    doc.roles
        .insert(new_name.to_lowercase(), base.customized(options));
    Ok(messages)
}

/// `(simplename)\s*(\(\s*(simplename)\s*\)\s*)?$`
fn parse_role_arguments(args: &str) -> Option<(String, Option<String>)> {
    let s: Vec<char> = args.chars().collect();
    let space = |i: usize| {
        s[i..]
            .iter()
            .take_while(|&&c| crate::util::is_space(c))
            .count()
    };
    for e in crate::patterns::simplename_ends(&s, 0) {
        let name: String = s[..e].iter().collect();
        let mut i = e + space(e);
        if i == s.len() {
            return Some((name, None));
        }
        if s[i] != '(' {
            continue;
        }
        i += 1;
        i += space(i);
        for b in crate::patterns::simplename_ends(&s, i) {
            let mut j = b + space(b);
            if s.get(j) == Some(&')') {
                j += 1;
                j += space(j);
                if j == s.len() {
                    return Some((name, Some(s[i..b].iter().collect())));
                }
            }
        }
    }
    None
}

fn default_role(
    _: &mut Machine,
    doc: &mut Document,
    call: &mut Call,
) -> Result<Vec<Node>, Failure> {
    let Some(name) = call.arguments.first().cloned() else {
        doc.roles.remove("");
        return Ok(Vec::new());
    };
    let (found, messages) = crate::parser::roles::lookup(doc, &name, call.lineno);
    let mut out: Vec<Node> = messages.into_iter().map(Node::from).collect();
    match found {
        Some(role) => {
            doc.roles.insert(String::new(), role);
        }
        None => {
            let error = doc.message(
                ERROR,
                &format!("Unknown interpreted text role \"{name}\"."),
                vec![fixed(Kind::LiteralBlock, &call.block_text, call.span).into()],
                At::Top(Some(call.lineno)),
            );
            out.push(error.into());
        }
    }
    Ok(out)
}

fn title(_: &mut Machine, doc: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    doc.root()
        .attributes
        .set("title", call.arguments[0].clone());
    Ok(Vec::new())
}

fn meta(m: &mut Machine, doc: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    assert_has_content(call)?;
    doc.open(Element::new(Kind::Other("Element".into())), false);
    let (offset, _) = m.nested_list_parse(
        doc,
        (
            super::super::machine::LineList::new(call.content.clone(), None),
            0,
            call.content_offset,
        ),
        State::MetaBody,
        true,
        |_| {},
    );
    let mut node = doc.close().unwrap();
    if offset - call.content_offset != call.content.len() as isize {
        node.children.push(
            doc.message(
                ERROR,
                "Invalid meta directive.",
                vec![fixed(Kind::LiteralBlock, &call.block_text, call.span).into()],
                At::Top(Some(call.lineno)),
            )
            .into(),
        );
    }
    let index = doc
        .root()
        .children
        .iter()
        .position(|c| !matches!(c, Node::Element(e) if matches!(e.kind, Kind::Title | Kind::Subtitle | Kind::Rubric | Kind::Meta)))
        .unwrap_or(0);
    doc.insert_in_root(index, node.children);
    Ok(Vec::new())
}

impl Machine {
    /// A field of a meta directive (docutils' `MetaBody.parsemeta`).
    pub(crate) fn meta_field(
        &mut self,
        doc: &mut Document,
        state: State,
    ) -> crate::parser::machine::Step {
        let end = crate::patterns::field_marker(&self.text().chars().collect::<Vec<_>>()).unwrap();
        let marker: String = self.text().chars().take(end).collect();
        let name = &marker[1..];
        let name = &name[..name.rfind(':').unwrap_or(name.len())];
        let name = crate::util::unescape(&crate::util::escape2null(name));
        let (indented, _, _, _) = self.get_first_known_indented(doc, end, false, true, true);
        let line = self.line().clone();
        if indented.is_empty() {
            let msg = doc.message(
                INFO,
                &format!("No content for meta tag \"{name}\"."),
                vec![fixed(Kind::LiteralBlock, &line.text, Some(line.span)).into()],
                At::Top(None),
            );
            doc.append(msg);
            return Ok(state);
        }
        let texts: Vec<&str> = indented.iter().map(|l| l.text.as_str()).collect();
        let mut node = Element::new(Kind::Meta);
        node.attributes.set(
            "content",
            crate::util::unescape(&crate::util::escape2null(&texts.join(" "))),
        );
        let tokens: Vec<&str> = split_whitespace(&name).collect();
        match extract_name_value(tokens.first().copied().unwrap_or("")) {
            Ok(pairs) if !pairs.is_empty() => {
                let (a, v) = &pairs[0];
                node.attributes.set(a.to_lowercase(), v.clone());
            }
            _ => node
                .attributes
                .set("name", tokens.first().copied().unwrap_or("")),
        }
        for token in tokens.iter().skip(1) {
            match extract_name_value(token) {
                Ok(pairs) => {
                    if let Some((a, v)) = pairs.into_iter().next() {
                        node.attributes.set(a.to_lowercase(), v);
                    }
                }
                Err(detail) => {
                    let msg = doc.message(
                        ERROR,
                        &format!("Error parsing meta tag attribute \"{token}\": {detail}."),
                        vec![fixed(Kind::LiteralBlock, &line.text, Some(line.span)).into()],
                        At::Top(None),
                    );
                    doc.append(msg);
                    return Ok(state);
                }
            }
        }
        doc.append(node);
        Ok(state)
    }
}

/// docutils' `extract_name_value`: `name=value` pairs.
fn extract_name_value(line: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    let mut line = line.to_owned();
    while !line.is_empty() {
        let Some(eq) = line.find('=') else {
            return Err("missing \"=\"".to_owned());
        };
        let name = strip(&line[..eq]).to_owned();
        if eq == 0 || name.is_empty() {
            return Err("missing attribute name before \"=\"".to_owned());
        }
        line = crate::util::lstrip(&line[eq + 1..]).to_owned();
        if line.is_empty() {
            return Err(format!("missing value after \"{name}=\""));
        }
        let first = line.chars().next().unwrap();
        let data;
        if first == '\'' || first == '"' {
            let Some(end) = line[1..].find(first).map(|i| i + 1) else {
                return Err(format!("attribute \"{name}\" missing end quote ({first})"));
            };
            if line[end + 1..]
                .chars()
                .next()
                .is_some_and(|c| !strip(&c.to_string()).is_empty())
            {
                return Err(format!(
                    "attribute \"{name}\" end quote ({first}) not followed by whitespace"
                ));
            }
            data = line[1..end].to_owned();
            line = crate::util::lstrip(&line[end + 1..]).to_owned();
        } else {
            match line.find(' ') {
                None => {
                    data = line.clone();
                    line.clear();
                }
                Some(sp) => {
                    data = line[..sp].to_owned();
                    line = crate::util::lstrip(&line[sp + 1..]).to_owned();
                }
            }
        }
        out.push((name.to_lowercase(), data));
    }
    Ok(out)
}

fn date(_: &mut Machine, _: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    not_in_substitution(call)?;
    let format = call.content_text();
    let format = if format.is_empty() {
        "%Y-%m-%d".to_owned()
    } else {
        format
    };
    Ok(vec![Text::new(crate::util::strftime(&format), None).into()])
}

fn contents(_: &mut Machine, doc: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    if invalid_parent(&doc.top().kind) {
        return Err(failure(
            ERROR,
            format!(
                "The \"{}\" directive may not be used within topics or body elements.",
                call.name
            ),
        ));
    }
    let mut messages = Vec::new();
    let title = if let Some(text) = call.arguments.first().cloned() {
        let (nodes, msgs) = inline(doc, call, &text);
        messages = msgs;
        Some(Element::new(Kind::Title).with_children(nodes))
    } else if call.has("local") {
        None
    } else {
        Some(Element::new(Kind::Title).with_child(Text::new("Contents", None)))
    };
    let mut topic = Element::new(Kind::Topic);
    topic.span = call.span;
    topic.attributes.push("classes", "contents");
    for c in call.classes() {
        topic.attributes.push("classes", c);
    }
    if call.has("local") {
        topic.attributes.push("classes", "local");
    }
    let name = match &title {
        Some(t) => astext(t),
        None => "Contents".to_owned(),
    };
    if let Some(t) = title {
        topic.children.push(t.into());
    }
    let name = normalize_name(&name);
    if !doc.has_name(&name) {
        topic.attributes.push("names", name);
    }
    let line = doc.current_line.map(|l| l.saturating_sub(1));
    doc.note_implicit_target(&mut topic, None, line);
    let details = call
        .options
        .iter()
        .map(|(k, v)| (k.clone(), detail(v)))
        .collect();
    let mut p = pending("docutils.transforms.parts.Contents", details);
    p.span = call.span;
    doc.note_pending(&mut p, 720);
    topic.children.push(p.into());
    let mut out = vec![Node::from(topic)];
    out.extend(messages.into_iter().map(Node::from));
    Ok(out)
}

fn sectnum(_: &mut Machine, doc: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    let details = call
        .options
        .iter()
        .map(|(k, v)| (k.clone(), detail(v)))
        .collect();
    let mut p = pending("docutils.transforms.parts.SectNum", details);
    p.span = call.span;
    doc.note_pending(&mut p, 710);
    Ok(vec![p.into()])
}

fn decoration(m: &mut Machine, doc: &mut Document, call: &mut Call) -> Result<Vec<Node>, Failure> {
    assert_has_content(call)?;
    let kind = if call.name.eq_ignore_ascii_case("header") {
        Kind::Header
    } else {
        Kind::Footer
    };
    // Parsed in a header or footer element, which may hold messages.
    let content = parse_content(m, doc, call, Element::new(kind.clone())).children;
    doc.add_to_decoration(kind, content);
    Ok(Vec::new())
}

fn target_notes(
    _: &mut Machine,
    doc: &mut Document,
    call: &mut Call,
) -> Result<Vec<Node>, Failure> {
    let mut p = pending("docutils.transforms.references.TargetNotes", Vec::new());
    p.span = call.span;
    add_name(doc, call, &mut p);
    for (k, v) in &call.options {
        p.attributes.set(format!(".details.{k}"), detail(v));
    }
    doc.note_pending(&mut p, 540);
    Ok(vec![p.into()])
}

fn test_directive(
    _: &mut Machine,
    doc: &mut Document,
    call: &mut Call,
) -> Result<Vec<Node>, Failure> {
    let arguments: Vec<String> = call.arguments.iter().map(|a| py_repr(a)).collect();
    let options: Vec<String> = call
        .options
        .iter()
        .map(|(k, v)| {
            let value = match v {
                OptValue::Str(s) => py_repr(s),
                OptValue::Int(n) => n.to_string(),
                OptValue::None => "None".to_owned(),
                OptValue::List(l) => format!(
                    "[{}]",
                    l.iter().map(|s| py_repr(s)).collect::<Vec<_>>().join(", ")
                ),
                OptValue::IntList(l) => format!(
                    "[{}]",
                    l.iter().map(i64::to_string).collect::<Vec<_>>().join(", ")
                ),
            };
            format!("{}: {value}", py_repr(k))
        })
        .collect();
    let head = format!(
        "Directive processed. Type=\"{}\", arguments=[{}], options={{{}}}, content:",
        call.name,
        arguments.join(", "),
        options.join(", ")
    );
    let info = if call.content.is_empty() {
        doc.message(
            INFO,
            &format!("{head} None"),
            Vec::new(),
            At::Top(Some(call.lineno)),
        )
    } else {
        let text = call.content_text();
        doc.message(
            INFO,
            &head,
            vec![fixed(Kind::LiteralBlock, &text, call.span).into()],
            At::Top(Some(call.lineno)),
        )
    };
    Ok(vec![info.into()])
}
