//! Body elements (docutils' `Body` state and the states that continue its
//! compound constructs): block quotes, bullet, enumerated, field and
//! option lists, doctest blocks and line blocks.

use super::document::{At, Document, ERROR, INFO, LINE, LINE_INDENT, RAW, WARNING, fixed};
use super::inline::{Origin, inline_text};
use super::machine::{Flow, Machine, State, Step, Tr, join, span_of};
use crate::lines::Line;
use crate::nodes::{AttrValue, Element, Kind, Node, Text};
use crate::patterns::{self, Format};
use crate::util::{char_len, from_roman, lstrip, rstrip, skip_chars, split_whitespace, to_roman};

/// Enumeration sequences, in the order docutils tries them.
const SEQUENCES: [&str; 5] = [
    "arabic",
    "loweralpha",
    "upperalpha",
    "lowerroman",
    "upperroman",
];

/// Whether `text` is an enumerator of `sequence`.
fn sequence_matches(sequence: &str, text: &str) -> bool {
    let mut chars = text.chars();
    match sequence {
        "arabic" => !text.is_empty() && text.chars().all(|c| c.is_ascii_digit()),
        "loweralpha" => {
            chars.next().is_some_and(|c| c.is_ascii_lowercase()) && chars.next().is_none()
        }
        "upperalpha" => {
            chars.next().is_some_and(|c| c.is_ascii_uppercase()) && chars.next().is_none()
        }
        "lowerroman" => !text.is_empty() && text.chars().all(|c| "ivxlcdm".contains(c)),
        "upperroman" => !text.is_empty() && text.chars().all(|c| "IVXLCDM".contains(c)),
        _ => false,
    }
}

/// An enumerator's format, sequence, text and ordinal (`None` if invalid).
fn parse_enumerator(
    e: &patterns::Enumerator,
    expected: Option<&str>,
) -> (Format, &'static str, Option<i64>) {
    let text = e.text.as_str();
    let mut sequence: Option<&'static str> = None;
    if text == "#" {
        sequence = Some("#");
    } else if let Some(expected) = expected {
        sequence = SEQUENCES
            .into_iter()
            .find(|&s| s == expected && sequence_matches(s, text));
    } else if text == "i" {
        sequence = Some("lowerroman");
    } else if text == "I" {
        sequence = Some("upperroman");
    }
    let sequence = sequence.unwrap_or_else(|| {
        SEQUENCES
            .into_iter()
            .find(|s| sequence_matches(s, text))
            .unwrap_or("arabic")
    });
    let first = text.chars().next().unwrap_or('a') as i64;
    let ordinal = match sequence {
        "#" => Some(1),
        "arabic" => text.parse().ok(),
        "loweralpha" => Some(first - 'a' as i64 + 1),
        "upperalpha" => Some(first - 'A' as i64 + 1),
        _ => from_roman(text),
    };
    (e.format, sequence, ordinal)
}

/// The enumerator after `ordinal` and the auto-enumerator, each with a
/// trailing space; `None` if out of range.
fn make_enumerator(ordinal: i64, sequence: &str, format: Format) -> Option<(String, String)> {
    let enumerator = match sequence {
        "#" => "#".to_owned(),
        "arabic" => ordinal.to_string(),
        s if s.ends_with("alpha") => {
            if ordinal > 26 {
                return None;
            }
            let c = char::from(b'a' + (ordinal - 1) as u8);
            if s.starts_with("upper") {
                c.to_ascii_uppercase().to_string()
            } else {
                c.to_string()
            }
        }
        s => {
            let roman = to_roman(ordinal)?;
            if s.starts_with("lower") {
                roman.to_lowercase()
            } else {
                roman
            }
        }
    };
    let (prefix, suffix) = (format.prefix(), format.suffix());
    Some((
        format!("{prefix}{enumerator}{suffix} "),
        format!("{prefix}#{suffix} "),
    ))
}

/// A line as characters, for the pattern matchers.
fn chars_of(line: &str) -> Vec<char> {
    line.chars().collect()
}

