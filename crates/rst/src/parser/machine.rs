//! The line-by-line state machine (docutils' `statemachine.StateMachineWS`
//! and `RSTState`): moving through the input, extracting text and indented
//! blocks, and running states until the input ends.
//!
//! As in docutils, nested constructs (list items, block quotes, ...) are
//! parsed by a nested machine over their own lines. Line offsets are
//! signed because docutils lets them go to -1 and indexes like Python.

use std::cell::RefCell;
use std::rc::Rc;

use super::document::{At, Document, WARNING};
use crate::lines::Line;
use crate::nodes::Span;
use crate::patterns::Format;
use crate::util::{char_len, lstrip, skip_chars, strip};

/// A state of the machine (a docutils `State` class).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    Body,
    BulletList,
    DefinitionList,
    EnumeratedList,
    FieldList,
    OptionList,
    LineBlock,
    Explicit,
    /// Directive options: fields whose bodies are plain text.
    ExtensionOptions,
    /// The `meta` directive's fields.
    MetaBody,
    Text,
    Definition,
    Line,
    SubstitutionDef,
    QuotedLiteralBlock,
}

/// A transition: a line pattern and what to do on a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tr {
    Blank,
    Indent,
    Bullet,
    Enumerator,
    FieldMarker,
    OptionMarker,
    Doctest,
    LineBlock,
    GridTableTop,
    SimpleTableTop,
    ExplicitMarkup,
    Anonymous,
    Line,
    Text,
    Underline,
    EmbeddedDirective,
    InitialQuoted,
    Quoted,
}

const BODY: &[Tr] = &[
    Tr::Blank,
    Tr::Indent,
    Tr::Bullet,
    Tr::Enumerator,
    Tr::FieldMarker,
    Tr::OptionMarker,
    Tr::Doctest,
    Tr::LineBlock,
    Tr::GridTableTop,
    Tr::SimpleTableTop,
    Tr::ExplicitMarkup,
    Tr::Anonymous,
    Tr::Line,
    Tr::Text,
];
const TEXT: &[Tr] = &[Tr::Blank, Tr::Indent, Tr::Underline, Tr::Text];
const SUBSTITUTION: &[Tr] = &[Tr::Blank, Tr::Indent, Tr::EmbeddedDirective, Tr::Text];
const QUOTE_FIRST: &[Tr] = &[Tr::Blank, Tr::Indent, Tr::InitialQuoted, Tr::Text];
const QUOTE_REST: &[Tr] = &[Tr::Quoted, Tr::Blank, Tr::Indent, Tr::Text];

/// How a transition ends other than normally (docutils uses exceptions).
#[derive(Debug, Clone, Copy)]
pub(crate) enum Flow {
    /// End this machine (`EOFError`).
    Eof,
    /// Retry the line in the same state with this transition only.
    TransitionCorrection(Tr),
    /// Retry the line in another state, optionally with one transition.
    StateCorrection(State, Option<Tr>),
}

/// The result of a transition: the next state.
pub(crate) type Step = Result<State, Flow>;

/// Input lines shared by the machines working on them. Like docutils'
/// `ViewList`, a list taken from another remembers where (`parent`), and
/// lines inserted into it (by `include`) go into that list too.
pub(crate) struct LineList {
    lines: Rc<[Line]>,
    parent: Option<(Lines, isize)>,
}

/// A shared [`LineList`].
pub(crate) type Lines = Rc<RefCell<LineList>>;

impl LineList {
    /// A list of `lines`, taken from `parent` at an index, if any.
    pub(crate) fn new(lines: Vec<Line>, parent: Option<(Lines, isize)>) -> Lines {
        Rc::new(RefCell::new(LineList {
            lines: Rc::from(lines),
            parent,
        }))
    }

    /// The lines now.
    pub(crate) fn get(this: &Lines) -> Rc<[Line]> {
        this.borrow().lines.clone()
    }

