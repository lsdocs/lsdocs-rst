//! Explicit markup (`..` blocks and `__` anonymous targets): footnotes,
//! citations, hyperlink targets, substitution definitions, directives and
//! comments.
//!
//! Directives are not run yet: every directive is reported as unknown,
//! as docutils does for a directive it has no implementation for.

use super::document::{At, Document, ERROR, INFO, MsgNode, WARNING, fixed};
use super::machine::{Machine, State, Step, join, span_of};
use crate::lines::Line;
use crate::nodes::{Element, Kind, Node, Span, Text};
use crate::patterns;
use crate::util::{
    char_len, escape2null, normalize_name, rstrip, skip_chars, split_escaped_whitespace,
    split_whitespace, strip, take_chars, unescape, whitespace_normalize_name,
};

/// Nodes produced by an explicit construct, and whether it ended with a
/// blank line.
type Construct = (Vec<Node>, bool);

/// Element kinds that are inline (docutils' `nodes.Inline`), kept inside a
/// substitution definition.
fn is_inline(kind: &Kind) -> bool {
    matches!(
        kind,
        Kind::Emphasis
            | Kind::Strong
            | Kind::Literal
            | Kind::Reference
            | Kind::FootnoteReference
            | Kind::CitationReference
            | Kind::SubstitutionReference
            | Kind::TitleReference
            | Kind::Abbreviation
            | Kind::Acronym
            | Kind::Superscript
            | Kind::Subscript
            | Kind::Math
            | Kind::Image
            | Kind::Inline
            | Kind::Problematic
            | Kind::Generated
            | Kind::Target
            | Kind::Raw
    )
}

/// What a substitution definition may not contain, if `e` is such.
fn disallowed_in_substitution(e: &Element) -> Option<&'static str> {
    let flag = |name: &str| {
        e.attributes
            .get(name)
            .is_some_and(|v| !matches!(v, crate::nodes::AttrValue::Bool(false)))
    };
    if e.kind == Kind::Reference && flag("anonymous") {
        return Some("Anonymous references");
    }
    if e.kind == Kind::FootnoteReference && flag("auto") {
        return Some("References to auto-numbered and auto-symbol footnotes");
    }
    if !e.attributes.get_list("names").is_empty() || !e.attributes.get_list("ids").is_empty() {
        return Some("Targets (names and identifiers)");
    }
    None
}

/// Every element below `e`, depth first.
fn descendants(e: &Element) -> Vec<&Element> {
    let mut out = Vec::new();
    for child in e.elements() {
        out.push(child);
        out.extend(descendants(child));
    }
    out
}

impl Machine {
    /// Explicit markup in Body: the construct, then any that follow.
    pub(crate) fn explicit_markup(&mut self, doc: &mut Document, state: State) -> Step {
        let (nodes, blank_finish) = self.explicit_construct(doc);
        doc.extend(nodes);
        self.explicit_list(doc, blank_finish);
        Ok(state)
    }

    /// A later explicit construct.
    pub(crate) fn explicit_next(&mut self, doc: &mut Document, state: State) -> Step {
        let (nodes, blank_finish) = self.explicit_construct(doc);
        doc.extend(nodes);
        self.blank_finish = blank_finish;
        Ok(state)
    }

    /// An anonymous target (`__ uri`) in Body, then any constructs after.
    pub(crate) fn anonymous(&mut self, doc: &mut Document, state: State) -> Step {
        let (nodes, blank_finish) = self.anonymous_target(doc);
        doc.extend(nodes);
        self.explicit_list(doc, blank_finish);
        Ok(state)
    }

    /// A later anonymous target.
    pub(crate) fn anonymous_next(&mut self, doc: &mut Document, state: State) -> Step {
        let (nodes, blank_finish) = self.anonymous_target(doc);
        doc.extend(nodes);
        self.blank_finish = blank_finish;
        Ok(state)
    }

    /// Parses the explicit constructs following the current one.
    fn explicit_list(&mut self, doc: &mut Document, blank_finish: bool) {
        let (offset, blank_finish) =
            self.nested_list_parse(doc, self.rest(), State::Explicit, blank_finish, |_| {});
        self.goto_line(doc, offset);
        if !blank_finish {
            let w = self.unindent_warning(doc, "Explicit markup");
            doc.append(w);
        }
    }