/// The `--` (or em dash) starting a block quote attribution: the end of
/// the dash and the spaces after it, if text follows.
fn attribution_start(line: &str) -> Option<usize> {
    let s = chars_of(line);
    let dash = if s.first() == Some(&'\u{2014}') {
        1
    } else if s.starts_with(&['-', '-']) {
        let n = if s.get(2) == Some(&'-') { 3 } else { 2 };
        if s.get(n) == Some(&'-') {
            return None;
        }
        n
    } else {
        return None;
    };
    let spaces = s[dash..].iter().take_while(|&&c| c == ' ').count();
    let end = dash + spaces;
    (end < s.len() && s[end] != '\n').then_some(end)
}

/// Splits `options` at `, ` outside angle brackets.
fn split_options(marker: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut rest = marker;
    let mut current = String::new();
    while let Some(i) = rest.find(", ") {
        let after = &rest[i + 2..];
        let inside = after
            .find(['<', '>'])
            .is_some_and(|j| after.as_bytes()[j] == b'>');
        current.push_str(&rest[..i]);
        if inside {
            current.push_str(", ");
        } else {
            parts.push(std::mem::take(&mut current));
        }
        rest = after;
    }
    current.push_str(rest);
    parts.push(current);
    parts
}

/// The `option` elements of an option marker, or a markup error.
fn parse_option_marker(marker: &str) -> Result<Vec<Element>, String> {
    let mut options = Vec::new();
    for optionstring in split_options(rstrip(marker)) {
        let mut tokens: Vec<String> = split_whitespace(&optionstring).map(str::to_owned).collect();
        let mut delimiter = " ";
        let first = tokens.first().cloned().unwrap_or_default();
        if let Some((name, value)) = first.split_once('=') {
            tokens.splice(..1, [name.to_owned(), value.to_owned()]);
            delimiter = "=";
        } else if char_len(&first) > 2
            && ((first.starts_with('-') && !first.starts_with("--")) || first.starts_with('+'))
        {
            let head: String = first.chars().take(2).collect();
            let tail = skip_chars(&first, 2).to_owned();
            tokens.splice(..1, [head, tail]);
            delimiter = "";
        }
        if tokens.len() > 1
            && tokens[1].starts_with('<')
            && tokens.last().is_some_and(|t| t.ends_with('>'))
        {
            let joined = tokens[1..].join(" ");
            tokens.truncate(1);
            tokens.push(joined);
        }
        if tokens.is_empty() || tokens.len() > 2 {
            return Err(format!(
                "wrong number of option tokens (={}), should be 1 or 2: \"{optionstring}\"",
                tokens.len()
            ));
        }
        let mut option = Element::new(Kind::Option).with_child(
            Element::new(Kind::OptionString).with_child(Text::new(tokens[0].clone(), None)),
        );
        if let Some(arg) = tokens.get(1) {
            option.children.push(
                Element::new(Kind::OptionArgument)
                    .with_attr("delimiter", delimiter)
                    .with_child(Text::new(arg.clone(), None))
                    .into(),
            );
        }
        options.push(option);
    }
    Ok(options)
}

/// Nests line block lines by indentation (docutils'
/// `nest_line_block_segment`).
pub(crate) fn nest_line_block_segment(items: Vec<Node>) -> Vec<Node> {
    let indent = |n: &Node| match n {
        Node::Element(e) => match e.attributes.get(LINE_INDENT) {
            Some(AttrValue::Int(i)) => *i,
            _ => 0,
        },
        Node::Text(_) => 0,
    };
    let least = items.iter().map(indent).min().unwrap_or(0);
    let mut out = Vec::new();
    let mut nested: Vec<Node> = Vec::new();
    let flush = |nested: &mut Vec<Node>, out: &mut Vec<Node>| {
        if !nested.is_empty() {
            let mut block = Element::new(Kind::LineBlock);
            block.children = nest_line_block_segment(std::mem::take(nested));
            super::document::cover_children(&mut block);
            out.push(block.into());
        }
    };
    for item in items {
        if indent(&item) > least {
            nested.push(item);
        } else {
            flush(&mut nested, &mut out);
            out.push(item);
        }
    }
    flush(&mut nested, &mut out);
    out
}