    /// Inserts `new` at `index`, here and in the lists this one was taken
    /// from.
    fn insert(this: &Lines, index: usize, new: &[Line]) {
        let (index, parent) = {
            let mut list = this.borrow_mut();
            let mut lines = list.lines.to_vec();
            let index = index.min(lines.len());
            lines.splice(index..index, new.iter().cloned());
            list.lines = Rc::from(lines);
            (index, list.parent.clone())
        };
        if let Some((parent, offset)) = parent {
            LineList::insert(&parent, (index as isize + offset).max(0) as usize, new);
        }
    }
}

/// A state machine over some lines.
pub(crate) struct Machine {
    list: Lines,
    /// `list`'s lines when last looked at.
    data: Rc<[Line]>,
    lo: usize,
    /// The absolute line offset of the first line.
    pub(crate) input_offset: isize,
    /// The current line, relative to the first.
    pub(crate) line_offset: isize,
    /// Whether section titles are allowed.
    pub(crate) match_titles: bool,
    top: bool,
    /// Whether the list or construct parsed ended with a blank line.
    pub(crate) blank_finish: bool,
    // Enumerated lists: the last ordinal, the format, auto-numbering.
    pub(crate) last_ordinal: i64,
    pub(crate) format: Format,
    pub(crate) auto: bool,
    // Quoted literal blocks: the quote character, messages, first line.
    pub(crate) quote: Option<char>,
    pub(crate) messages: Vec<crate::nodes::Element>,
    pub(crate) initial_lineno: usize,
}

impl Machine {
    /// A machine over `data[lo..]`, the first line at absolute offset
    /// `input_offset`.
    pub(crate) fn new(list: Lines, lo: usize, input_offset: isize) -> Self {
        let data = LineList::get(&list);
        Self {
            lo: lo.min(data.len()),
            list,
            data,
            input_offset,
            line_offset: -1,
            match_titles: false,
            top: false,
            blank_finish: true,
            last_ordinal: 0,
            format: Format::Period,
            auto: false,
            quote: None,
            messages: Vec::new(),
            initial_lineno: 0,
        }
    }

    /// A machine over a view from [`Machine::rest`] or
    /// [`Machine::current_on`].
    pub(crate) fn new_view((list, lo, offset): (Lines, usize, isize)) -> Self {
        Self::new(list, lo, offset)
    }

    /// A machine over `lines`, taken from no other list.
    pub(crate) fn detached(lines: Vec<Line>, input_offset: isize) -> Self {
        Self::new(LineList::new(lines, None), 0, input_offset)
    }

    /// The document's top-level machine; the document looks lines up in
    /// its list.
    pub(crate) fn top(doc: &mut Document, lines: Vec<Line>) -> Self {
        let mut m = Self::detached(lines, 0);
        doc.top_lines = Some(m.list.clone());
        m.match_titles = true;
        m.top = true;
        m
    }

    /// Catches up with lines inserted into the list.
    fn refresh(&mut self) {
        let lines = LineList::get(&self.list);
        if !Rc::ptr_eq(&lines, &self.data) {
            self.data = lines;
        }
    }

    /// Inserts `lines` from `source` after the current line, between
    /// padding lines (docutils' `insert_input`).
    pub(crate) fn insert_input(&mut self, lines: Vec<Line>, source: &str) {
        let span = lines.first().map(|l| l.span).unwrap_or_default();
        let blank = |what: &str, offset: isize| Line {
            text: String::new(),
            span,
            origin: crate::lines::LineOrigin {
                source: Some(Rc::from(format!("internal padding {what} {source}"))),
                offset,
            },
        };
        let mut new = vec![blank("before", -1)];
        let after = blank("after", lines.len() as isize);
        new.extend(lines);
        new.push(after);
        let index = self.lo + (self.line_offset + 1).max(0) as usize;
        LineList::insert(&self.list, index, &new);
        self.refresh();
    }

    /// The number of lines.
    pub(crate) fn lines_len(&self) -> isize {
        self.len()
    }

    fn len(&self) -> isize {
        (self.data.len() - self.lo) as isize
    }

