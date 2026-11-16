//! Inline markup: a port of docutils' `Inliner`.
//!
//! Text is scanned for start-strings (emphasis, strong, literal, inline
//! targets, substitution references, interpreted text) and whole
//! constructs (references, footnote and citation references); each is
//! checked against the recognition rules, and its end-string searched
//! for. What is left is searched for standalone URIs.
//!
//! As in docutils, backslash escapes are turned into nulls first
//! ([`escape2null`]) and Text nodes keep them; the parser removes them
//! from every Text node when it finishes (docutils does so when printing).

use super::document::{At, Document, MsgNode, NoteList, RAW, WARNING};
use super::roles;
use crate::lines::{Line, is_python_whitespace};
use crate::nodes::{Element, Kind, Node, Span, Text};
use crate::patterns::{is_email, simplename_ends};
use crate::tables;
use crate::util::{
    escape2null, in_table, is_space, normalize_name, restore_backslashes, split_escaped_whitespace,
    split_whitespace, unescape, whitespace_normalize_name,
};

/// Whether `c1` and `c2` are a matching open/close pair (docutils'
/// `punctuation_chars.match_chars`).
fn match_chars(c1: char, c2: char) -> bool {
    let Some(i) = tables::OPENERS.iter().position(|&c| c == c1) else {
        return false;
    };
    tables::CLOSERS.get(i) == Some(&c2)
        || tables::QUOTE_PAIRS
            .iter()
            .any(|&(o, cs)| o == c1 && cs.contains(c2))
}

/// Where each character of a text came from: a byte offset per character,
/// plus one for the end.
#[derive(Debug, Clone, Default)]
pub(crate) struct Origin(Vec<usize>);

impl Origin {
    /// The origin of `lines` joined with line breaks.
    pub(crate) fn of(lines: &[Line]) -> Self {
        let mut offsets = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                offsets.push(lines[i - 1].span.end);
            }
            for (b, _) in line.text.char_indices() {
                offsets.push((line.span.start + b).min(line.span.end));
            }
        }
        offsets.push(lines.last().map_or(0, |l| l.span.end));
        Origin(offsets)
    }

    /// The origin of the text from character `n` on.
    pub(crate) fn skip(mut self, n: usize) -> Self {
        let n = n.min(self.0.len().saturating_sub(1));
        self.0.drain(..n);
        self
    }

    fn span(&self, a: usize, b: usize) -> Option<Span> {
        let last = self.0.len().checked_sub(1)?;
        let start = self.0[a.min(last)];
        let end = self.0[b.min(last)].max(start);
        Some(Span::new(start, end))
    }
}

/// Nodes and system messages from inline text.
pub(crate) type Inlines = (Vec<Node>, Vec<Element>);

/// Parses inline `text` (docutils' `Inliner.parse`). Messages refer to
/// line `lineno`; `origin` maps characters back to the source.
pub(crate) fn inline_text(
    doc: &mut Document,
    text: &str,
    lineno: usize,
    origin: &Origin,
) -> Inlines {
    let mut inliner = Inliner {
        doc,
        lineno,
        origin,
    };
    inliner.parse(text)
}

/// The kind of construct an initial match found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum K {
    Strong,
    Emphasis,
    Literal,
    Target,
    Substitution,
    Interpreted,
    Reference,
    Anonymous,
    /// A footnote or citation reference: the label's start and end, and
    /// whether it is a citation.
    Footnote(usize, usize, bool),
}

/// A match of docutils' `initial` pattern. Offsets are in characters of
/// the searched text.
#[derive(Debug, Clone, Copy)]
struct Start {
    kind: K,
    /// Where the whole match starts (the role, for interpreted text).
    start: usize,
    /// The start-string (or whole construct).
    group: (usize, usize),
    /// The role prefix, colons included.
    role: Option<(usize, usize)>,
}

/// An end-string match.
#[derive(Debug, Clone, Copy)]
struct End {
    /// The end-string.
    group: (usize, usize),
    /// Interpreted text: a suffix role, colons included, and the reference
    /// suffix.
    role: Option<(usize, usize)>,
}

fn text_of(s: &[char], a: usize, b: usize) -> String {
    s[a.min(s.len())..b.min(s.len())].iter().collect()
}

/// `(^|(?<=\s|[openers delimiters]))` at `i`.
fn start_ok(s: &[char], i: usize) -> bool {
    i == 0 || in_table(s[i - 1], tables::START_PRECEDING)
}

/// `($|(?=\s|[\x00 closing_delimiters delimiters closers]))` at `p`.
fn end_ok(s: &[char], p: usize) -> bool {
    p >= s.len() || in_table(s[p], tables::END_FOLLOWING)
}

/// `(?!\s)` at `p`.
fn not_space_after(s: &[char], p: usize) -> bool {
    p >= s.len() || !is_space(s[p])
}