impl Machine {
    /// An indented block: block quotes.
    pub(crate) fn indent(&mut self, doc: &mut Document, state: State) -> Step {
        let (indented, _, offset, blank_finish) = self.get_indented(doc, false, true);
        let elements = self.block_quote(doc, indented, offset);
        doc.extend(elements);
        if !blank_finish {
            let w = self.unindent_warning(doc, "Block quote");
            doc.append(w);
        }
        Ok(state)
    }

    /// Block quotes from `indented`, split at attributions.
    pub(crate) fn block_quote(
        &mut self,
        doc: &mut Document,
        mut indented: Vec<Line>,
        mut offset: isize,
    ) -> Vec<Element> {
        let mut elements = Vec::new();
        while !indented.is_empty() {
            let split = split_attribution(&indented);
            let (quote_lines, rest) = match &split {
                Some(s) => (indented[..s.start].to_vec(), Some(s)),
                None => (indented.clone(), None),
            };
            doc.open(Element::new(Kind::BlockQuote), false);
            self.nested_parse(doc, quote_lines, offset);
            let mut quote = doc.close().unwrap();
            let Some(split) = rest else {
                elements.push(quote);
                break;
            };
            let text = rstrip(&join(&split.lines)).to_owned();
            let lineno = (offset + split.start as isize + 1).max(0) as usize;
            let (nodes, messages) = inline_text(doc, &text, lineno, &Origin::of(&split.lines));
            let mut attribution = Element::new(Kind::Attribution).with_children(nodes);
            attribution.span = span_of(&split.lines);
            quote.children.push(attribution.into());
            super::document::cover_children(&mut quote);
            elements.push(quote);
            elements.extend(messages);
            offset += split.end as isize;
            indented = indented[split.end.min(indented.len())..].to_vec();
            while indented.first().is_some_and(|l| l.text.is_empty()) {
                indented.remove(0);
                offset += 1;
            }
        }
        elements
    }

    /// A list item's content, from the current line, its marker `indent`
    /// characters wide.
    fn list_item(&mut self, doc: &mut Document, indent: usize) -> (Element, bool) {
        let start = self.line_offset;
        let (indented, offset, blank_finish) = if !skip_chars(self.text(), indent).is_empty() {
            self.get_known_indented(doc, indent)
        } else {
            let (lines, _, offset, blank_finish) =
                self.get_first_known_indented(doc, indent, false, true, true);
            (lines, offset, blank_finish)
        };
        let mut item = Element::new(Kind::ListItem);
        item.span = self.span(start, start);
        if !indented.is_empty() {
            doc.open(item, false);
            self.nested_parse(doc, indented, offset);
            item = doc.close().unwrap();
        }
        (item, blank_finish)
    }

    /// The first item of a bullet list, and the rest of the list.
    pub(crate) fn bullet(&mut self, doc: &mut Document, state: State) -> Step {
        let end = patterns::bullet(&chars_of(self.text())).unwrap();
        let bullet = self.text().chars().next().unwrap().to_string();
        let list = Element::new(Kind::BulletList).with_attr("bullet", bullet);
        doc.open(list, true);
        let (item, blank_finish) = self.list_item(doc, end);
        doc.append(item);
        let (offset, blank_finish) =
            self.nested_list_parse(doc, self.rest(), State::BulletList, blank_finish, |_| {});
        self.goto_line(doc, offset);
        doc.close();
        if !blank_finish {
            let w = self.unindent_warning(doc, "Bullet list");
            doc.append(w);
        }
        Ok(state)
    }

    /// A later bullet list item.
    pub(crate) fn bullet_list_item(&mut self, doc: &mut Document, state: State) -> Step {
        let first = self.text().chars().next().unwrap().to_string();
        if doc.top().attributes.get_str("bullet") != Some(first.as_str()) {
            return self.invalid_input(doc);
        }
        let end = patterns::bullet(&chars_of(self.text())).unwrap();
        let (item, blank_finish) = self.list_item(doc, end);
        doc.append(item);
        self.blank_finish = blank_finish;
        Ok(state)
    }

