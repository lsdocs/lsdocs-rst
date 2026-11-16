//! docutils' tree (from `lsdocs-rst`) to the shared document shape.

use std::collections::{HashMap, HashSet};

use lsdocs_plugin_sdk::doctree::{
    self as dt, Admonition, BlockQuote, Caption, CodeBlock, Custom, DefinitionDescription,
    DefinitionList, DefinitionTerm, Document, Emphasis, Figure, FootnoteDefinition,
    FootnoteReference, Heading, Image, InlineCode, LineBreak, Link, LinkTarget, List, ListItem,
    Math, Metadata, Paragraph, Raw, Section, Strong, Table, TableCell, TableRow, ThematicBreak,
    Value,
};
use lsdocs_plugin_sdk::protocol::{Diagnostic, Severity};
use lsdocs_rst::nodes::{AttrValue, Element, Kind, Node, Span};
use serde_json::json;

/// Converts docutils' `tree` for the file `source` (whose text is `text`),
/// with the messages the transforms reported outside the tree, into a
/// document and diagnostics.
pub fn convert(
    source: &str,
    text: &str,
    tree: &Element,
    messages: &[Element],
) -> (Document, Vec<Diagnostic>) {
    let mut cx = Cx {
        source,
        lines: lsdocs_rst::lines::lines(text)
            .into_iter()
            .map(|l| l.span)
            .collect(),
        refids: HashSet::new(),
        spans: HashMap::new(),
        metadata: Metadata::new(),
        diagnostics: Vec::new(),
    };
    cx.scan(tree);
    let mut doc = Document::new(source);
    doc.children = cx.blocks(&tree.children, 0);
    for msg in messages {
        cx.message(msg);
    }
    doc.metadata = std::mem::take(&mut cx.metadata);
    (doc, cx.diagnostics)
}

struct Cx<'a> {
    source: &'a str,
    /// The span of each input line.
    lines: Vec<Span>,
    /// Ids references point to.
    refids: HashSet<String>,
    /// The span of each element with an id.
    spans: HashMap<String, Span>,
    metadata: Metadata,
    diagnostics: Vec<Diagnostic>,
}

fn span(s: Option<Span>) -> Option<dt::Span> {
    let clamp = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    s.map(|s| dt::Span::new(clamp(s.start), clamp(s.end)))
}

fn ids(e: &Element) -> &[String] {
    e.attributes.get_list("ids")
}

fn classes(e: &Element) -> &[String] {
    e.attributes.get_list("classes")
}

fn attr<'e>(e: &'e Element, name: &str) -> Option<&'e str> {
    e.attributes.get_str(name)
}

/// The child elements of `e` of `kind`.
fn children_of<'e>(e: &'e Element, kind: &'e Kind) -> impl Iterator<Item = &'e Element> {
    e.elements().filter(move |c| &c.kind == kind)
}

/// Ids an author gave `e` (labels, `:name:` options), not those docutils
/// made up.
fn explicit_ids(e: &Element) -> Vec<String> {
    match e.kind {
        // The first id comes from the title.
        Kind::Section => ids(e).iter().skip(1).cloned().collect(),
        Kind::Footnote
        | Kind::Citation
        | Kind::FootnoteReference
        | Kind::CitationReference
        | Kind::Problematic
        | Kind::SystemMessage => Vec::new(),
        _ => {
            // docutils' own ids are `<tag>-<number>`.
            let prefix = format!("{}-", e.kind.tag());
            ids(e)
                .iter()
                .filter(|id| {
                    !id.strip_prefix(&prefix)
                        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                })
                .cloned()
                .collect()
        }
    }
}

fn anchor(id: &str) -> dt::Node {
    dt::Anchor {
        span: None,
        id: id.to_owned(),
    }
    .into()
}

/// The text of a literal block, without the line numbers of
/// `:number-lines:`.
fn code_text(e: &Element) -> String {
    let mut out = String::new();
    for child in &e.children {
        match child {
            Node::Text(t) => out.push_str(&t.value),
            Node::Element(c) if c.kind == Kind::Inline && classes(c).iter().any(|c| c == "ln") => {}
            Node::Element(c) => out.push_str(&code_text(c)),
        }
    }
    out
}