    /// The line at `offset`, indexing like Python (negative from the end).
    pub(crate) fn get(&self, offset: isize) -> Option<&Line> {
        let len = self.len();
        let i = if offset < 0 { offset + len } else { offset };
        (0..len)
            .contains(&i)
            .then(|| &self.data[self.lo + i as usize])
    }

    /// The current line.
    pub(crate) fn line(&self) -> &Line {
        self.get(self.line_offset).expect("a current line")
    }

    /// The current line's text.
    pub(crate) fn text(&self) -> &str {
        &self.line().text
    }

    /// Lines `a..=b` of this machine, as far as they exist.
    pub(crate) fn lines(&self, a: isize, b: isize) -> Vec<Line> {
        (a.max(0)..=b)
            .filter_map(|i| self.get(i).cloned())
            .collect()
    }

    /// The span of lines `a..=b`, trailing blank lines left out.
    pub(crate) fn span(&self, a: isize, b: isize) -> Option<Span> {
        span_of(&self.lines(a, b))
    }

    /// The remaining lines after the current one, for a nested machine.
    pub(crate) fn rest(&self) -> (Lines, usize, isize) {
        let next = (self.line_offset + 1).max(0) as usize;
        (
            self.list.clone(),
            self.lo + next,
            self.abs_line_offset() + 1,
        )
    }

    /// The lines from the current one on.
    pub(crate) fn current_on(&self) -> (Lines, usize, isize) {
        let current = self.line_offset.max(0) as usize;
        (self.list.clone(), self.lo + current, self.abs_line_offset())
    }

    /// The source (`None` for the document) and line of the current line,
    /// as this machine finds them (docutils' `get_source_and_line()`).
    pub(crate) fn source_and_line(&self) -> (Option<Rc<str>>, Option<usize>) {
        let len = self.len();
        let off = self.line_offset;
        let (line, past_end) = match self.get(off) {
            Some(l) => (l, 0),
            None if off == len && len > 0 => (self.get(len - 1).unwrap(), 1),
            None => return (None, None),
        };
        let number = line.origin.offset + 1 + past_end;
        (line.origin.source.clone(), usize::try_from(number).ok())
    }

    pub(crate) fn abs_line_offset(&self) -> isize {
        self.line_offset + self.input_offset
    }

    pub(crate) fn abs_line_number(&self) -> usize {
        (self.line_offset + self.input_offset + 1).max(0) as usize
    }

    /// Tells the document where this machine is (docutils' observers).
    fn notify(&self, doc: &mut Document) {
        let len = self.len();
        let off = self.line_offset;
        let (source, line) = match self.get(off) {
            Some(l) => (l.origin.source.clone(), Some(l.origin.offset + 1)),
            // Just past the end: the last line's source, no line.
            None if off == len && len > 0 => {
                (self.get(len - 1).unwrap().origin.source.clone(), None)
            }
            None => (None, None),
        };
        doc.current_source = source;
        doc.current_line = line.and_then(|l| usize::try_from(l).ok());
        if self.top {
            doc.top_offset = off;
        }
    }

    /// Moves `n` lines on; `false` past the end.
    pub(crate) fn next_line(&mut self, doc: &mut Document, n: isize) -> bool {
        self.refresh();
        self.line_offset += n;
        self.notify(doc);
        self.get(self.line_offset).is_some()
    }

    /// Moves `n` lines back.
    pub(crate) fn previous_line(&mut self, doc: &mut Document, n: isize) {
        self.refresh();
        self.line_offset -= n;
        self.notify(doc);
    }

    /// Jumps to absolute line offset `abs`.
    pub(crate) fn goto_line(&mut self, doc: &mut Document, abs: isize) {
        self.refresh();
        self.line_offset = abs - self.input_offset;
        self.notify(doc);
    }

    /// Whether the next line is blank or there is none.
    pub(crate) fn is_next_line_blank(&self) -> bool {
        self.get(self.line_offset + 1)
            .is_none_or(|l| strip(&l.text).is_empty())
    }