    /// Whether an enumerator is the start of a list item: valid, and the
    /// next line blank, indented, or the next enumerator.
    fn is_enumerated_list_item(
        &self,
        ordinal: Option<i64>,
        sequence: &str,
        format: Format,
    ) -> bool {
        let Some(ordinal) = ordinal else {
            return false;
        };
        let Some(next) = self.get(self.line_offset + 1) else {
            return true;
        };
        let first: String = next.text.chars().take(1).collect();
        if first.trim().is_empty() {
            return true;
        }
        make_enumerator(ordinal + 1, sequence, format)
            .is_some_and(|(n, auto)| next.text.starts_with(&n) || next.text.starts_with(&auto))
    }

    /// The first item of an enumerated list, and the rest of the list.
    pub(crate) fn enumerator(&mut self, doc: &mut Document, state: State) -> Step {
        let e = patterns::enumerator(&chars_of(self.text())).unwrap();
        let (format, sequence, ordinal) = parse_enumerator(&e, None);
        if !self.is_enumerated_list_item(ordinal, sequence, format) {
            return Err(Flow::TransitionCorrection(Tr::Text));
        }
        let ordinal = ordinal.unwrap();
        let enumtype = if sequence == "#" { "arabic" } else { sequence };
        let mut list = Element::new(Kind::EnumeratedList)
            .with_attr("enumtype", enumtype)
            .with_attr("prefix", format.prefix())
            .with_attr("suffix", format.suffix());
        let mut msg = None;
        if ordinal != 1 {
            list.attributes.set("start", ordinal);
            msg = Some(doc.message(
                INFO,
                &format!(
                    "Enumerated list start value not ordinal-1: \"{}\" (ordinal {ordinal})",
                    e.text
                ),
                Vec::new(),
                At::Exact(Some(self.abs_line_number())),
            ));
        }
        doc.open(list, true);
        if let Some(msg) = msg {
            doc.append_to_grandparent(msg);
        }
        let (item, blank_finish) = self.list_item(doc, e.end);
        doc.append(item);
        let (offset, blank_finish) =
            self.nested_list_parse(doc, self.rest(), State::EnumeratedList, blank_finish, |m| {
                m.last_ordinal = ordinal;
                m.format = format;
                m.auto = sequence == "#";
            });
        self.goto_line(doc, offset);
        doc.close();
        if !blank_finish {
            let w = self.unindent_warning(doc, "Enumerated list");
            doc.append(w);
        }
        Ok(state)
    }

    /// A later enumerated list item.
    pub(crate) fn enumerated_list_item(&mut self, doc: &mut Document, state: State) -> Step {
        let e = patterns::enumerator(&chars_of(self.text())).unwrap();
        let enumtype = doc
            .top()
            .attributes
            .get_str("enumtype")
            .unwrap_or("arabic")
            .to_owned();
        let (format, sequence, ordinal) = parse_enumerator(&e, Some(&enumtype));
        if format != self.format
            || (sequence != "#"
                && (sequence != enumtype || self.auto || ordinal != Some(self.last_ordinal + 1)))
            || !self.is_enumerated_list_item(ordinal, sequence, format)
        {
            return self.invalid_input(doc);
        }
        if sequence == "#" {
            self.auto = true;
        }
        let (item, blank_finish) = self.list_item(doc, e.end);
        doc.append(item);
        self.blank_finish = blank_finish;
        self.last_ordinal = ordinal.unwrap_or(1);
        Ok(state)
    }

