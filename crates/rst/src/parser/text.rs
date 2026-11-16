//! Text blocks (docutils' `Text`, `Definition`, `Line` and
//! `QuotedLiteralBlock` states): paragraphs, literal blocks, definition
//! lists, section titles and transitions.

use super::document::{At, Document, ERROR, INFO, MsgNode, Style, WARNING, fixed, paragraph};
use super::inline::{Origin, astext, inline_text};
use super::machine::{Flow, Machine, State, Step, Tr, join, matched, span_of};
use crate::lines::Line;
use crate::nodes::{Element, Kind, Node, Text};
use crate::patterns;
use crate::util::{char_len, column_width, lstrip, normalize_name, rstrip, strip};

/// Whether `data` ends with `::` that is not escaped.
fn ends_with_literal_marker(data: &str) -> bool {
    let Some(before) = data.strip_suffix("::") else {
        return false;
    };
    let slashes = before.len() - before.trim_end_matches('\\').len();
    slashes % 2 == 0
}

/// Splits a term at classifier delimiters (` : `, docutils'
/// `' +: +'`).
fn split_classifiers(text: &str) -> Vec<String> {
    let s: Vec<char> = text.chars().collect();
    let mut parts = Vec::new();
    let mut last = 0;
    let mut i = 0;
    while i < s.len() {
        if s[i] == ' ' {
            let j = i + s[i..].iter().take_while(|&&c| c == ' ').count();
            if s.get(j) == Some(&':') && s.get(j + 1) == Some(&' ') {
                let k = j + 1 + s[j + 1..].iter().take_while(|&&c| c == ' ').count();
                parts.push(s[last..i].iter().collect());
                last = k;
                i = k;
                continue;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    parts.push(s[last..].iter().collect());
    parts
}

impl Machine {
    /// A paragraph of `lines`; also whether a literal block follows (the
    /// paragraph ends with `::`).
    /// Messages follow the paragraph; `lineno` is its first line.
    fn paragraph(&self, doc: &mut Document, lines: &[Line], lineno: usize) -> (Vec<Element>, bool) {
        let data = rstrip(&join(lines)).to_owned();
        let (text, literal_next) = if ends_with_literal_marker(&data) {
            let chars: Vec<char> = data.chars().collect();
            if chars.len() == 2 {
                return (Vec::new(), true);
            } else if matches!(chars[chars.len() - 3], ' ' | '\n') {
                let t: String = chars[..chars.len() - 3].iter().collect();
                (rstrip(&t).to_owned(), true)
            } else {
                (chars[..chars.len() - 1].iter().collect(), true)
            }
        } else {
            (data, false)
        };
        let span = span_of(lines);
        let (nodes, messages) = inline_text(doc, &text, lineno, &Origin::of(lines));
        let mut p = Element::new(Kind::Paragraph).with_children(nodes);
        p.span = span;
        let mut out = vec![p];
        out.extend(messages);
        (out, literal_next)
    }

    /// A blank line ends the paragraph.
    pub(crate) fn text_blank(&mut self, doc: &mut Document, context: &mut Vec<Line>) -> Step {
        let lineno = self.abs_line_number().saturating_sub(1);
        let (p, literal_next) = self.paragraph(doc, context, lineno);
        doc.extend(p);
        if literal_next {
            let nodes = self.literal_block(doc);
            doc.extend(nodes);
        }
        context.clear();
        Ok(State::Body)
    }

    /// More lines of a paragraph.
    pub(crate) fn text_text(&mut self, doc: &mut Document, context: &mut Vec<Line>) -> Step {
        let lineno = self.abs_line_number().saturating_sub(1);
        let (block, msg) = match self.get_text_block(doc, true) {
            Ok(block) => (block, None),
            Err((block, line)) => {
                let msg = doc.message(
                    ERROR,
                    "Unexpected indentation.",
                    Vec::new(),
                    At::Exact(Some(line)),
                );
                (block, Some(msg))
            }
        };
        let lines: Vec<Line> = context.iter().cloned().chain(block).collect();
        let (p, literal_next) = self.paragraph(doc, &lines, lineno);
        doc.extend(p);
        if let Some(msg) = msg {
            doc.append(msg);
        }
        if literal_next {
            self.next_line(doc, 1);
            let nodes = self.literal_block(doc);
            doc.extend(nodes);
        }
        context.clear();
        Ok(State::Body)
    }

    /// The literal block after a `::` paragraph: indented, or quoted.
    fn literal_block(&mut self, doc: &mut Document) -> Vec<Node> {
        let (mut indented, _, _, blank_finish) = self.get_indented(doc, false, true);
        while indented.last().is_some_and(|l| strip(&l.text).is_empty()) {
            indented.pop();
        }
        if indented.is_empty() {
            return self.quoted_literal_block(doc);
        }
        let mut nodes: Vec<Node> =
            vec![fixed(Kind::LiteralBlock, &join(&indented), span_of(&indented)).into()];
        if !blank_finish {
            nodes.push(self.unindent_warning(doc, "Literal block").into());
        }
        nodes
    }

    /// A literal block whose lines all start with the same punctuation.
    fn quoted_literal_block(&mut self, doc: &mut Document) -> Vec<Node> {
        doc.open(Element::new(Kind::Other("Element".into())), false);
        let mut m = Machine::new_view(self.current_on());
        m.run(doc, State::QuotedLiteralBlock);
        self.goto_line(doc, m.abs_line_offset());
        doc.close_detached().children
    }

    /// Quoted literal block lines.
    pub(crate) fn quoted(&mut self, doc: &mut Document, tr: Tr, context: &mut Vec<Line>) -> Step {
        match tr {
            Tr::Blank if !context.is_empty() => Err(Flow::Eof),
            Tr::Blank => Ok(State::QuotedLiteralBlock),
            Tr::Indent => {
                let msg = doc.message(
                    ERROR,
                    "Unexpected indentation.",
                    Vec::new(),
                    At::Top(Some(self.abs_line_number())),
                );
                self.messages.push(msg);
                self.previous_line(doc, 1);
                Err(Flow::Eof)
            }
            Tr::InitialQuoted => {
                self.quote = self.text().chars().next();
                self.initial_lineno = self.abs_line_number();
                *context = vec![self.line().clone()];
                Ok(State::QuotedLiteralBlock)
            }
            Tr::Quoted => {
                context.push(self.line().clone());
                Ok(State::QuotedLiteralBlock)
            }
            _ => {
                if !context.is_empty() {
                    let msg = doc.message(
                        ERROR,
                        "Inconsistent literal block quoting.",
                        Vec::new(),
                        At::Top(Some(self.abs_line_number())),
                    );
                    self.messages.push(msg);
                    self.previous_line(doc, 1);
                }
                Err(Flow::Eof)
            }
        }
    }

    /// The end of a quoted literal block.
    pub(crate) fn quoted_eof(&mut self, doc: &mut Document, context: &mut [Line]) {
        if !context.is_empty() {
            doc.append(fixed(Kind::LiteralBlock, &join(context), span_of(context)));
        } else {
            let msg = doc.message(
                WARNING,
                "Literal block expected; none found.",
                Vec::new(),
                At::Top(Some(self.abs_line_number())),
            );
            doc.append(msg);
            self.previous_line(doc, 1);
        }
        let messages = std::mem::take(&mut self.messages);
        doc.extend(messages);
    }

    /// A definition list item: the term in `termline`, the definition
    /// indented from the current line.
    fn definition_list_item(&mut self, doc: &mut Document, termline: &[Line]) -> (Element, bool) {
        let dd_lineno = self.abs_line_number();
        let (indented, _, offset, blank_finish) = self.get_indented(doc, false, true);
        let mut item = Element::new(Kind::DefinitionListItem);
        item.span = span_of(termline);
        let (terms, messages) = term(doc, &termline[0], dd_lineno.saturating_sub(1));
        item.children.extend(terms);
        let mut definition = Element::new(Kind::Definition).with_children(messages);
        if termline[0].text.ends_with("::") {
            definition.children.push(
                doc.message(
                    INFO,
                    "Blank line missing before literal block (after the \"::\")? \
                     Interpreted as a definition list item.",
                    Vec::new(),
                    At::Top(Some(dd_lineno)),
                )
                .into(),
            );
        }
        doc.open(definition, false);
        self.nested_parse(doc, indented, offset);
        item.children.push(doc.close_detached().into());
        super::document::cover_children(&mut item);
        (item, blank_finish)
    }

    /// An indented line after text: a definition list.
    pub(crate) fn text_indent(&mut self, doc: &mut Document, context: &mut Vec<Line>) -> Step {
        let (item, blank_finish) = self.definition_list_item(doc, context);
        doc.open(Element::new(Kind::DefinitionList), true);
        doc.append(item);
        let (offset, blank_finish) = self.nested_list_parse(
            doc,
            self.rest(),
            State::DefinitionList,
            blank_finish,
            |_| {},
        );
        self.goto_line(doc, offset);
        doc.close();
        if !blank_finish {
            let w = self.unindent_warning(doc, "Definition list");
            doc.append(w);
        }
        context.clear();
        Ok(State::Body)
    }

    /// A later definition list item.
    pub(crate) fn definition_indent(
        &mut self,
        doc: &mut Document,
        context: &mut Vec<Line>,
    ) -> Step {
        let (item, blank_finish) = self.definition_list_item(doc, context);
        doc.append(item);
        self.blank_finish = blank_finish;
        context.clear();
        Ok(State::DefinitionList)
    }

    /// An underline after text: a section title.
    pub(crate) fn text_underline(&mut self, doc: &mut Document, context: &mut Vec<Line>) -> Step {
        let lineno = self.abs_line_number();
        let title = rstrip(&context[0].text).to_owned();
        let underline = rstrip(self.text()).to_owned();
        let source = format!("{title}\n{underline}");
        let span = span_of(&[context[0].clone(), self.line().clone()]);
        let blocktext = format!("{}\n{}", context[0].text, self.text());
        let mut messages = Vec::new();
        if column_width(&title) > char_len(&underline) {
            if char_len(&underline) < 4 {
                if self.match_titles {
                    let msg = doc.message(
                        INFO,
                        "Possible title underline, too short for the title.\n\
                         Treating it as ordinary text because it's so short.",
                        Vec::new(),
                        At::Top(Some(lineno)),
                    );
                    doc.append(msg);
                }
                return Err(Flow::TransitionCorrection(Tr::Text));
            }
            messages.push(doc.message(
                WARNING,
                "Title underline too short.",
                vec![fixed(Kind::LiteralBlock, &blocktext, span).into()],
                At::Top(Some(lineno)),
            ));
        }
        if !self.match_titles {
            let msg = doc.message(
                ERROR,
                "Unexpected section title.",
                vec![fixed(Kind::LiteralBlock, &blocktext, span).into()],
                {
                    let (source, line) = self.source_and_line();
                    At::Source(source, line)
                },
            );
            doc.extend(messages);
            doc.append(msg);
            context.clear();
            return Ok(State::Body);
        }
        let style = Style::Under(matched(underline.chars().next())?);
        let title_line = context[0].clone();
        context.clear();
        self.section(
            doc,
            &title,
            (&title_line, 0),
            &source,
            style,
            lineno - 1,
            messages,
        );
        Ok(State::Body)
    }

    /// Adds a section if its title style fits the hierarchy.
    #[allow(clippy::too_many_arguments)]
    fn section(
        &mut self,
        doc: &mut Document,
        title: &str,
        title_from: (&Line, usize),
        source: &str,
        style: Style,
        lineno: usize,
        messages: Vec<Element>,
    ) {
        if self.check_subsection(doc, source, style, lineno) {
            self.new_subsection(doc, title, title_from, lineno, messages);
        }
    }

    /// Checks a title's level against the styles seen so far, and moves
    /// the current parent up to where the new section belongs.
    fn check_subsection(
        &mut self,
        doc: &mut Document,
        source: &str,
        style: Style,
        lineno: usize,
    ) -> bool {
        let old_level = doc.section_depth();
        let new_level = doc
            .title_styles
            .iter()
            .position(|&s| s == style)
            .map_or(doc.title_styles.len() + 1, |i| i + 1);
        if new_level > old_level + 1 {
            let styles: Vec<String> = doc.title_styles.iter().map(|s| s.display()).collect();
            let msg = doc.message(
                ERROR,
                &format!("Inconsistent title style: skip from level {old_level} to {new_level}."),
                vec![
                    fixed(Kind::LiteralBlock, source, None).into(),
                    paragraph(&format!("Established title styles: {}", styles.join(" "))).into(),
                ],
                At::Top(Some(lineno)),
            );
            doc.append(msg);
            return false;
        }
        if new_level <= old_level {
            doc.close_sections_to(new_level - 1);
        }
        if new_level > doc.title_styles.len() {
            doc.title_styles.push(style);
        }
        true
    }

    /// Opens a new section with `title`.
    fn new_subsection(
        &mut self,
        doc: &mut Document,
        title: &str,
        (title_line, skip): (&Line, usize),
        lineno: usize,
        messages: Vec<Element>,
    ) {
        // The section goes where it is now; title messages go after it.
        let at = doc.position();
        let title_span = Some(title_line.span);
        let origin = Origin::of(std::slice::from_ref(title_line)).skip(skip);
        let (nodes, title_messages) = inline_text(doc, title, lineno, &origin);
        let mut title_el = Element::new(Kind::Title).with_children(nodes);
        title_el.span = title_span;
        let name = normalize_name(&astext(&title_el));
        let mut section = Element::new(Kind::Section);
        section.span = title_span;
        section.attributes.push("names", name);
        section.children.push(title_el.into());
        section
            .children
            .extend(messages.into_iter().map(Node::from));
        section
            .children
            .extend(title_messages.into_iter().map(Node::from));
        let line = doc.current_line;
        doc.note_implicit_target(&mut section, Some(MsgNode::Itself), line);
        doc.open_at(section, at);
    }

    /// A blank line after a line of punctuation: a transition.
    pub(crate) fn line_blank(&mut self, doc: &mut Document, context: &mut Vec<Line>) -> Step {
        if char_len(strip(&context[0].text)) < 4 {
            return self.state_correction(doc, context, 1);
        }
        let mut t = transition(&context[0]);
        t.attributes
            .set(super::document::LINE, self.abs_line_number() as i64 - 1);
        doc.append(t);
        context.clear();
        Ok(State::Body)
    }

    /// The input ends after a line of punctuation: a transition.
    pub(crate) fn line_eof(
        &mut self,
        doc: &mut Document,
        context: &mut Vec<Line>,
    ) -> Result<(), Flow> {
        if char_len(strip(&context[0].text)) < 4 {
            self.state_correction(doc, context, 1)?;
        }
        let mut t = transition(&context[0]);
        t.attributes
            .set(super::document::LINE, self.abs_line_number() as i64 - 1);
        doc.append(t);
        Ok(())
    }

    /// Text after an overline: a section title, if an underline follows.
    pub(crate) fn line_text(&mut self, doc: &mut Document, context: &mut Vec<Line>) -> Step {
        let lineno = self.abs_line_number() - 1;
        let overline_line = context[0].clone();
        let title_line = self.line().clone();
        let (overline, title) = (overline_line.text.clone(), title_line.text.clone());
        let short = char_len(rstrip(&overline)) < 4;
        if !self.next_line(doc, 1) {
            let blocktext = format!("{overline}\n{title}");
            if short {
                return self.short_overline(doc, context, lineno, 2);
            }
            let span = span_of(&[overline_line, title_line]);
            let msg = doc.message(
                ERROR,
                "Incomplete section title.",
                vec![fixed(Kind::LiteralBlock, &blocktext, span).into()],
                At::Top(Some(lineno)),
            );
            doc.append(msg);
            context.clear();
            return Ok(State::Body);
        }
        let underline_line = self.line().clone();
        let underline_raw = underline_line.text.clone();
        let source = format!("{overline}\n{title}\n{underline_raw}");
        let span = span_of(&[overline_line.clone(), title_line.clone(), underline_line]);
        let overline = rstrip(&overline).to_owned();
        let underline = rstrip(&underline_raw).to_owned();
        let problem = if !patterns::line(&underline.chars().collect::<Vec<_>>()) {
            Some("Missing matching underline for section title overline.")
        } else if overline != underline {
            Some("Title overline & underline mismatch.")
        } else {
            None
        };
        if let Some(problem) = problem {
            if short {
                return self.short_overline(doc, context, lineno, 2);
            }
            let msg = doc.message(
                ERROR,
                problem,
                vec![fixed(Kind::LiteralBlock, &source, span).into()],
                At::Top(Some(lineno)),
            );
            doc.append(msg);
            context.clear();
            return Ok(State::Body);
        }
        let title = rstrip(&title).to_owned();
        let mut messages = Vec::new();
        if column_width(&title) > char_len(&overline) {
            if short {
                return self.short_overline(doc, context, lineno, 2);
            }
            messages.push(doc.message(
                WARNING,
                "Title overline too short.",
                vec![fixed(Kind::LiteralBlock, &source, span).into()],
                At::Top(Some(lineno)),
            ));
        }
        let style = Style::Over(
            matched(overline.chars().next())?,
            matched(underline.chars().next())?,
        );
        let skip = char_len(&title) - char_len(lstrip(&title));
        context.clear();
        self.section(
            doc,
            lstrip(&title),
            (&title_line, skip),
            &source,
            style,
            lineno + 1,
            messages,
        );
        Ok(State::Body)
    }

    /// Two lines of punctuation in a row.
    pub(crate) fn line_underline(&mut self, doc: &mut Document, context: &mut Vec<Line>) -> Step {
        let overline = context[0].clone();
        let blocktext = format!("{}\n{}", overline.text, self.text());
        let lineno = self.abs_line_number() - 1;
        if char_len(rstrip(&overline.text)) < 4 {
            self.short_overline(doc, context, lineno, 1)?;
        }
        let span = span_of(&[overline, self.line().clone()]);
        let msg = doc.message(
            ERROR,
            "Invalid section title or transition marker.",
            vec![fixed(Kind::LiteralBlock, &blocktext, span).into()],
            At::Top(Some(lineno)),
        );
        doc.append(msg);
        context.clear();
        Ok(State::Body)
    }

    /// A short overline is text after all.
    fn short_overline(
        &mut self,
        doc: &mut Document,
        context: &mut Vec<Line>,
        lineno: usize,
        lines: isize,
    ) -> Step {
        let msg = doc.message(
            INFO,
            "Possible incomplete section title.\nTreating the overline as \
             ordinary text because it's so short.",
            Vec::new(),
            At::Top(Some(lineno)),
        );
        doc.append(msg);
        self.state_correction(doc, context, lines)
    }

    /// Back up `lines` lines and reparse as text.
    fn state_correction(
        &mut self,
        doc: &mut Document,
        context: &mut Vec<Line>,
        lines: isize,
    ) -> Step {
        self.previous_line(doc, lines);
        context.clear();
        Err(Flow::StateCorrection(State::Body, Some(Tr::Text)))
    }
}

/// A transition element for marker line `line`.
fn transition(line: &Line) -> Element {
    let mut t = Element::new(Kind::Transition);
    t.span = Some(line.span);
    t
}

/// A definition list term and its classifiers.
fn term(doc: &mut Document, line: &Line, lineno: usize) -> (Vec<Node>, Vec<Element>) {
    let mut term = Element::new(Kind::Term);
    term.span = Some(line.span);
    let mut classifiers: Vec<Element> = Vec::new();
    let (inlines, messages) = inline_text(
        doc,
        &line.text,
        lineno,
        &Origin::of(std::slice::from_ref(line)),
    );
    for node in inlines {
        let current = classifiers.last_mut().unwrap_or(&mut term);
        match node {
            Node::Text(t) => {
                let parts = split_classifiers(&t.value);
                if parts.len() == 1 {
                    current.children.push(Node::Text(t));
                } else {
                    let first = rstrip(&parts[0]).to_owned();
                    current.children.push(Text::new(first, t.span).into());
                    for part in &parts[1..] {
                        classifiers.push(
                            Element::new(Kind::Classifier)
                                .with_child(Text::new(part.clone(), t.span)),
                        );
                    }
                }
            }
            other => current.children.push(other),
        }
    }
    let nodes = std::iter::once(term).chain(classifiers);
    (nodes.map(Node::from).collect(), messages)
}