    pub(crate) fn at_eof(&self) -> bool {
        self.line_offset >= self.len() - 1
    }

    /// A warning that `name` ends without a blank line.
    pub(crate) fn unindent_warning(&self, doc: &Document, name: &str) -> crate::nodes::Element {
        doc.message(
            WARNING,
            &format!("{name} ends without a blank line; unexpected unindent."),
            Vec::new(),
            At::Top(Some(self.abs_line_number() + 1)),
        )
    }

    // -- blocks -------------------------------------------------------------

    /// The text block from the current line to the next blank line; with
    /// `flush_left`, `Err` with the block so far and the line number of
    /// the first indented line. Moves to the block's last line.
    pub(crate) fn get_text_block(
        &mut self,
        doc: &mut Document,
        flush_left: bool,
    ) -> Result<Vec<Line>, (Vec<Line>, usize)> {
        let start = self.line_offset;
        let mut end = start;
        let mut indented = None;
        while let Some(line) = self.get(end).filter(|_| end >= 0) {
            if strip(&line.text).is_empty() {
                break;
            }
            if flush_left && line.text.starts_with(' ') {
                indented = Some((self.input_offset + end + 1) as usize);
                break;
            }
            end += 1;
        }
        let block = self.lines(start, end - 1);
        self.next_line(doc, block.len() as isize - 1);
        match indented {
            Some(line) => Err((block, line)),
            None => Ok(block),
        }
    }

    /// docutils' `StringList.get_indented`, from line `start`.
    fn indented_lines(
        &self,
        start: isize,
        until_blank: bool,
        strip_indent: bool,
        block_indent: Option<usize>,
        first_indent: Option<usize>,
    ) -> (Vec<Line>, usize, bool) {
        let mut indent = block_indent;
        let first_indent = first_indent.or(block_indent);
        let mut end = start;
        if first_indent.is_some() {
            end += 1;
        }
        let mut blank_finish = true;
        while let Some(line) = self.get(end).filter(|_| end >= 0) {
            let text = line.text.as_str();
            let short_indent =
                block_indent.is_some_and(|n| !strip(crate::util::take_chars(text, n)).is_empty());
            if !text.is_empty() && (!text.starts_with(' ') || short_indent) {
                blank_finish =
                    end > start && self.get(end - 1).is_some_and(|l| strip(&l.text).is_empty());
                break;
            }
            let stripped = lstrip(text);
            if stripped.is_empty() {
                if until_blank {
                    blank_finish = true;
                    break;
                }
            } else if block_indent.is_none() {
                let line_indent = char_len(text) - char_len(stripped);
                indent = Some(indent.map_or(line_indent, |i| i.min(line_indent)));
            }
            end += 1;
        }
        let mut block = self.lines(start, end - 1);
        if let (Some(n), Some(first)) = (first_indent, block.first_mut()) {
            *first = trim_line(first, n);
        }
        let indent = indent.unwrap_or(0);
        if indent > 0 && strip_indent {
            let skip = usize::from(first_indent.is_some());
            for line in block.iter_mut().skip(skip) {
                *line = trim_line(line, indent);
            }
        }
        (block, indent, blank_finish)
    }

    /// An indented block of unknown indent, from the current line: the
    /// lines, the indent, the absolute offset of the first line, and
    /// whether it ended with a blank line.
    pub(crate) fn get_indented(
        &mut self,
        doc: &mut Document,
        until_blank: bool,
        strip_indent: bool,
    ) -> (Vec<Line>, usize, isize, bool) {
        let mut offset = self.abs_line_offset();
        let (mut block, indent, blank_finish) =
            self.indented_lines(self.line_offset, until_blank, strip_indent, None, None);
        if !block.is_empty() {
            self.next_line(doc, block.len() as isize - 1);
        }
        while block.first().is_some_and(|l| strip(&l.text).is_empty()) {
            block.remove(0);
            offset += 1;
        }
        (block, indent, offset, blank_finish)
    }