    /// A field: its name and body. With `simple` (directive options), the
    /// body is only split into paragraphs of plain text.
    fn field(&mut self, doc: &mut Document, end: usize, simple: bool) -> (Element, bool) {
        let marker: String = self.text().chars().take(end).collect();
        let name = &marker[1..];
        let name = &name[..name.rfind(':').unwrap_or(name.len())];
        let start = self.line_offset;
        let lineno = self.abs_line_number();
        let origin = Origin::of(std::slice::from_ref(self.line())).skip(1);
        let (indented, _, offset, blank_finish) =
            self.get_first_known_indented(doc, end, false, true, true);
        let span = self.span(start, start);
        let (nodes, messages) = inline_text(doc, name, lineno, &origin);
        let mut field_name = Element::new(Kind::FieldName).with_children(nodes);
        field_name.span = span;
        let mut body = Element::new(Kind::FieldBody).with_children(messages);
        body.attributes.set(RAW, join(&indented));
        if simple {
            let mut lines: Vec<Line> = Vec::new();
            for line in indented.iter().cloned().chain([Line::default()]) {
                if !crate::util::strip(&line.text).is_empty() {
                    lines.push(line);
                } else if !lines.is_empty() {
                    let text = join(&lines);
                    let mut p =
                        Element::new(Kind::Paragraph).with_child(Text::new(text, span_of(&lines)));
                    p.span = span_of(&lines);
                    body.children.push(p.into());
                    lines.clear();
                }
            }
        } else if !indented.is_empty() {
            doc.open(body, false);
            self.nested_parse(doc, indented, offset);
            body = doc.close().unwrap();
        }
        let mut field = Element::new(Kind::Field)
            .with_child(field_name)
            .with_child(body);
        field.span = span;
        field.attributes.set(LINE, lineno as i64);
        super::document::cover_children(&mut field);
        (field, blank_finish)
    }

    /// The first field of a field list, and the rest of the list.
    pub(crate) fn field_marker(&mut self, doc: &mut Document, state: State) -> Step {
        let end = patterns::field_marker(&chars_of(self.text())).unwrap();
        doc.open(Element::new(Kind::FieldList), true);
        let (field, blank_finish) = self.field(doc, end, false);
        doc.append(field);
        let (offset, blank_finish) =
            self.nested_list_parse(doc, self.rest(), State::FieldList, blank_finish, |_| {});
        self.goto_line(doc, offset);
        doc.close();
        if !blank_finish {
            let w = self.unindent_warning(doc, "Field list");
            doc.append(w);
        }
        Ok(state)
    }

    /// A later field.
    pub(crate) fn field_list_item(&mut self, doc: &mut Document, state: State) -> Step {
        let end = patterns::field_marker(&chars_of(self.text())).unwrap();
        let (field, blank_finish) = self.field(doc, end, state == State::ExtensionOptions);
        doc.append(field);
        self.blank_finish = blank_finish;
        Ok(state)
    }

    /// An option list item; a markup error, or a correction to text when
    /// there is no description.
    fn option_list_item(
        &mut self,
        doc: &mut Document,
        end: usize,
    ) -> Result<Result<(Element, bool), String>, Flow> {
        let offset = self.abs_line_offset();
        let marker: String = self.text().chars().take(end).collect();
        let options = match parse_option_marker(&marker) {
            Ok(options) => options,
            Err(e) => return Ok(Err(e)),
        };
        let start = self.line_offset;
        let (indented, _, line_offset, blank_finish) =
            self.get_first_known_indented(doc, end, false, true, true);
        if indented.is_empty() {
            self.goto_line(doc, offset);
            return Err(Flow::TransitionCorrection(Tr::Text));
        }
        let mut group = Element::new(Kind::OptionGroup);
        group.children = options.into_iter().map(Node::from).collect();
        group.span = self.span(start, start);
        doc.open(Element::new(Kind::Description), false);
        self.nested_parse(doc, indented, line_offset);
        let description = doc.close().unwrap();
        let mut item = Element::new(Kind::OptionListItem)
            .with_child(group)
            .with_child(description);
        super::document::cover_children(&mut item);
        Ok(Ok((item, blank_finish)))
    }