/// `(?<![\s\x00])` at `p`.
fn not_space_or_null_before(s: &[char], p: usize) -> bool {
    p == 0 || !(is_space(s[p - 1]) || s[p - 1] == '\0')
}

/// Simplename ends at `p`, using Python's `\w`.
fn names(s: &[char], p: usize) -> Vec<usize> {
    simplename_ends(s, p)
}

/// docutils' `initial` pattern, tried at `i`.
fn initial_at(s: &[char], i: usize) -> Option<Start> {
    if !start_ok(s, i) {
        return None;
    }
    let at = |p: usize, t: &str| t.chars().enumerate().all(|(k, c)| s.get(p + k) == Some(&c));
    let simple = |kind, len: usize| Start {
        kind,
        start: i,
        group: (i, i + len),
        role: None,
    };
    // Simple start-strings, followed by non-whitespace.
    let starts: [(bool, K, usize); 5] = [
        (at(i, "**"), K::Strong, 2),
        (at(i, "*") && !at(i + 1, "*"), K::Emphasis, 1),
        (at(i, "``"), K::Literal, 2),
        (at(i, "_`"), K::Target, 2),
        (at(i, "|") && !at(i + 1, "|"), K::Substitution, 1),
    ];
    for (ok, kind, len) in starts {
        if ok && not_space_after(s, i + len) {
            return Some(simple(kind, len));
        }
    }
    // Whole constructs, followed by an end-string suffix.
    for e in names(s, i) {
        if s.get(e) == Some(&'_') {
            if s.get(e + 1) == Some(&'_') && end_ok(s, e + 2) {
                return Some(simple(K::Anonymous, e + 2 - i));
            }
            if end_ok(s, e + 1) {
                return Some(simple(K::Reference, e + 1 - i));
            }
        }
    }
    if s.get(i) == Some(&'[') {
        let q = i + 1;
        let mut candidates: Vec<(usize, bool)> = Vec::new();
        let digits = s[q..].iter().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 {
            candidates.push((q + digits, false));
        }
        if s.get(q) == Some(&'#') {
            candidates.extend(names(s, q + 1).into_iter().map(|e| (e, false)));
            candidates.push((q + 1, false));
        }
        if s.get(q) == Some(&'*') {
            candidates.push((q + 1, false));
        }
        candidates.extend(names(s, q).into_iter().map(|e| (e, true)));
        for (e, citation) in candidates {
            if at(e, "]_") && end_ok(s, e + 2) {
                return Some(Start {
                    kind: K::Footnote(q, e, citation),
                    start: i,
                    group: (i, e + 2),
                    role: None,
                });
            }
        }
    }
    // Interpreted text or a phrase reference, with an optional role.
    if s.get(i) == Some(&':') {
        for e in names(s, i + 1) {
            if s.get(e) == Some(&':')
                && s.get(e + 1) == Some(&'`')
                && s.get(e + 2) != Some(&'`')
                && not_space_after(s, e + 2)
            {
                return Some(Start {
                    kind: K::Interpreted,
                    start: i,
                    group: (e + 1, e + 2),
                    role: Some((i, e + 1)),
                });
            }
        }
    }
    if s.get(i) == Some(&'`') && s.get(i + 1) != Some(&'`') && not_space_after(s, i + 1) {
        return Some(simple(K::Interpreted, 1));
    }
    None
}