    /// An indented block whose lines all have at least `indent` columns.
    pub(crate) fn get_known_indented(
        &mut self,
        doc: &mut Document,
        indent: usize,
    ) -> (Vec<Line>, isize, bool) {
        let mut offset = self.abs_line_offset();
        let (mut block, _, blank_finish) =
            self.indented_lines(self.line_offset, false, true, Some(indent), None);
        self.next_line(doc, block.len() as isize - 1);
        while block.first().is_some_and(|l| strip(&l.text).is_empty()) {
            block.remove(0);
            offset += 1;
        }
        (block, offset, blank_finish)
    }

    /// An indented block whose first line's indent is `indent` (the rest
    /// unknown).
    pub(crate) fn get_first_known_indented(
        &mut self,
        doc: &mut Document,
        indent: usize,
        until_blank: bool,
        strip_indent: bool,
        strip_top: bool,
    ) -> (Vec<Line>, usize, isize, bool) {
        let mut offset = self.abs_line_offset();
        let (mut block, indent, blank_finish) = self.indented_lines(
            self.line_offset,
            until_blank,
            strip_indent,
            None,
            Some(indent),
        );
        self.next_line(doc, block.len() as isize - 1);
        if strip_top {
            while block.first().is_some_and(|l| strip(&l.text).is_empty()) {
                block.remove(0);
                offset += 1;
            }
        }
        (block, indent, offset, blank_finish)
    }

    // -- running --------------------------------------------------------------