    /// The first item of an option list, and the rest of the list.
    pub(crate) fn option_marker(&mut self, doc: &mut Document, state: State) -> Step {
        let end = patterns::option_marker(&chars_of(self.text())).unwrap();
        let (item, blank_finish) = match self.option_list_item(doc, end)? {
            Ok(item) => item,
            Err(error) => {
                let msg = doc.message(
                    ERROR,
                    &format!("Invalid option list marker: {error}"),
                    Vec::new(),
                    At::Top(None),
                );
                doc.append(msg);
                let (indented, _, offset, blank_finish) =
                    self.get_first_known_indented(doc, end, false, true, true);
                let elements = self.block_quote(doc, indented, offset);
                doc.extend(elements);
                if !blank_finish {
                    let w = self.unindent_warning(doc, "Option list");
                    doc.append(w);
                }
                return Ok(state);
            }
        };
        doc.open(Element::new(Kind::OptionList), true);
        doc.append(item);
        let (offset, blank_finish) =
            self.nested_list_parse(doc, self.rest(), State::OptionList, blank_finish, |_| {});
        self.goto_line(doc, offset);
        doc.close();
        if !blank_finish {
            let w = self.unindent_warning(doc, "Option list");
            doc.append(w);
        }
        Ok(state)
    }

    /// A later option list item.
    pub(crate) fn option_list_next(&mut self, doc: &mut Document, state: State) -> Step {
        let end = patterns::option_marker(&chars_of(self.text())).unwrap();
        match self.option_list_item(doc, end)? {
            Ok((item, blank_finish)) => {
                doc.append(item);
                self.blank_finish = blank_finish;
                Ok(state)
            }
            Err(_) => self.invalid_input(doc),
        }
    }

    /// A doctest block: the text block as is.
    pub(crate) fn doctest(&mut self, doc: &mut Document, state: State) -> Step {
        let block = match self.get_text_block(doc, false) {
            Ok(b) | Err((b, _)) => b,
        };
        doc.append(fixed(Kind::DoctestBlock, &join(&block), span_of(&block)));
        Ok(state)
    }

    /// One line of a line block, with continuation lines.
    /// Returns the line, its messages, and whether a blank line ended it.
    fn line_block_line(&mut self, doc: &mut Document) -> (Element, Vec<Element>, bool) {
        let (end, spaces) = patterns::line_block(&chars_of(self.text())).unwrap();
        let empty = rstrip(self.text()) == "|";
        let start = self.line_offset;
        let lineno = self.abs_line_number();
        let (indented, _, _, blank_finish) =
            self.get_first_known_indented(doc, end, true, true, true);
        let text = join(&indented);
        let span = self.span(start, self.line_offset);
        let (nodes, messages) = inline_text(doc, &text, lineno, &Origin::of(&indented));
        let mut line = Element::new(Kind::Line).with_children(nodes);
        line.span = span;
        if !empty {
            line.attributes.set(LINE_INDENT, spaces as i64 - 1);
        }
        (line, messages, blank_finish)
    }

    /// A line block.
    pub(crate) fn line_block(&mut self, doc: &mut Document, state: State) -> Step {
        // The first line is parsed in the block's parent, where the block
        // already is.
        let at = doc.position();
        let lineno = self.abs_line_number();
        let (line, messages, mut blank_finish) = self.line_block_line(doc);
        doc.open_at(Element::new(Kind::LineBlock), at);
        doc.append(line);
        for msg in messages {
            doc.append_to_grandparent(msg);
        }
        if !blank_finish {
            let (offset, bf) =
                self.nested_list_parse(doc, self.rest(), State::LineBlock, false, |_| {});
            blank_finish = bf;
            self.goto_line(doc, offset);
        }
        let block = doc.top();
        if let Some(Node::Element(first)) = block.children.first_mut()
            && first.attributes.get(LINE_INDENT).is_none()
        {
            first.attributes.set(LINE_INDENT, 0i64);
        }
        let mut previous = 0;
        for child in &mut block.children {
            if let Node::Element(e) = child {
                match e.attributes.get(LINE_INDENT) {
                    Some(AttrValue::Int(i)) => previous = *i,
                    _ => e.attributes.set(LINE_INDENT, previous),
                }
            }
        }
        let items = std::mem::take(&mut block.children);
        block.children = nest_line_block_segment(items);
        doc.close();
        if !blank_finish {
            let w = doc.message(
                WARNING,
                "Line block ends without a blank line.",
                Vec::new(),
                At::Top(Some(lineno + 1)),
            );
            doc.append(w);
        }
        Ok(state)
    }