impl Cx<'_> {
    /// Notes the ids references point to and where each id is.
    fn scan(&mut self, e: &Element) {
        if let Some(id) = attr(e, "refid")
            && e.kind == Kind::Reference
        {
            self.refids.insert(id.to_owned());
        }
        if let Some(s) = e.span {
            for id in ids(e) {
                self.spans.entry(id.clone()).or_insert(s);
            }
        }
        for child in e.elements() {
            self.scan(child);
        }
    }

    // -- diagnostics ------------------------------------------------------

    /// A system message as a diagnostic.
    fn message(&mut self, msg: &Element) {
        let level = match msg.attributes.get("level") {
            Some(AttrValue::Int(n)) => *n,
            _ => 2,
        };
        let severity = match level {
            ..=1 => return,
            2 | 3 => Severity::Warning,
            _ => Severity::Error,
        };
        let text = msg
            .elements()
            .find(|c| c.kind == Kind::Paragraph)
            .map(Element::astext)
            .unwrap_or_default();
        let msg_source = attr(msg, "source");
        let own = msg_source.is_none_or(|s| s == self.source);
        let backref = msg
            .attributes
            .get_list("backrefs")
            .iter()
            .find_map(|id| self.spans.get(id).copied());
        let line = match msg.attributes.get("line") {
            Some(AttrValue::Int(n)) if *n >= 1 => Some(*n as usize),
            _ => None,
        };
        let at = backref.or_else(|| {
            line.filter(|_| own)
                .and_then(|l| self.lines.get(l - 1).copied())
        });
        self.diagnostics.push(Diagnostic {
            severity,
            message: text,
            source: Some(msg_source.unwrap_or(self.source).to_owned()),
            span: if own || backref.is_some() {
                span(at)
            } else {
                None
            },
            code: Some("rst".into()),
        });
    }

    // -- blocks -----------------------------------------------------------

    fn blocks(&mut self, nodes: &[Node], depth: u8) -> Vec<dt::Node> {
        let mut out = Vec::new();
        for node in nodes {
            if let Node::Element(e) = node {
                self.block(e, depth, &mut out);
            }
        }
        out
    }

    /// The blocks of `e`'s children, but its first `title` as a bold
    /// paragraph (topics, sidebars).
    fn titled_blocks(&mut self, e: &Element, depth: u8) -> Vec<dt::Node> {
        let mut out = Vec::new();
        for child in e.elements() {
            if matches!(child.kind, Kind::Title | Kind::Subtitle) {
                out.push(
                    Paragraph {
                        span: span(child.span),
                        children: vec![
                            Strong {
                                span: span(child.span),
                                children: self.inlines(&child.children),
                            }
                            .into(),
                        ],
                    }
                    .into(),
                );
            } else {
                self.block(child, depth, &mut out);
            }
        }
        out
    }

    fn custom(&mut self, e: &Element, kind: &str, children: Vec<dt::Node>) -> dt::Node {
        let mut attrs = std::collections::BTreeMap::new();
        let classes = classes(e);
        if !classes.is_empty() {
            attrs.insert("classes".into(), json!(classes));
        }
        Custom {
            span: span(e.span),
            kind: kind.into(),
            attrs,
            children,
        }
        .into()
    }

    fn block(&mut self, e: &Element, depth: u8, out: &mut Vec<dt::Node>) {
        if !matches!(e.kind, Kind::Section | Kind::Figure) {
            out.extend(explicit_ids(e).iter().map(|id| anchor(id)));
        }
        let s = span(e.span);
        let node: dt::Node = match &e.kind {
            Kind::Section => self.section(e, depth),
            Kind::Paragraph => Paragraph {
                span: s,
                children: self.inlines(&e.children),
            }
            .into(),
            Kind::BulletList | Kind::EnumeratedList => {
                let ordered = e.kind == Kind::EnumeratedList;
                let start = match e.attributes.get("start") {
                    Some(AttrValue::Int(n)) if *n != 1 => u64::try_from(*n).ok(),
                    _ => None,
                };
                let items = children_of(e, &Kind::ListItem)
                    .map(|item| {
                        ListItem {
                            span: span(item.span),
                            checked: None,
                            children: self.blocks(&item.children, depth),
                        }
                        .into()
                    })
                    .collect();
                List {
                    span: s,
                    ordered,
                    start,
                    children: items,
                }
                .into()
            }
            Kind::DefinitionList => self.definition_list(e, depth),
            Kind::FieldList => {
                let mut children = Vec::new();
                for field in children_of(e, &Kind::Field) {
                    for part in field.elements() {
                        match part.kind {
                            Kind::FieldName => children.push(
                                DefinitionTerm {
                                    span: span(part.span),
                                    children: self.inlines(&part.children),
                                }
                                .into(),
                            ),
                            Kind::FieldBody => children.push(
                                DefinitionDescription {
                                    span: span(part.span),
                                    children: self.blocks(&part.children, depth),
                                }
                                .into(),
                            ),
                            _ => {}
                        }
                    }
                }
                DefinitionList { span: s, children }.into()
            }
            Kind::OptionList => self.option_list(e, depth),
            Kind::LiteralBlock => {
                let language = classes(e)
                    .iter()
                    .position(|c| c == "code")
                    .and_then(|i| classes(e).get(i + 1))
                    .cloned();
                CodeBlock {
                    span: s,
                    language,
                    info: None,
                    value: code_text(e),
                }
                .into()
            }
            Kind::DoctestBlock => CodeBlock {
                span: s,
                language: Some("pycon".into()),
                info: None,
                value: e.astext(),
            }
            .into(),
            Kind::MathBlock => Math {
                span: s,
                value: e.astext(),
                display: true,
            }
            .into(),
            Kind::LineBlock => {
                let mut lines = Vec::new();
                line_block(e, 0, &mut lines);
                let mut children = Vec::new();
                for (i, (level, line)) in lines.into_iter().enumerate() {
                    if i > 0 {
                        children.push(LineBreak { span: None }.into());
                    }
                    if level > 0 {
                        children.push(dt::text("\u{a0}".repeat(4 * level)));
                    }
                    children.extend(self.inlines(&line.children));
                }
                Paragraph { span: s, children }.into()
            }
            Kind::BlockQuote => {
                let mut children = Vec::new();
                for child in e.elements() {
                    if child.kind == Kind::Attribution {
                        let mut inlines = vec![dt::text("\u{2014} ")];
                        inlines.extend(self.inlines(&child.children));
                        children.push(
                            Paragraph {
                                span: span(child.span),
                                children: inlines,
                            }
                            .into(),
                        );
                    } else {
                        self.block(child, depth, &mut children);
                    }
                }
                BlockQuote { span: s, children }.into()
            }
            Kind::Attention
            | Kind::Caution
            | Kind::Danger
            | Kind::Error
            | Kind::Hint
            | Kind::Important
            | Kind::Note
            | Kind::Tip
            | Kind::Warning => Admonition {
                span: s,
                kind: e.kind.tag().to_owned(),
                title: None,
                children: self.blocks(&e.children, depth),
            }
            .into(),
            Kind::Admonition => {
                let title = children_of(e, &Kind::Title).next().map(Element::astext);
                let rest: Vec<Node> = e
                    .children
                    .iter()
                    .filter(|n| !matches!(n, Node::Element(c) if c.kind == Kind::Title))
                    .cloned()
                    .collect();
                Admonition {
                    span: s,
                    kind: "admonition".into(),
                    title,
                    children: self.blocks(&rest, depth),
                }
                .into()
            }
            Kind::Topic => {
                let kind = ["contents", "dedication", "abstract"]
                    .into_iter()
                    .find(|k| classes(e).iter().any(|c| c == k))
                    .unwrap_or("topic");
                let children = self.titled_blocks(e, depth);
                self.custom(e, kind, children)
            }
            Kind::Sidebar => {
                let children = self.titled_blocks(e, depth);
                self.custom(e, "sidebar", children)
            }
            Kind::Rubric => {
                let children = self.inlines(&e.children);
                self.custom(e, "rubric", children)
            }
            Kind::Container => {
                let children = self.blocks(&e.children, depth);
                self.custom(e, "container", children)
            }
            Kind::Legend => {
                let children = self.blocks(&e.children, depth);
                self.custom(e, "legend", children)
            }
            Kind::Decoration => {
                for part in e.elements() {
                    let children = self.blocks(&part.children, depth);
                    let node = self.custom(part, part.kind.tag(), children);
                    out.push(node);
                }
                return;
            }
            Kind::Compound => {
                out.extend(self.blocks(&e.children, depth));
                return;
            }
            Kind::Figure => self.figure(e, depth, out),
            Kind::Table => self.table(e, depth),
            Kind::Image => Paragraph {
                span: s,
                children: vec![image(e)],
            }
            .into(),
            Kind::Reference => Paragraph {
                span: s,
                children: self.inlines(std::slice::from_ref(&Node::Element(e.clone()))),
            }
            .into(),
            Kind::Footnote | Kind::Citation => {
                let label = children_of(e, &Kind::Label)
                    .next()
                    .map(Element::astext)
                    .unwrap_or_default();
                let rest: Vec<Node> = e
                    .children
                    .iter()
                    .filter(|n| !matches!(n, Node::Element(c) if c.kind == Kind::Label))
                    .cloned()
                    .collect();
                FootnoteDefinition {
                    span: s,
                    label,
                    children: self.blocks(&rest, depth),
                }
                .into()
            }
            Kind::Raw => Raw {
                span: s,
                format: attr(e, "format").unwrap_or_default().to_owned(),
                value: e.astext(),
            }
            .into(),
            Kind::Transition => ThematicBreak { span: s }.into(),
            Kind::Docinfo => {
                self.docinfo(e);
                return;
            }
            Kind::Meta => {
                if let (Some(name), Some(content)) = (attr(e, "name"), attr(e, "content")) {
                    let meta = self
                        .metadata
                        .entry("meta".into())
                        .or_insert_with(|| json!({}));
                    if let Some(map) = meta.as_object_mut() {
                        map.insert(name.to_owned(), json!(content));
                    }
                }
                return;
            }
            Kind::SystemMessage => {
                self.message(e);
                return;
            }
            Kind::Other(name) if name == "directive" => self.directive(e),
            Kind::Other(name) if name == super::DIRECTIVE => self.declared(e, depth),
            // Invisible, or handled by their parents.
            Kind::Comment
            | Kind::SubstitutionDefinition
            | Kind::Target
            | Kind::Pending
            | Kind::Title
            | Kind::Subtitle => return,
            _ => {
                let children = self.blocks(&e.children, depth);
                self.custom(e, e.kind.tag(), children)
            }
        };
        out.push(node);
    }

    fn section(&mut self, e: &Element, depth: u8) -> dt::Node {
        let depth = depth.saturating_add(1);
        let mut explicit = explicit_ids(e);
        // docutils' id from the title, if something links to it.
        if let Some(own) = ids(e).first()
            && self.refids.contains(own)
        {
            explicit.push(own.clone());
        }
        let mut children = Vec::new();
        if let Some(title) = children_of(e, &Kind::Title).next() {
            children.push(
                Heading {
                    span: span(title.span),
                    level: depth.min(6),
                    id: None,
                    children: self.inlines(&title.children),
                }
                .into(),
            );
        }
        let id = (!explicit.is_empty()).then(|| explicit.remove(0));
        children.extend(explicit.iter().map(|id| anchor(id)));
        for child in e.elements() {
            if child.kind != Kind::Title {
                self.block(child, depth, &mut children);
            }
        }
        Section {
            span: span(e.span),
            id,
            children,
        }
        .into()
    }

    fn definition_list(&mut self, e: &Element, depth: u8) -> dt::Node {
        let mut children = Vec::new();
        for item in children_of(e, &Kind::DefinitionListItem) {
            for part in item.elements() {
                match part.kind {
                    Kind::Term => {
                        let mut inlines = self.inlines(&part.children);
                        // Classifiers follow their term.
                        for classifier in children_of(item, &Kind::Classifier) {
                            inlines.push(dt::text(" : "));
                            inlines.extend(self.inlines(&classifier.children));
                        }
                        children.push(
                            DefinitionTerm {
                                span: span(part.span),
                                children: inlines,
                            }
                            .into(),
                        );
                    }
                    Kind::Definition => children.push(
                        DefinitionDescription {
                            span: span(part.span),
                            children: self.blocks(&part.children, depth),
                        }
                        .into(),
                    ),
                    _ => {}
                }
            }
        }
        DefinitionList {
            span: span(e.span),
            children,
        }
        .into()
    }

    fn option_list(&mut self, e: &Element, depth: u8) -> dt::Node {
        let mut children = Vec::new();
        for item in children_of(e, &Kind::OptionListItem) {
            for part in item.elements() {
                match part.kind {
                    Kind::OptionGroup => {
                        let mut inlines = Vec::new();
                        for (i, option) in children_of(part, &Kind::Option).enumerate() {
                            if i > 0 {
                                inlines.push(dt::text(", "));
                            }
                            let mut value = String::new();
                            for piece in option.elements() {
                                if piece.kind == Kind::OptionArgument {
                                    value.push_str(attr(piece, "delimiter").unwrap_or(" "));
                                }
                                value.push_str(&piece.astext());
                            }
                            inlines.push(
                                InlineCode {
                                    span: span(option.span),
                                    value,
                                }
                                .into(),
                            );
                        }
                        children.push(
                            DefinitionTerm {
                                span: span(part.span),
                                children: inlines,
                            }
                            .into(),
                        );
                    }
                    Kind::Description => children.push(
                        DefinitionDescription {
                            span: span(part.span),
                            children: self.blocks(&part.children, depth),
                        }
                        .into(),
                    ),
                    _ => {}
                }
            }
        }
        DefinitionList {
            span: span(e.span),
            children,
        }
        .into()
    }

    fn figure(&mut self, e: &Element, depth: u8, out: &mut Vec<dt::Node>) -> dt::Node {
        // A figure's `:name:` is its image's.
        let mut ids = explicit_ids(e);
        for image in children_of(e, &Kind::Image) {
            ids.extend(explicit_ids(image));
        }
        let mut ids = ids.into_iter();
        let id = ids.next();
        out.extend(ids.map(|id| anchor(&id)));
        let mut children = Vec::new();
        let mut caption = None;
        for child in e.elements() {
            match child.kind {
                Kind::Image => children.push(image(child)),
                Kind::Reference => children.extend(self.inline(child)),
                Kind::Caption => {
                    caption = Some(
                        Caption {
                            span: span(child.span),
                            children: self.inlines(&child.children),
                        }
                        .into(),
                    );
                }
                _ => self.block(child, depth, &mut children),
            }
        }
        children.extend(caption);
        Figure {
            span: span(e.span),
            id,
            children,
        }
        .into()
    }

    fn table(&mut self, e: &Element, depth: u8) -> dt::Node {
        let mut rows = Vec::new();
        for tgroup in children_of(e, &Kind::Tgroup) {
            for part in tgroup.elements() {
                let header = match part.kind {
                    Kind::Thead => true,
                    Kind::Tbody => false,
                    _ => continue,
                };
                for row in children_of(part, &Kind::Row) {
                    let cells = children_of(row, &Kind::Entry)
                        .map(|entry| {
                            // A lone paragraph is the cell's text.
                            let children = match entry.children.as_slice() {
                                [Node::Element(p)] if p.kind == Kind::Paragraph => {
                                    self.inlines(&p.children)
                                }
                                nodes => self.blocks(nodes, depth),
                            };
                            TableCell {
                                span: span(entry.span),
                                children,
                            }
                            .into()
                        })
                        .collect();
                    rows.push(
                        TableRow {
                            span: span(row.span),
                            header,
                            children: cells,
                        }
                        .into(),
                    );
                }
            }
        }
        let table: dt::Node = Table {
            span: span(e.span),
            align: Vec::new(),
            children: rows,
        }
        .into();
        match children_of(e, &Kind::Title).next() {
            Some(title) => Figure {
                span: span(e.span),
                id: None,
                children: vec![
                    table,
                    Caption {
                        span: span(title.span),
                        children: self.inlines(&title.children),
                    }
                    .into(),
                ],
            }
            .into(),
            None => table,
        }
    }

    /// A directive with no implementation: a custom node of its name.
    fn directive(&mut self, e: &Element) -> dt::Node {
        let mut attrs = std::collections::BTreeMap::new();
        if let Some(argument) = attr(e, "argument") {
            attrs.insert("argument".into(), json!(argument));
        }
        let mut options = serde_json::Map::new();
        for child in e.elements() {
            match child.kind {
                Kind::FieldList => {
                    for field in children_of(child, &Kind::Field) {
                        let mut parts = field.elements();
                        if let (Some(name), Some(body)) = (parts.next(), parts.next()) {
                            options.insert(name.astext(), json!(body.astext()));
                        }
                    }
                }
                Kind::LiteralBlock => {
                    attrs.insert("content".into(), json!(child.astext()));
                }
                _ => {}
            }
        }
        if !options.is_empty() {
            attrs.insert("options".into(), Value::Object(options));
        }
        Custom {
            span: span(e.span),
            kind: attr(e, "name").unwrap_or("directive").to_lowercase(),
            attrs,
            children: Vec::new(),
        }
        .into()
    }

    /// A directive a plugin declared: a custom node of its name, with the
    /// attributes `Declared` made, an inline argument as a first
    /// `directive_argument` child, and the content parsed.
    fn declared(&mut self, e: &Element, depth: u8) -> dt::Node {
        let attrs = attr(e, "attrs")
            .and_then(|a| serde_json::from_str(a).ok())
            .unwrap_or_default();
        let mut children = Vec::new();
        for child in &e.children {
            match child {
                Node::Element(c) if c.kind == Kind::Other(super::ARGUMENT.into()) => {
                    children.push(
                        Custom {
                            span: span(c.span),
                            kind: "directive_argument".into(),
                            attrs: std::collections::BTreeMap::new(),
                            children: self.inlines(&c.children),
                        }
                        .into(),
                    );
                }
                Node::Element(c) => self.block(c, depth, &mut children),
                Node::Text(_) => {}
            }
        }
        Custom {
            span: span(e.span),
            kind: attr(e, "name").unwrap_or("directive").to_owned(),
            attrs,
            children,
        }
        .into()
    }

    /// Bibliographic fields as metadata.
    fn docinfo(&mut self, e: &Element) {
        for item in e.elements() {
            match &item.kind {
                Kind::Authors => {
                    let authors: Vec<String> = item.elements().map(Element::astext).collect();
                    self.metadata.insert("authors".into(), json!(authors));
                }
                Kind::Field => {
                    let mut parts = item.elements();
                    if let (Some(name), Some(body)) = (parts.next(), parts.next()) {
                        self.metadata
                            .insert(name.astext().to_lowercase(), field_value(&body.astext()));
                    }
                    for msg in children_of(item, &Kind::FieldBody)
                        .flat_map(|b| children_of(b, &Kind::SystemMessage))
                    {
                        self.message(msg);
                    }
                }
                kind => {
                    self.metadata
                        .insert(kind.tag().to_owned(), json!(item.astext()));
                }
            }
        }
    }

    // -- inlines ----------------------------------------------------------

    fn inlines(&mut self, nodes: &[Node]) -> Vec<dt::Node> {
        let mut out: Vec<dt::Node> = Vec::new();
        for node in nodes {
            let new = match node {
                Node::Text(t) => vec![
                    dt::Text {
                        span: span(t.span),
                        value: t.value.clone(),
                    }
                    .into(),
                ],
                Node::Element(e) => self.inline(e),
            };
            for n in new {
                // Adjacent text merges.
                if let (dt::Node::Text(new), Some(dt::Node::Text(prev))) = (&n, out.last_mut()) {
                    prev.value.push_str(&new.value);
                    prev.span = match (prev.span, new.span) {
                        (Some(a), Some(b)) => {
                            Some(dt::Span::new(a.start.min(b.start), a.end.max(b.end)))
                        }
                        _ => None,
                    };
                    continue;
                }
                out.push(n);
            }
        }
        out
    }

    fn inline(&mut self, e: &Element) -> Vec<dt::Node> {
        let s = span(e.span);
        let node: dt::Node = match &e.kind {
            Kind::Emphasis | Kind::TitleReference => Emphasis {
                span: s,
                children: self.inlines(&e.children),
            }
            .into(),
            Kind::Strong => Strong {
                span: s,
                children: self.inlines(&e.children),
            }
            .into(),
            Kind::Literal => InlineCode {
                span: s,
                value: e.astext(),
            }
            .into(),
            Kind::Reference => {
                let (url, target) = if let Some(uri) = attr(e, "refuri") {
                    (uri.to_owned(), None)
                } else if let Some(id) = attr(e, "refid") {
                    (
                        format!("#{id}"),
                        Some(LinkTarget {
                            anchor: Some(id.to_owned()),
                            ..LinkTarget::default()
                        }),
                    )
                } else {
                    return self.inlines(&e.children);
                };
                Link {
                    span: s,
                    url,
                    title: None,
                    target,
                    children: self.inlines(&e.children),
                }
                .into()
            }
            Kind::Other(kind) if kind == super::ROLE => {
                let mut attrs = std::collections::BTreeMap::new();
                for name in ["name", "text"] {
                    attrs.insert(name.into(), json!(attr(e, name).unwrap_or_default()));
                }
                Custom {
                    span: s,
                    kind: "role".into(),
                    attrs,
                    children: self.inlines(&e.children),
                }
                .into()
            }
            Kind::FootnoteReference | Kind::CitationReference => FootnoteReference {
                span: s,
                label: e.astext(),
            }
            .into(),
            Kind::Image => image(e),
            Kind::Math => Math {
                span: s,
                value: e.astext(),
                display: false,
            }
            .into(),
            Kind::Raw => Raw {
                span: s,
                format: attr(e, "format").unwrap_or_default().to_owned(),
                value: e.astext(),
            }
            .into(),
            Kind::Target => {
                let mut out: Vec<dt::Node> = explicit_ids(e).iter().map(|id| anchor(id)).collect();
                out.extend(self.inlines(&e.children));
                return out;
            }
            Kind::SystemMessage => {
                self.message(e);
                return Vec::new();
            }
            // Anything else: its content.
            _ => return self.inlines(&e.children),
        };
        vec![node]
    }
}

/// A field's text as a metadata value: `true` and `false` as booleans,
/// numbers as numbers, anything else as a string (as in YAML front
/// matter).
fn field_value(text: &str) -> Value {
    let text = text.trim();
    match text {
        "true" => return json!(true),
        "false" => return json!(false),
        _ => {}
    }
    if let Ok(n) = text.parse::<i64>() {
        return json!(n);
    }
    if let Ok(x) = text.parse::<f64>()
        && x.is_finite()
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+'))
    {
        return json!(x);
    }
    json!(text)
}

/// The lines of a line block, with their nesting levels.
fn line_block<'e>(e: &'e Element, level: usize, out: &mut Vec<(usize, &'e Element)>) {
    for child in e.elements() {
        match child.kind {
            Kind::Line => out.push((level, child)),
            Kind::LineBlock => line_block(child, level + 1, out),
            _ => {}
        }
    }
}

fn image(e: &Element) -> dt::Node {
    Image {
        span: span(e.span),
        url: attr(e, "uri").unwrap_or_default().to_owned(),
        alt: attr(e, "alt").unwrap_or_default().to_owned(),
        title: None,
    }
    .into()
}