    /// Which construct the current line starts, parsed; a comment if none.
    fn explicit_construct(&mut self, doc: &mut Document) -> Construct {
        let line = self.text().to_owned();
        let s: Vec<char> = line.chars().collect();
        let mut errors = Vec::new();
        let result = if let Some((label, end)) = patterns::footnote(&s) {
            Some(Ok(self.footnote(doc, &label, end, false)))
        } else if let Some((label, end)) = patterns::citation(&s) {
            Some(Ok(self.footnote(doc, &label, end, true)))
        } else if let Some(end) = patterns::hyperlink_target(&s) {
            Some(self.hyperlink_target(doc, end))
        } else if let Some(end) = patterns::substitution_def(&s) {
            Some(self.substitution_def(doc, &line, end))
        } else {
            patterns::directive(&s).map(|(name, _)| Ok(self.directive(doc, &name)))
        };
        match result {
            Some(Ok(construct)) => return construct,
            Some(Err(message)) => {
                let w = doc.message(
                    WARNING,
                    &message,
                    Vec::new(),
                    At::Top(Some(self.abs_line_number())),
                );
                errors.push(Node::from(w));
            }
            None => {}
        }
        let end = patterns::explicit_markup(&s).unwrap_or(s.len());
        let (mut nodes, blank_finish) = self.comment(doc, &line, end);
        nodes.extend(errors);
        (nodes, blank_finish)
    }

    /// A comment: the explicit markup block's text.
    fn comment(&mut self, doc: &mut Document, line: &str, end: usize) -> Construct {
        let start = self.line_offset;
        if self.is_next_line_blank() && strip(skip_chars(line, end)).is_empty() {
            let mut c = fixed(Kind::Comment, "", None);
            c.span = self.span(start, start);
            return (vec![c.into()], true);
        }
        let (mut indented, _, _, blank_finish) =
            self.get_first_known_indented(doc, end, false, true, true);
        while indented.last().is_some_and(|l| strip(&l.text).is_empty()) {
            indented.pop();
        }
        let mut c = fixed(Kind::Comment, &join(&indented), span_of(&indented));
        c.span = self.span(start, self.line_offset);
        (vec![c.into()], blank_finish)
    }

    /// A footnote (`.. [1]`, `.. [#]`, `.. [#name]`, `.. [*]`) or, with
    /// `citation`, a citation (`.. [name]`).
    fn footnote(
        &mut self,
        doc: &mut Document,
        label: &str,
        end: usize,
        citation: bool,
    ) -> Construct {
        let start = self.line_offset;
        let srcline = Some(self.abs_line_number());
        let (indented, _, offset, blank_finish) =
            self.get_first_known_indented(doc, end, false, true, true);
        let mut name = normalize_name(label);
        let label_el = || Element::new(Kind::Label).with_child(Text::new(label, None));
        let mut node = Element::new(if citation {
            Kind::Citation
        } else {
            Kind::Footnote
        });
        node.span = self.span(start, start);
        if citation {
            node.children.push(label_el().into());
            node.attributes.push("names", name.clone());
            doc.note_explicit_target(&mut node, Some(MsgNode::Itself), srcline);
        } else {
            if let Some(rest) = name.strip_prefix('#') {
                name = rest.to_owned();
                node.attributes.set("auto", 1i64);
                if !name.is_empty() {
                    node.attributes.push("names", name.clone());
                }
            } else if name == "*" {
                name.clear();
                node.attributes.set("auto", "*");
            } else {
                node.children.push(label_el().into());
                node.attributes.push("names", name.clone());
            }
            doc.set_id(&mut node, None, srcline);
            if name.is_empty() {
                doc.set_id(&mut node, Some(MsgNode::Itself), srcline);
            } else {
                doc.note_explicit_target(&mut node, Some(MsgNode::Itself), srcline);
            }
        }
        if indented.is_empty() {
            let what = if citation { "Citation" } else { "Footnote" };
            let w = doc.message(
                WARNING,
                &format!("{what} content expected."),
                Vec::new(),
                At::Top(None),
            );
            node.children.push(w.into());
        } else {
            doc.open(node, false);
            self.nested_parse(doc, indented, offset);
            node = doc.close().unwrap();
        }
        (vec![node.into()], blank_finish)
    }