    /// The transitions tried in `state`, in order.
    fn transitions(&self, state: State) -> &'static [Tr] {
        match state {
            State::Text | State::Definition | State::Line => TEXT,
            State::SubstitutionDef => SUBSTITUTION,
            State::QuotedLiteralBlock if self.quote.is_some() => QUOTE_REST,
            State::QuotedLiteralBlock => QUOTE_FIRST,
            _ => BODY,
        }
    }

    /// Whether transition `tr`'s pattern matches `s`.
    fn matches(&self, tr: Tr, s: &[char]) -> bool {
        use crate::patterns as p;
        match tr {
            Tr::Blank => p::blank(s),
            Tr::Indent => p::indent(s),
            Tr::Bullet => p::bullet(s).is_some(),
            Tr::Enumerator => p::enumerator(s).is_some(),
            Tr::FieldMarker => p::field_marker(s).is_some(),
            Tr::OptionMarker => p::option_marker(s).is_some(),
            Tr::Doctest => p::doctest(s).is_some(),
            Tr::LineBlock => p::line_block(s).is_some(),
            Tr::GridTableTop => super::table::is_grid_top(s),
            Tr::SimpleTableTop => super::table::is_simple_top(s),
            Tr::ExplicitMarkup => p::explicit_markup(s).is_some(),
            Tr::Anonymous => p::anonymous(s).is_some(),
            Tr::Line | Tr::Underline => p::line(s),
            Tr::Text => true,
            Tr::EmbeddedDirective => p::embedded_directive(s).is_some(),
            Tr::InitialQuoted => s.first().is_some_and(|&c| p::is_punctuation(c)),
            Tr::Quoted => self.quote.is_some_and(|q| s.first() == Some(&q)),
        }
    }

    /// Runs the machine from `state` until its input ends or a state ends
    /// it. Elements go into the document's current parent.
    pub(crate) fn run(&mut self, doc: &mut Document, mut state: State) {
        self.line_offset = -1;
        let mut context: Vec<Line> = Vec::new();
        let mut only: Option<Tr> = None;
        loop {
            let result = if self.next_line(doc, 1) {
                let chars: Vec<char> = self.text().chars().collect();
                let candidates: Vec<Tr> = match only {
                    Some(tr) => vec![tr],
                    None => self.transitions(state).to_vec(),
                };
                match candidates.into_iter().find(|&tr| self.matches(tr, &chars)) {
                    Some(tr) => self.transition(doc, state, tr, &mut context),
                    None => Ok(state),
                }
            } else {
                Err(Flow::Eof)
            };
            let result = match result {
                Err(Flow::Eof) => match self.eof(doc, state, &mut context) {
                    Ok(()) => return,
                    Err(flow) => Err(flow),
                },
                other => other,
            };
            match result {
                Ok(next) => {
                    state = next;
                    only = None;
                }
                Err(Flow::TransitionCorrection(tr)) => {
                    self.previous_line(doc, 1);
                    only = Some(tr);
                }
                Err(Flow::StateCorrection(next, tr)) => {
                    self.previous_line(doc, 1);
                    state = next;
                    only = tr;
                }
                Err(Flow::Eof) => return,
            }
        }
    }

    /// Runs a nested machine over `lines`, taken from this machine's
    /// lines at absolute offset `offset`, from state Body, into the
    /// document's current parent. Returns the new absolute line offset.
    /// Lines inserted meanwhile are skipped here too (docutils'
    /// `nested_parse`).
    pub(crate) fn nested_parse(
        &mut self,
        doc: &mut Document,
        lines: Vec<Line>,
        offset: isize,
    ) -> isize {
        let index = offset - (self.input_offset - self.lo as isize);
        let parent = Some((self.list.clone(), index));
        self.nested_parse_in(doc, lines, offset, parent)
    }

    /// [`Machine::nested_parse`] for lines not taken from this machine's
    /// (table cells).
    pub(crate) fn nested_parse_detached(
        &mut self,
        doc: &mut Document,
        lines: Vec<Line>,
        offset: isize,
    ) -> isize {
        self.nested_parse_in(doc, lines, offset, None)
    }

    fn nested_parse_in(
        &mut self,
        doc: &mut Document,
        lines: Vec<Line>,
        offset: isize,
        parent: Option<(Lines, isize)>,
    ) -> isize {
        let len = lines.len() as isize;
        let connected = parent.is_some();
        let list = LineList::new(lines, parent);
        let mut m = Machine::new(list.clone(), 0, offset);
        m.run(doc, State::Body);
        let new_offset = m.abs_line_offset();
        let added = LineList::get(&list).len() as isize - len;
        self.refresh();
        if connected && added != 0 {
            self.next_line(doc, added);
        }
        new_offset
    }

    /// Runs a nested machine for the rest of a list (or other compound
    /// construct) from `state`. Returns the new absolute line offset and
    /// whether the construct ended with a blank line.
    pub(crate) fn nested_list_parse(
        &self,
        doc: &mut Document,
        (list, lo, offset): (Lines, usize, isize),
        state: State,
        blank_finish: bool,
        setup: impl FnOnce(&mut Machine),
    ) -> (isize, bool) {
        let mut m = Machine::new(list, lo, offset);
        m.blank_finish = blank_finish;
        setup(&mut m);
        m.run(doc, state);
        (m.abs_line_offset(), m.blank_finish)
    }

    /// Ends the input in `state`.
    fn eof(
        &mut self,
        doc: &mut Document,
        state: State,
        context: &mut Vec<Line>,
    ) -> Result<(), Flow> {
        match state {
            State::Text => {
                if !context.is_empty() {
                    self.text_blank(doc, context)?;
                }
                Ok(())
            }
            State::Definition => {
                self.previous_line(doc, 2);
                Ok(())
            }
            State::Line => self.line_eof(doc, context),
            State::QuotedLiteralBlock => {
                self.quoted_eof(doc, context);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Not part of the construct being parsed: back up, end the machine.
    pub(crate) fn invalid_input(&mut self, doc: &mut Document) -> Step {
        self.previous_line(doc, 1);
        Err(Flow::Eof)
    }

    /// Calls the method for transition `tr` in `state`.
    fn transition(
        &mut self,
        doc: &mut Document,
        state: State,
        tr: Tr,
        context: &mut Vec<Line>,
    ) -> Step {
        use State as S;
        match (state, tr) {
            // Text states.
            (S::Text, Tr::Blank) => self.text_blank(doc, context),
            (S::Text, Tr::Indent) => self.text_indent(doc, context),
            (S::Text, Tr::Underline) => self.text_underline(doc, context),
            (S::Text, Tr::Text) => self.text_text(doc, context),
            (S::Definition, Tr::Indent) => self.definition_indent(doc, context),
            (S::Definition, _) => Err(Flow::Eof),
            (S::Line, Tr::Blank) => self.line_blank(doc, context),
            (S::Line, Tr::Indent | Tr::Text) => self.line_text(doc, context),
            (S::Line, Tr::Underline) => self.line_underline(doc, context),
            (S::QuotedLiteralBlock, _) => self.quoted(doc, tr, context),
            // Whitespace.
            (S::LineBlock | S::Explicit, Tr::Blank) => self.invalid_input(doc),
            (_, Tr::Blank) => Ok(state),
            (S::Body | S::SubstitutionDef, Tr::Indent) => self.indent(doc, state),
            // Body and the states continuing its compound constructs.
            (S::Body, Tr::Bullet) => self.bullet(doc, state),
            (S::BulletList, Tr::Bullet) => self.bullet_list_item(doc, state),
            (S::Body, Tr::Enumerator) => self.enumerator(doc, state),
            (S::EnumeratedList, Tr::Enumerator) => self.enumerated_list_item(doc, state),
            (S::Body, Tr::FieldMarker) => self.field_marker(doc, state),
            (S::FieldList | S::ExtensionOptions, Tr::FieldMarker) => {
                self.field_list_item(doc, state)
            }
            (S::MetaBody, Tr::FieldMarker) => self.meta_field(doc, state),
            (S::Body, Tr::OptionMarker) => self.option_marker(doc, state),
            (S::OptionList, Tr::OptionMarker) => self.option_list_next(doc, state),
            (S::Body, Tr::Doctest) => self.doctest(doc, state),
            (S::Body, Tr::LineBlock) => self.line_block(doc, state),
            (S::LineBlock, Tr::LineBlock) => self.line_block_next(doc, state),
            (S::Body, Tr::GridTableTop) => self.table_top(doc, state, true),
            (S::Body, Tr::SimpleTableTop) => self.table_top(doc, state, false),
            (S::Body, Tr::ExplicitMarkup) => self.explicit_markup(doc, state),
            (S::Explicit, Tr::ExplicitMarkup) => self.explicit_next(doc, state),
            (S::Body, Tr::Anonymous) => self.anonymous(doc, state),
            (S::Explicit, Tr::Anonymous) => self.anonymous_next(doc, state),
            (S::Body, Tr::Line) => self.punctuation_line(doc, state, context),
            (S::Body, Tr::Text) => {
                *context = vec![self.line().clone()];
                Ok(S::Text)
            }
            (S::DefinitionList, Tr::Text) => {
                *context = vec![self.line().clone()];
                Ok(S::Definition)
            }
            (S::SubstitutionDef, Tr::EmbeddedDirective) => self.embedded_directive(doc),
            (S::SubstitutionDef, Tr::Text) => {
                if !self.at_eof() {
                    self.blank_finish = self.is_next_line_blank();
                }
                Err(Flow::Eof)
            }
            _ => self.invalid_input(doc),
        }
    }
}

/// `line` without its first `n` characters. The span moves with them
/// (exact unless the removed characters came from tabs).
pub(crate) fn trim_line(line: &Line, n: usize) -> Line {
    let text = skip_chars(&line.text, n);
    let removed = line.text.len() - text.len();
    Line {
        text: text.to_owned(),
        span: Span::new(
            (line.span.start + removed).min(line.span.end),
            line.span.end,
        ),
        origin: line.origin.clone(),
    }
}

/// The span covering `lines`, trailing blank lines left out.
pub(crate) fn span_of(lines: &[Line]) -> Option<Span> {
    let last = lines.iter().rposition(|l| !l.text.trim().is_empty())?;
    Some(lines[0].span.to(lines[last].span))
}

/// `lines`' texts joined with line breaks.
pub(crate) fn join(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}