/// Searches for an end-string in `sub` (docutils' end patterns).
fn find_end(kind: K, sub: &[char]) -> Option<End> {
    let at = |p: usize, t: &str| {
        t.chars()
            .enumerate()
            .all(|(k, c)| sub.get(p + k) == Some(&c))
    };
    let plain = |a: usize, b: usize| End {
        group: (a, b),
        role: None,
    };
    for j in 0..sub.len() {
        match kind {
            K::Emphasis if at(j, "*") && not_space_or_null_before(sub, j) && end_ok(sub, j + 1) => {
                return Some(plain(j, j + 1));
            }
            K::Strong if at(j, "**") && not_space_or_null_before(sub, j) && end_ok(sub, j + 2) => {
                return Some(plain(j, j + 2));
            }
            K::Literal
                if at(j, "``") && (j == 0 || !is_space(sub[j - 1])) && end_ok(sub, j + 2) =>
            {
                return Some(plain(j, j + 2));
            }
            K::Target if at(j, "`") && not_space_or_null_before(sub, j) && end_ok(sub, j + 1) => {
                return Some(plain(j, j + 1));
            }
            K::Substitution if at(j, "|") && not_space_or_null_before(sub, j) => {
                for n in [2, 1, 0] {
                    if (1..=n).all(|k| sub.get(j + k) == Some(&'_')) && end_ok(sub, j + 1 + n) {
                        return Some(plain(j, j + 1 + n));
                    }
                }
            }
            K::Interpreted if at(j, "`") => {
                // (?<!(?<!\x00)[\s\x00])
                if j > 0
                    && (is_space(sub[j - 1]) || sub[j - 1] == '\0')
                    && !(j > 1 && sub[j - 2] == '\0')
                {
                    continue;
                }
                let mut roles: Vec<(Option<(usize, usize)>, usize)> = Vec::new();
                if sub.get(j + 1) == Some(&':') {
                    for e in names(sub, j + 2) {
                        if sub.get(e) == Some(&':') {
                            roles.push((Some((j + 1, e + 1)), e + 1));
                        }
                    }
                }
                roles.push((None, j + 1));
                for (role, p) in roles {
                    for n in [2, 1, 0] {
                        if (0..n).all(|k| sub.get(p + k) == Some(&'_')) && end_ok(sub, p + n) {
                            return Some(End {
                                group: (j, p + n),
                                role,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// `[-_.!~*'()[\];/:@&=+$,%a-zA-Z0-9\x00]`
fn is_uric(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-_.!~*'()[];/:@&=+$,%\0".contains(c)
}

/// `[_~*/=+a-zA-Z0-9]`
fn is_urilast(c: char) -> bool {
    c.is_ascii_alphanumeric() || "_~*/=+".contains(c)
}

/// `[-_!~*'{|}/#?^`&=+$%a-zA-Z0-9\x00]`
fn is_emailc(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-_!~*'{|}/#?^`&=+$%\0".contains(c)
}

/// `uri_end` at `x`: a final URI character.
fn uri_end_at(s: &[char], x: usize) -> bool {
    s.get(x)
        .is_some_and(|&c| is_urilast(c) || (is_uric(c) && s.get(x + 1) == Some(&'>')))
}

/// The ends of `uric* uri_end` from `p`, longest first.
fn uric_run_ends(s: &[char], p: usize) -> Vec<usize> {
    let run = s[p.min(s.len())..]
        .iter()
        .take_while(|&&c| is_uric(c))
        .count();
    (p..p + run)
        .rev()
        .filter(|&x| uri_end_at(s, x))
        .map(|x| x + 1)
        .collect()
}

/// A standalone URI or email address at `i`: its end, and whether it is
/// an email address (and so where its scheme ends, for absolute URIs).
fn uri_at(s: &[char], i: usize) -> Option<(usize, Option<usize>)> {
    // Absolute URI: scheme ":" hierarchical part, query, fragment.
    if s.get(i).is_some_and(char::is_ascii_alphabetic) {
        let scheme_end = i
            + 1
            + s[i + 1..]
                .iter()
                .take_while(|&&c| c.is_ascii_alphanumeric() || ".+-".contains(c))
                .count();
        if s.get(scheme_end) == Some(&':') {
            let q = scheme_end + 1;
            let mut starts = Vec::new();
            if s.get(q) == Some(&'/') && s.get(q + 1) == Some(&'/') {
                starts.push(q + 2);
            }
            if s.get(q) == Some(&'/') {
                starts.push(q + 1);
            }
            starts.push(q);
            for p in starts {
                for e1 in uric_run_ends(s, p) {
                    let mut e2s = Vec::new();
                    if s.get(e1) == Some(&'?') {
                        e2s.extend(uric_run_ends(s, e1 + 1));
                    }
                    e2s.push(e1);
                    for e2 in e2s {
                        let mut e3s = Vec::new();
                        if s.get(e2) == Some(&'#') {
                            e3s.extend(uric_run_ends(s, e2 + 1));
                        }
                        e3s.push(e2);
                        for e3 in e3s {
                            if end_ok(s, e3) {
                                return Some((e3, Some(scheme_end)));
                            }
                        }
                    }
                }
            }
        }
    }
    // Email address.
    let mut k = i;
    loop {
        let run = s[k.min(s.len())..]
            .iter()
            .take_while(|&&c| is_emailc(c))
            .count();
        if run == 0 {
            return None;
        }
        k += run;
        if s.get(k) == Some(&'.') {
            k += 1;
        } else {
            break;
        }
    }
    if s.get(k) != Some(&'@') || s[k - 1] == '\0' {
        return None;
    }
    let h = k + 1;
    if !s.get(h).is_some_and(|&c| is_emailc(c)) {
        return None;
    }
    let host_end = h + s[h..]
        .iter()
        .take_while(|&&c| is_emailc(c) || c == '.')
        .count();
    for y in (h + 1..=host_end).rev() {
        if uri_end_at(s, y) && end_ok(s, y + 1) {
            return Some((y + 1, None));
        }
    }
    None
}

/// docutils' `uri` pattern searched in `s`: start, end, scheme end.
fn find_uri(s: &[char]) -> Option<(usize, usize, Option<usize>)> {
    (0..s.len()).find_map(|i| {
        if !start_ok(s, i) {
            return None;
        }
        uri_at(s, i).map(|(end, scheme)| (i, end, scheme))
    })
}

/// The number made of the digits `s[a..b]`, if it fits.
fn number(s: &[char], a: usize, b: usize) -> Option<i64> {
    text_of(s, a, b).parse().ok()
}

/// The end of the run of digits (Python's `\d`, ASCII only here) at `i`.
fn digits_end(s: &[char], i: usize) -> usize {
    i + s[i.min(s.len())..]
        .iter()
        .take_while(|c| c.is_ascii_digit())
        .count()
}

/// docutils' `pep` pattern searched in `s`: start, end and PEP number.
/// `pep-(\d+)(.txt)?` or `PEP\s+(\d+)`, between inline markup
/// boundaries; the regex backtracks, so `(.txt)?` may take a digit.
fn find_pep(s: &[char]) -> Option<(usize, usize, i64)> {
    let at = |i: usize, word: &str| {
        word.chars()
            .enumerate()
            .all(|(k, c)| s.get(i + k) == Some(&c))
    };
    (0..s.len()).find_map(|i| {
        if !start_ok(s, i) {
            return None;
        }
        if at(i, "pep-") {
            let d = i + 4;
            let end = digits_end(s, d);
            for e in (d + 1..=end).rev() {
                // `(.txt)?`: any character but a line break, then `txt`.
                if s.get(e).is_some_and(|&c| c != '\n') && at(e + 1, "txt") && end_ok(s, e + 4) {
                    return number(s, d, e).map(|n| (i, e + 4, n));
                }
                if end_ok(s, e) {
                    return number(s, d, e).map(|n| (i, e, n));
                }
            }
        } else if at(i, "PEP") {
            let spaces = s[i + 3..].iter().take_while(|&&c| is_space(c)).count();
            let d = i + 3 + spaces;
            let end = digits_end(s, d);
            if spaces > 0 && end > d && end_ok(s, end) {
                return number(s, d, end).map(|n| (i, end, n));
            }
        }
        None
    })
}

/// docutils' `rfc` pattern searched in `s`: start, end and RFC number.
/// `RFC(-|\s+)?(\d+)` between inline markup boundaries.
fn find_rfc(s: &[char]) -> Option<(usize, usize, i64)> {
    (0..s.len()).find_map(|i| {
        if !start_ok(s, i) || s.get(i..i + 3) != Some(&['R', 'F', 'C'][..]) {
            return None;
        }
        let after = i + 3;
        let separator = if s.get(after) == Some(&'-') {
            1
        } else {
            s[after..].iter().take_while(|&&c| is_space(c)).count()
        };
        // With the separator, then without it.
        for d in [after + separator, after] {
            let end = digits_end(s, d);
            if end > d && end_ok(s, end) {
                return number(s, d, end).map(|n| (i, end, n));
            }
        }
        None
    })
}

/// Whether a URI matches at the start of `s` (docutils' `uri.match`).
fn uri_matches(s: &[char]) -> bool {
    uri_at(s, 0).is_some()
}

/// An embedded `<target>` at the end of a phrase reference: the start of
/// the match (spaces before the bracket included) and the bracketed text.
fn embedded_link(s: &[char]) -> Option<(usize, usize, usize)> {
    let n = s.len();
    if n < 3 || s[n - 1] != '>' || is_space(s[n - 2]) || s[n - 2] == '\0' {
        return None;
    }
    let mut k = n - 1;
    let open = loop {
        if k == 0 {
            return None;
        }
        k -= 1;
        let escaped = k > 0 && s[k - 1] == '\0';
        match s[k] {
            '<' if !escaped => break k,
            '>' if !escaped => return None,
            _ => {}
        }
    };
    if open + 1 >= n - 1 || is_space(s[open + 1]) {
        return None;
    }
    let start = if open == 0 {
        0
    } else {
        let spaces = s[..open]
            .iter()
            .rev()
            .take_while(|&&c| c == ' ' || c == '\n')
            .count();
        if spaces == 0 {
            return None;
        }
        open - spaces
    };
    Some((start, open + 1, n - 1))
}

/// An element of `kind` holding `text` (none if empty).
pub(crate) fn element(kind: Kind, text: &str, span: Option<Span>) -> Element {
    let mut e = Element::new(kind);
    e.span = span;
    if !text.is_empty() {
        e.children.push(Text::new(text, span).into());
    }
    e
}

/// A node's text with escapes removed, as docutils' `astext()` gives it.
pub(crate) fn astext(e: &Element) -> String {
    e.children
        .iter()
        .map(|c| match c {
            Node::Text(t) => unescape(&t.value),
            Node::Element(e) => astext(e),
        })
        .collect()
}

/// What an inline construct method returns: the text before it, the
/// nodes, the text after, messages, and the end-string.
struct Found {
    before: usize,
    nodes: Vec<Node>,
    after: usize,
    messages: Vec<Element>,
    end_string: String,
}

pub(crate) struct Inliner<'a> {
    pub(crate) doc: &'a mut Document,
    pub(crate) lineno: usize,
    origin: &'a Origin,
}

impl Inliner<'_> {
    /// A span for characters `a..b` of the text, `base` being where the
    /// searched string starts.
    fn span(&self, base: usize, a: usize, b: usize) -> Option<Span> {
        self.origin.span(base + a, base + b)
    }

    fn parse(&mut self, text: &str) -> Inlines {
        let full: Vec<char> = escape2null(text).chars().collect();
        let mut base = 0;
        let mut remaining: Vec<char> = full.clone();
        // Parsed nodes live in the document until the parse ends, so a
        // duplicate target can find an earlier one.
        let saved = std::mem::take(&mut self.doc.scratch);
        let mut unprocessed: Vec<(usize, Vec<char>)> = Vec::new();
        let mut messages = Vec::new();
        while !remaining.is_empty() {
            let Some(m) = (0..remaining.len()).find_map(|i| initial_at(&remaining, i)) else {
                break;
            };
            let found = self.dispatch(&remaining, base, m);
            unprocessed.push((base, remaining[..found.before].to_vec()));
            messages.extend(found.messages);
            let new_base = base + found.after;
            let rest = remaining[found.after..].to_vec();
            if !found.nodes.is_empty() {
                let (start, chars) = join_unprocessed(&mut unprocessed);
                let text = self.implicit_inline(&chars, start);
                self.doc.scratch.extend(text);
                self.doc.scratch.extend(found.nodes);
            }
            base = new_base;
            remaining = rest;
        }
        let (start, mut chars) = join_unprocessed(&mut unprocessed);
        let start = if chars.is_empty() { base } else { start };
        chars.extend(remaining);
        if !chars.is_empty() {
            let text = self.implicit_inline(&chars, start);
            self.doc.scratch.extend(text);
        }
        let processed = std::mem::replace(&mut self.doc.scratch, saved);
        (processed, messages)
    }

    fn dispatch(&mut self, s: &[char], base: usize, m: Start) -> Found {
        match m.kind {
            K::Emphasis => self.inline_obj(s, base, m, Kind::Emphasis, false),
            K::Strong => self.inline_obj(s, base, m, Kind::Strong, false),
            K::Literal => self.inline_obj(s, base, m, Kind::Literal, true),
            K::Target => self.inline_internal_target(s, base, m),
            K::Substitution => self.substitution_reference(s, base, m),
            K::Interpreted => self.interpreted_or_phrase_ref(s, base, m),
            K::Reference => self.reference(s, base, m, false),
            K::Anonymous => self.reference(s, base, m, true),
            K::Footnote(q, e, citation) => self.footnote_reference(s, base, m, (q, e, citation)),
        }
    }

    /// Whether a start-string is quoted: between a matching pair of
    /// opening and closing characters, or at the end of the text.
    fn quoted_start(s: &[char], m: &Start) -> bool {
        if m.start == 0 {
            return false;
        }
        let pre = s[m.start - 1];
        match s.get(m.group.1) {
            None => true,
            Some(&post) => match_chars(pre, post),
        }
    }

    /// A message (`level`) for this text's line.
    fn message(&self, level: i64, text: &str) -> Element {
        self.doc
            .message(level, text, Vec::new(), At::Top(Some(self.lineno)))
    }

    /// A `problematic` element for `text`, linked with `msg` both ways.
    pub(crate) fn problematic(
        &mut self,
        text: &str,
        msg: &mut Element,
        span: Option<Span>,
    ) -> Element {
        let msgid = self.doc.set_id(msg, Some(MsgNode::Parent), None);
        let mut prb = element(Kind::Problematic, text, span);
        prb.attributes.set("refid", msgid);
        let prbid = self.doc.set_id(&mut prb, None, None);
        msg.attributes.push("backrefs", prbid);
        prb
    }

    /// Emphasis, strong, literal, targets and substitution references: a
    /// start-string, text, an end-string.
    fn inline_obj(
        &mut self,
        s: &[char],
        base: usize,
        m: Start,
        kind: Kind,
        restore: bool,
    ) -> Found {
        let (match_start, match_end) = m.group;
        if Self::quoted_start(s, &m) {
            return Found {
                before: match_end,
                nodes: Vec::new(),
                after: match_end,
                messages: Vec::new(),
                end_string: String::new(),
            };
        }
        if let Some(end) = find_end(m.kind, &s[match_end..])
            && end.group.0 > 0
        {
            let mut text = text_of(s, match_end, match_end + end.group.0);
            if restore {
                text = restore_backslashes(&text);
            }
            let text_end = match_end + end.group.1;
            let mut node = element(kind, &text, self.span(base, match_start, text_end));
            node.attributes
                .set(RAW, restore_backslashes(&text_of(s, match_start, text_end)));
            return Found {
                before: match_start,
                nodes: vec![node.into()],
                after: text_end,
                messages: Vec::new(),
                end_string: text_of(s, match_end + end.group.0, text_end),
            };
        }
        let mut msg = self.message(
            WARNING,
            &format!("Inline {} start-string without end-string.", kind.tag()),
        );
        let text = restore_backslashes(&text_of(s, match_start, match_end));
        let prb = self.problematic(&text, &mut msg, self.span(base, match_start, match_end));
        Found {
            before: match_start,
            nodes: vec![prb.into()],
            after: match_end,
            messages: vec![msg],
            end_string: String::new(),
        }
    }

    fn inline_internal_target(&mut self, s: &[char], base: usize, m: Start) -> Found {
        let mut found = self.inline_obj(s, base, m, Kind::Target, false);
        if let Some(Node::Element(target)) = found.nodes.first_mut()
            && target.kind == Kind::Target
        {
            let name = normalize_name(&astext(target));
            target.attributes.push("names", name);
            self.doc
                .note_explicit_target(target, Some(MsgNode::Parent), None);
        }
        found
    }

    fn substitution_reference(&mut self, s: &[char], base: usize, m: Start) -> Found {
        let mut found = self.inline_obj(s, base, m, Kind::SubstitutionReference, false);
        if found.nodes.len() == 1
            && let Some(Node::Element(subref)) = found.nodes.first_mut()
            && subref.kind == Kind::SubstitutionReference
        {
            let subref_text = astext(subref);
            subref
                .attributes
                .set("refname", whitespace_normalize_name(&subref_text));
            if found.end_string.ends_with('_') {
                let mut reference = Element::new(Kind::Reference);
                reference.span = subref.span;
                reference
                    .attributes
                    .set(RAW, format!("|{subref_text}{}", found.end_string));
                if found.end_string.ends_with("__") {
                    reference.attributes.set("anonymous", true);
                } else {
                    reference
                        .attributes
                        .set("refname", normalize_name(&subref_text));
                    self.doc.note_refname(&mut reference);
                }
                reference.children.push(found.nodes.remove(0));
                found.nodes = vec![reference.into()];
            }
        }
        found
    }

    fn interpreted_or_phrase_ref(&mut self, s: &[char], base: usize, m: Start) -> Found {
        let (match_start, match_end) = m.group;
        let role_start = m.start;
        let mut role = m.role.map(|(a, b)| text_of(s, a + 1, b - 1));
        let mut position = "";
        if role.is_some() {
            position = "prefix";
        } else if Self::quoted_start(s, &m) {
            return Found {
                before: match_end,
                nodes: Vec::new(),
                after: match_end,
                messages: Vec::new(),
                end_string: String::new(),
            };
        }
        let problem = |this: &mut Self, text: &str, end: usize| -> Found {
            let mut msg = this.message(WARNING, text);
            let raw = restore_backslashes(&text_of(s, role_start, end));
            let prb = this.problematic(&raw, &mut msg, this.span(base, role_start, end));
            Found {
                before: role_start,
                nodes: vec![prb.into()],
                after: end,
                messages: vec![msg],
                end_string: String::new(),
            }
        };
        if let Some(end) = find_end(K::Interpreted, &s[match_end..])
            && end.group.0 > 0
        {
            let text_end = match_end + end.group.1;
            if let Some((a, b)) = end.role {
                if role.is_some() {
                    return problem(
                        self,
                        "Multiple roles in interpreted text (both prefix and suffix present; \
                         only one allowed).",
                        text_end,
                    );
                }
                role = Some(text_of(&s[match_end..], a + 1, b - 1));
                position = "suffix";
            }
            let escaped = text_of(s, match_end, match_end + end.group.0);
            let rawsource = restore_backslashes(&text_of(s, match_start, text_end));
            if rawsource.ends_with('_') {
                if role.is_some() {
                    return problem(
                        self,
                        &format!(
                            "Mismatch: both interpreted text role {position} and reference suffix."
                        ),
                        text_end,
                    );
                }
                let nodes =
                    self.phrase_ref(&rawsource, &escaped, self.span(base, match_start, text_end));
                return Found {
                    before: match_start,
                    nodes,
                    after: text_end,
                    messages: Vec::new(),
                    end_string: String::new(),
                };
            }
            let rawsource = restore_backslashes(&text_of(s, role_start, text_end));
            let span = self.span(base, role_start, text_end);
            let (nodes, messages) = roles::interpreted(
                self,
                &rawsource,
                &escaped,
                role.as_deref().unwrap_or(""),
                span,
            );
            return Found {
                before: role_start,
                nodes,
                after: text_end,
                messages,
                end_string: String::new(),
            };
        }
        let mut msg = self.message(
            WARNING,
            "Inline interpreted text or phrase reference start-string without end-string.",
        );
        let text = restore_backslashes(&text_of(s, match_start, match_end));
        let prb = self.problematic(&text, &mut msg, self.span(base, match_start, match_end));
        Found {
            before: match_start,
            nodes: vec![prb.into()],
            after: match_end,
            messages: vec![msg],
            end_string: String::new(),
        }
    }

    /// A phrase reference, possibly with an embedded URI or alias.
    fn phrase_ref(&mut self, rawsource: &str, escaped: &str, span: Option<Span>) -> Vec<Node> {
        let chars: Vec<char> = escaped.chars().collect();
        let mut text: String;
        let unescaped: String;
        let mut target = None;
        let mut alias = String::new();
        let mut alias_is_name = false;
        if let Some((start, a, b)) = embedded_link(&chars) {
            text = text_of(&chars, 0, start);
            let aliastext = text_of(&chars, a, b);
            let rawaliastext = restore_backslashes(&aliastext);
            let underscore_escaped = rawaliastext.ends_with("\\_");
            let alias_chars: Vec<char> = aliastext.chars().collect();
            if aliastext.ends_with('_') && !(underscore_escaped || uri_matches(&alias_chars)) {
                alias_is_name = true;
                alias = normalize_name(&unescape(&aliastext[..aliastext.len() - 1]));
                let mut t = Element::new(Kind::Target);
                t.span = span;
                t.attributes.set("refname", alias.clone());
                target = Some(t);
            } else {
                let parts = split_escaped_whitespace(&aliastext);
                let joined: Vec<String> = parts
                    .iter()
                    .map(|p| split_whitespace(p).collect::<String>())
                    .collect();
                alias = unescape(&joined.join(" "));
                if is_email(&alias) {
                    alias = format!("mailto:{alias}");
                }
                if let Some(stripped) = alias.strip_suffix("\\_") {
                    alias = format!("{stripped}_");
                }
                let mut t = Element::new(Kind::Target);
                t.span = span;
                t.attributes.set("refuri", alias.clone());
                target = Some(t);
            }
            if text.is_empty() {
                text = alias.clone();
            }
            unescaped = unescape(&text);
        } else {
            text = escaped.to_owned();
            unescaped = unescape(&text);
        }
        let refname = normalize_name(&unescaped);
        let mut reference = element(Kind::Reference, &text, span);
        reference
            .attributes
            .set("name", whitespace_normalize_name(&unescaped));
        reference.attributes.set(RAW, rawsource);
        let mut nodes = Vec::new();
        if rawsource.ends_with("__") {
            match &target {
                Some(_) if alias_is_name => {
                    reference.attributes.set("refname", alias);
                    self.doc.note_refname(&mut reference);
                }
                Some(_) => reference.attributes.set("refuri", alias),
                None => reference.attributes.set("anonymous", true),
            }
            nodes.push(reference.into());
        } else if let Some(mut t) = target {
            t.attributes.push("names", refname);
            if alias_is_name {
                reference.attributes.set("refname", alias);
                self.doc.note_indirect_target(&mut t);
                self.doc.note_refname(&mut reference);
            } else {
                reference.attributes.set("refuri", alias);
            }
            self.doc
                .note_implicit_target(&mut t, Some(MsgNode::Parent), None);
            if alias_is_name {
                let key = self.doc.key(&mut t);
                self.doc.notes.snapshots.insert(key, t.clone());
            }
            nodes.push(reference.into());
            nodes.push(t.into());
        } else {
            reference.attributes.set("refname", refname);
            self.doc.note_refname(&mut reference);
            nodes.push(reference.into());
        }
        nodes
    }

    fn footnote_reference(
        &mut self,
        s: &[char],
        base: usize,
        m: Start,
        (q, e, citation): (usize, usize, bool),
    ) -> Found {
        let label = text_of(s, q, e);
        let mut refname = normalize_name(&label);
        let span = self.span(base, m.group.0, m.group.1);
        let raw = format!("[{label}]_");
        let node = if citation {
            let mut r = element(Kind::CitationReference, &label, span);
            r.attributes.set(RAW, raw);
            r.attributes.set("refname", refname);
            self.doc.note_footnote_kind(&mut r, NoteList::CitationRef);
            r
        } else {
            let mut r = Element::new(Kind::FootnoteReference);
            r.span = span;
            r.attributes.set(RAW, raw);
            if let Some(rest) = refname.strip_prefix('#') {
                refname = rest.to_owned();
                r.attributes.set("auto", 1i64);
                self.doc
                    .note_footnote_kind(&mut r, NoteList::AutoFootnoteRef);
            } else if refname == "*" {
                refname.clear();
                r.attributes.set("auto", "*");
                self.doc
                    .note_footnote_kind(&mut r, NoteList::SymbolFootnoteRef);
            } else {
                r.children.push(Text::new(label, span).into());
            }
            if !refname.is_empty() {
                r.attributes.set("refname", refname);
                self.doc.note_footnote_kind(&mut r, NoteList::FootnoteRef);
            }
            r
        };
        let mut before = m.group.0;
        if !citation && self.doc.extensions.settings.trim_footnote_reference_space {
            while before > 0 && is_python_whitespace(s[before - 1]) {
                before -= 1;
            }
        }
        Found {
            before,
            nodes: vec![node.into()],
            after: m.group.1,
            messages: Vec::new(),
            end_string: String::new(),
        }
    }

    fn reference(&mut self, s: &[char], base: usize, m: Start, anonymous: bool) -> Found {
        let refend = if anonymous { 2 } else { 1 };
        let name = text_of(s, m.group.0, m.group.1 - refend);
        let mut r = element(
            Kind::Reference,
            &name,
            self.span(base, m.group.0, m.group.1),
        );
        r.attributes.set("name", whitespace_normalize_name(&name));
        r.attributes.set(RAW, text_of(s, m.group.0, m.group.1));
        if anonymous {
            r.attributes.set("anonymous", true);
        } else {
            r.attributes.set("refname", normalize_name(&name));
            self.doc.note_refname(&mut r);
        }
        Found {
            before: m.group.0,
            nodes: vec![r.into()],
            after: m.group.1,
            messages: Vec::new(),
            end_string: String::new(),
        }
    }

    /// Standalone URIs and email addresses in plain text (docutils'
    /// `implicit_inline`). `base` is where `s` starts in the text.
    fn implicit_inline(&mut self, s: &[char], base: usize) -> Vec<Node> {
        if s.is_empty() {
            return Vec::new();
        }
        if let Some((a, b, scheme)) = find_uri(s) {
            let known = match scheme {
                Some(scheme_end) => {
                    let scheme = text_of(s, a, scheme_end).to_lowercase();
                    tables::URI_SCHEMES.binary_search(&scheme.as_str()).is_ok()
                }
                None => true,
            };
            if known {
                let text = text_of(s, a, b);
                let mut refuri = unescape(&text);
                if scheme.is_none() {
                    refuri = format!("mailto:{refuri}");
                }
                return self.implicit_reference(s, base, (a, b), refuri);
            }
        }
        let settings = &self.doc.extensions.settings;
        if settings.pep_references
            && let Some((a, b, number)) = find_pep(s)
        {
            let refuri = settings.pep_url(number);
            return self.implicit_reference(s, base, (a, b), refuri);
        }
        if settings.rfc_references
            && let Some((a, b, number)) = find_rfc(s)
        {
            let refuri = settings.rfc_url(number);
            return self.implicit_reference(s, base, (a, b), refuri);
        }
        let span = self.span(base, 0, s.len());
        vec![Text::new(s.iter().collect::<String>(), span).into()]
    }

    /// A reference to `refuri` for `s[a..b]`, with the text around it
    /// searched again.
    fn implicit_reference(
        &mut self,
        s: &[char],
        base: usize,
        (a, b): (usize, usize),
        refuri: String,
    ) -> Vec<Node> {
        let mut r = element(Kind::Reference, &text_of(s, a, b), self.span(base, a, b));
        r.attributes.set("refuri", refuri);
        let mut out = self.implicit_inline(&s[..a], base);
        out.push(r.into());
        out.extend(self.implicit_inline(&s[b..], base + b));
        out
    }
}

/// Joins pending unprocessed text pieces: where the first starts, and the
/// characters.
fn join_unprocessed(pieces: &mut Vec<(usize, Vec<char>)>) -> (usize, Vec<char>) {
    let start = pieces.first().map_or(0, |p| p.0);
    let chars = pieces.drain(..).flat_map(|p| p.1).collect();
    (start, chars)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use crate::nodes::{Kind, Node, Span};

    fn find(e: &crate::Element, kind: &Kind) -> Option<crate::Element> {
        for child in &e.children {
            if let Node::Element(c) = child {
                if &c.kind == kind {
                    return Some(c.clone());
                }
                if let Some(found) = find(c, kind) {
                    return Some(found);
                }
            }
        }
        None
    }

    #[test]
    fn spans_point_into_the_source() {
        let text = "Intro.\n\n  - Hi *em* and\n    ``lit`` here.\n";
        let doc = crate::parse(text);
        let em = find(&doc, &Kind::Emphasis).unwrap();
        assert_eq!(&text[em.span.unwrap().start..em.span.unwrap().end], "*em*");
        let lit = find(&doc, &Kind::Literal).unwrap();
        let start = text.find("``lit``").unwrap();
        assert_eq!(lit.span, Some(Span::new(start, start + 7)));
    }

    #[test]
    fn escapes_are_removed() {
        let doc = crate::parse(r"a \*b\* c\\d");
        assert_eq!(doc.astext(), r"a *b* c\d");
    }
}