    /// A hyperlink target: `.. _name: uri`, `.. _name: other_`, `.. __: uri`.
    fn hyperlink_target(&mut self, doc: &mut Document, end: usize) -> Result<Construct, String> {
        let start = self.line_offset;
        let lineno = self.abs_line_number();
        let (block, _, _, blank_finish) =
            self.get_first_known_indented(doc, end, true, false, true);
        let span = self.span(start, self.line_offset);
        let mut block: Vec<String> = block.iter().map(|l| escape2null(&l.text)).collect();
        let mut escaped = block.first().cloned().unwrap_or_default();
        let mut index = 0;
        let (match_end, name) = loop {
            if let Some(m) = patterns::target_name(&escaped.chars().collect::<Vec<_>>()) {
                break m;
            }
            index += 1;
            match block.get(index) {
                Some(next) => escaped.push_str(next),
                None => return Err("malformed hyperlink target.".into()),
            }
        };
        block.drain(..index);
        let first = format!("{} ", block[0]);
        let cut = char_len(&block[0]).saturating_sub(char_len(&escaped) - match_end);
        block[0] = strip(skip_chars(&first, cut)).to_owned();
        let target = self.make_target(doc, &block, lineno, name, span);
        Ok((vec![target.into()], blank_finish))
    }

    /// An anonymous target: `__ uri`.
    fn anonymous_target(&mut self, doc: &mut Document) -> Construct {
        let start = self.line_offset;
        let lineno = self.abs_line_number();
        let end = patterns::anonymous(&self.text().chars().collect::<Vec<_>>()).unwrap();
        let (block, _, _, blank_finish) = self.get_first_known_indented(doc, end, true, true, true);
        let span = self.span(start, self.line_offset);
        let block: Vec<String> = block.iter().map(|l| escape2null(&l.text)).collect();
        let target = self.make_target(doc, &block, lineno, None, span);
        (vec![target.into()], blank_finish)
    }

    /// A target element from the text after its name.
    fn make_target(
        &mut self,
        doc: &mut Document,
        block: &[String],
        lineno: usize,
        name: Option<String>,
        span: Option<Span>,
    ) -> Element {
        let mut target = Element::new(Kind::Target);
        target.span = span;
        match parse_target(block) {
            TargetRef::Name(refname) => {
                target.attributes.set("refname", normalize_name(&refname));
                add_target(doc, name, "", &mut target, lineno);
            }
            TargetRef::Uri(uri) => add_target(doc, name, &uri, &mut target, lineno),
        }
        target
    }

    /// A substitution definition: `.. |name| directive:: ...`.
    fn substitution_def(
        &mut self,
        doc: &mut Document,
        line: &str,
        end: usize,
    ) -> Result<Construct, String> {
        let start = self.line_offset;
        let srcline = Some(self.abs_line_number());
        let (block, _, mut offset, blank_finish) =
            self.get_first_known_indented(doc, end, false, false, true);
        let span = self.span(start, self.line_offset);
        let blocktext = format!("{}{}", take_chars(line, end), join(&block));
        let mut block: Vec<Line> = block;
        let mut escaped = escape2null(rstrip(block.first().map_or("", |l| l.text.as_str())));
        let mut index = 0;
        let (match_end, subname) = loop {
            if let Some(m) = patterns::substitution_name(&escaped.chars().collect::<Vec<_>>()) {
                break m;
            }
            index += 1;
            match block.get(index) {
                Some(next) => {
                    escaped = format!("{escaped} {}", escape2null(strip(&next.text)));
                }
                None => return Err("malformed substitution definition.".into()),
            }
        };
        block.drain(..index);
        let first = strip(&block[0].text).to_owned();
        let cut = char_len(&first).saturating_sub(char_len(&escaped) - match_end);
        block[0].text = skip_chars(&first, cut).to_owned();
        if block[0].text.is_empty() {
            block.remove(0);
            offset += 1;
        }
        while block.last().is_some_and(|l| strip(&l.text).is_empty()) {
            block.pop();
        }
        let literal = || -> Node { fixed(Kind::LiteralBlock, &blocktext, span).into() };
        if block.is_empty() {
            let w = doc.message(
                WARNING,
                &format!("Substitution definition \"{subname}\" missing contents."),
                vec![literal()],
                At::Exact(srcline),
            );
            return Ok((vec![w.into()], blank_finish));
        }
        block[0].text = strip(&block[0].text).to_owned();
        let mut node = Element::new(Kind::SubstitutionDefinition);
        node.span = span;
        node.attributes
            .push("names", whitespace_normalize_name(&subname));
        doc.open(node, false);
        let (_, blank_finish) = self.nested_list_parse(
            doc,
            (std::rc::Rc::from(block), 0, offset),
            State::SubstitutionDef,
            blank_finish,
            |_| {},
        );
        let mut node = doc.close().unwrap();
        let children = std::mem::take(&mut node.children);
        for child in children {
            match child {
                Node::Element(e) if !is_inline(&e.kind) => doc.append(e),
                other => node.children.push(other),
            }
        }
        for e in descendants(&node) {
            if e.kind == Kind::Problematic {
                let mut msg = doc.message(
                    ERROR,
                    "Problematic content in substitution definition",
                    vec![literal()],
                    At::Exact(srcline),
                );
                msg.children.push(
                    Element::new(Kind::BlockQuote)
                        .with_child(
                            Element::new(Kind::Paragraph).with_children(node.children.clone()),
                        )
                        .into(),
                );
                return Ok((vec![msg.into()], blank_finish));
            }
            if let Some(what) = disallowed_in_substitution(e) {
                let msg = doc.message(
                    ERROR,
                    &format!("{what} are not supported in a substitution definition."),
                    vec![literal()],
                    At::Exact(srcline),
                );
                return Ok((vec![msg.into()], blank_finish));
            }
        }
        if node.children.is_empty() {
            let w = doc.message(
                WARNING,
                &format!("Substitution definition \"{subname}\" empty or invalid."),
                vec![literal()],
                At::Exact(srcline),
            );
            return Ok((vec![w.into()], blank_finish));
        }
        doc.note_substitution_def(&mut node, &subname, srcline);
        Ok((vec![node.into()], blank_finish))
    }