    /// A later line of a line block.
    pub(crate) fn line_block_next(&mut self, doc: &mut Document, state: State) -> Step {
        let (line, messages, blank_finish) = self.line_block_line(doc);
        doc.append(line);
        for msg in messages {
            doc.append_to_grandparent(msg);
        }
        self.blank_finish = blank_finish;
        Ok(state)
    }

    /// A line of punctuation: a section overline or a transition, or (where
    /// titles are not allowed) an error.
    pub(crate) fn punctuation_line(
        &mut self,
        doc: &mut Document,
        state: State,
        context: &mut Vec<Line>,
    ) -> Step {
        if self.match_titles {
            *context = vec![self.line().clone()];
            return Ok(State::Line);
        }
        let marker = crate::util::strip(self.text());
        if marker == "::" {
            return Err(Flow::TransitionCorrection(Tr::Text));
        }
        if char_len(marker) < 4 {
            let msg = doc.message(
                INFO,
                "Unexpected possible title overline or transition.\n\
                 Treating it as ordinary text because it's so short.",
                Vec::new(),
                At::Top(Some(self.abs_line_number())),
            );
            doc.append(msg);
            return Err(Flow::TransitionCorrection(Tr::Text));
        }
        let span = self.span(self.line_offset, self.line_offset);
        let msg = doc.message(
            ERROR,
            "Unexpected section title or transition.",
            vec![fixed(Kind::LiteralBlock, self.text(), span).into()],
            At::Top(Some(self.abs_line_number())),
        );
        doc.append(msg);
        Ok(state)
    }
}

/// An attribution found in a block quote: where it starts, its lines
/// (marker and indentation removed) and the index past its end.
struct Attribution {
    start: usize,
    lines: Vec<Line>,
    end: usize,
}

/// Finds a block quote attribution: a line after a blank line starting
/// with `--`, `---` or an em dash, its continuation lines indented alike.
fn split_attribution(indented: &[Line]) -> Option<Attribution> {
    let mut blank: Option<usize> = None;
    let mut nonblank_seen = false;
    for (i, line) in indented.iter().enumerate() {
        let text = rstrip(&line.text);
        if text.is_empty() {
            blank = Some(i);
            continue;
        }
        if nonblank_seen
            && i > 0
            && blank == Some(i - 1)
            && let Some(mark) = attribution_start(text)
            && let Some((end, indent)) = check_attribution(indented, i)
        {
            let mut lines: Vec<Line> = indented[i..end.min(indented.len())].to_vec();
            lines[0] = super::machine::trim_line(&lines[0], mark);
            for line in lines.iter_mut().skip(1) {
                *line = super::machine::trim_line(line, indent);
            }
            return Some(Attribution {
                start: i,
                lines,
                end,
            });
        }
        nonblank_seen = true;
    }
    None
}

/// The index past an attribution starting at `start` and the indent of its
/// continuation lines; `None` if they are not indented alike.
fn check_attribution(indented: &[Line], start: usize) -> Option<(usize, usize)> {
    let mut indent: Option<usize> = None;
    let mut i = start + 1;
    let mut ended = false;
    while i < indented.len() {
        let line = rstrip(&indented[i].text);
        if line.is_empty() {
            ended = true;
            break;
        }
        let this = char_len(line) - char_len(lstrip(line));
        match indent {
            None => indent = Some(this),
            Some(n) if n != this => return None,
            _ => {}
        }
        i += 1;
    }
    if !ended && start + 1 >= indented.len() {
        // Python's for-else on an empty range leaves `i` one further.
        i = start + 2;
    }
    Some((i, indent.unwrap_or(0)))
}