    /// A directive inside a substitution definition.
    pub(crate) fn embedded_directive(&mut self, doc: &mut Document) -> Step {
        let s: Vec<char> = self.text().chars().collect();
        let (name, _) = patterns::embedded_directive(&s).unwrap();
        let (nodes, blank_finish) = self.directive(doc, &name);
        doc.extend(nodes);
        if !self.at_eof() {
            self.blank_finish = blank_finish;
        }
        Err(super::machine::Flow::Eof)
    }

    /// A directive. None are implemented yet, so this reports it as
    /// unknown, with its text.
    fn directive(&mut self, doc: &mut Document, name: &str) -> Construct {
        let info = doc.message(
            INFO,
            &format!(
                "No directive entry for \"{name}\" in module \
                 \"docutils.parsers.rst.languages.en\".\n\
                 Trying \"{name}\" as canonical directive name."
            ),
            Vec::new(),
            At::Top(doc.current_line),
        );
        doc.append(info);
        self.unknown_directive(doc, name)
    }

    /// An error for an unknown directive, with the directive's text.
    fn unknown_directive(&mut self, doc: &mut Document, name: &str) -> Construct {
        let start = self.line_offset;
        let lineno = self.abs_line_number();
        let (indented, _, _, blank_finish) =
            self.get_first_known_indented(doc, 0, false, false, true);
        let text = join(&indented);
        let span = self.span(start, self.line_offset);
        let error = doc.message(
            ERROR,
            &format!("Unknown directive type \"{name}\"."),
            vec![fixed(Kind::LiteralBlock, &text, span).into()],
            At::Top(Some(lineno)),
        );
        (vec![error.into()], blank_finish)
    }
}

/// What a target points to.
enum TargetRef {
    /// Another target, by reference name.
    Name(String),
    /// A URI (empty for an internal target).
    Uri(String),
}

/// docutils' `parse_target`: a reference (`name_`) or a URI with
/// whitespace removed.
fn parse_target(block: &[String]) -> TargetRef {
    if block.last().is_some_and(|l| strip(l).ends_with('_')) {
        let reference: Vec<&str> = block.iter().map(|l| strip(l)).collect();
        let reference = whitespace_normalize_name(&reference.join(" "));
        if let Some(name) = patterns::reference(&reference.chars().collect::<Vec<_>>()) {
            return TargetRef::Name(unescape(&name));
        }
    }
    let parts = split_escaped_whitespace(&block.join(" "));
    let uri: Vec<String> = parts
        .iter()
        .map(|p| split_whitespace(&unescape(p)).collect::<String>())
        .collect();
    TargetRef::Uri(uri.join(" "))
}

/// Names and registers a target (docutils' `add_target`).
fn add_target(
    doc: &mut Document,
    name: Option<String>,
    refuri: &str,
    target: &mut Element,
    lineno: usize,
) {
    let line = Some(lineno);
    match name.filter(|n| !n.is_empty()) {
        Some(name) => {
            target
                .attributes
                .push("names", normalize_name(&unescape(&name)));
            if !refuri.is_empty() {
                let uri = if patterns::is_email(refuri) {
                    format!("mailto:{refuri}")
                } else {
                    refuri.to_owned()
                };
                target.attributes.set("refuri", uri);
            }
            doc.note_explicit_target(target, Some(MsgNode::Parent), line);
        }
        None => {
            if !refuri.is_empty() {
                target.attributes.set("refuri", refuri);
            }
            target.attributes.set("anonymous", true);
            doc.set_id(target, None, line);
        }
    }
}
