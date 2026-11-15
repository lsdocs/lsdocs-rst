//! Document-wide parse state: the elements being built, the names and ids
//! registered so far (docutils' `document.note_*` bookkeeping), section
//! title styles, and the reporter that makes `system_message` elements.

use std::collections::HashMap;

use crate::nodes::{AttrValue, Element, Kind, Node, Span, Text};
use crate::util::{make_id, whitespace_normalize_name};

/// Identifies an element that has registered names or ids, so a later
/// duplicate can update it wherever it is in the tree. Stored in a hidden
/// attribute until the parse ends.
type Key = i64;

/// The hidden attribute holding an element's [`Key`].
const KEY: &str = "\0key";

/// A hidden attribute: a line block line's indentation.
pub(crate) const LINE_INDENT: &str = "\0indent";

/// A section title style: the underline character, or the overline and
/// underline characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Style {
    Under(char),
    Over(char, char),
}

impl Style {
    pub(crate) fn display(self) -> String {
        match self {
            Style::Under(c) => c.to_string(),
            Style::Over(o, u) => format!("{o}/{u}"),
        }
    }
}

/// How a message's line number is found.
#[derive(Debug, Clone, Copy)]
pub(crate) enum At {
    /// An absolute line number (or the current line, if `None`) looked up
    /// through the top-level state machine, as docutils' reporter does.
    Top(Option<usize>),
    /// Exactly this line (or no line).
    Exact(Option<usize>),
}

/// Where a duplicate-name message goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MsgNode {
    /// Into the element being registered.
    Itself,
    /// Into the current parent element.
    Parent,
}

/// An element under construction. `at` is the index in the parent's
/// children it will be inserted at, or `None` for an element that is not
/// attached to the tree yet (it is handed back on close).
struct Frame {
    element: Element,
    at: Option<usize>,
}

/// Message levels, as docutils numbers them.
pub(crate) const INFO: i64 = 1;
pub(crate) const WARNING: i64 = 2;
pub(crate) const ERROR: i64 = 3;

fn level_name(level: i64) -> &'static str {
    match level {
        0 => "DEBUG",
        1 => "INFO",
        2 => "WARNING",
        3 => "ERROR",
        _ => "SEVERE",
    }
}

/// The document being parsed.
pub(crate) struct Document {
    stack: Vec<Frame>,
    source: Option<String>,
    /// The number of input lines (the top-level state machine's).
    pub(crate) top_len: usize,
    /// The top-level state machine's current line offset.
    pub(crate) top_offset: isize,
    /// The line of the state machine that moved last (docutils'
    /// `document.current_line`).
    pub(crate) current_line: Option<usize>,
    pub(crate) title_styles: Vec<Style>,
    names: HashMap<String, Option<Key>>,
    nametypes: HashMap<String, bool>,
    ids: HashMap<String, Key>,
    id_counter: HashMap<String, usize>,
    substitution_defs: HashMap<String, Key>,
    next_key: Key,
}

impl Document {
    pub(crate) fn new(root: Element, source: Option<String>, top_len: usize) -> Self {
        Self {
            stack: vec![Frame {
                element: root,
                at: None,
            }],
            source,
            top_len,
            top_offset: -1,
            current_line: None,
            title_styles: Vec::new(),
            names: HashMap::new(),
            nametypes: HashMap::new(),
            ids: HashMap::new(),
            id_counter: HashMap::new(),
            substitution_defs: HashMap::new(),
            next_key: 0,
        }
    }

    /// Closes every open element and returns the finished root.
    pub(crate) fn finish(mut self) -> Element {
        while self.stack.len() > 1 {
            self.close();
        }
        let mut root = self.stack.pop().unwrap().element;
        strip_hidden(&mut root);
        root
    }

    // -- building the tree ------------------------------------------------

    /// The current parent: the innermost open element.
    pub(crate) fn top(&mut self) -> &mut Element {
        &mut self.stack.last_mut().unwrap().element
    }

    /// Appends to the current parent.
    pub(crate) fn append(&mut self, node: impl Into<Node>) {
        self.top().children.push(node.into());
    }

    /// Appends each of `nodes` to the current parent.
    pub(crate) fn extend<N: Into<Node>>(&mut self, nodes: impl IntoIterator<Item = N>) {
        for node in nodes {
            self.append(node);
        }
    }

    /// Appends to the parent of the current parent.
    pub(crate) fn append_to_grandparent(&mut self, node: impl Into<Node>) {
        let n = self.stack.len();
        if n >= 2 {
            self.stack[n - 2].element.children.push(node.into());
        } else {
            self.append(node);
        }
    }

    /// Opens `element` as the new current parent. An attached element goes
    /// into its parent, at the current end, when closed; a detached one is
    /// handed back by [`Document::close`].
    pub(crate) fn open(&mut self, element: Element, attached: bool) {
        let at = attached.then(|| self.top().children.len());
        self.stack.push(Frame { element, at });
    }

    /// Closes the current parent: an attached element is inserted into its
    /// parent, a detached one returned. Its span grows to cover its
    /// children.
    pub(crate) fn close(&mut self) -> Option<Element> {
        let Frame { mut element, at } = self.stack.pop().expect("an open element");
        cover_children(&mut element);
        match at {
            Some(i) => {
                let parent = self.top();
                let i = i.min(parent.children.len());
                parent.children.insert(i, element.into());
                None
            }
            None => Some(element),
        }
    }

    /// The number of sections among the current parent and its attached
    /// ancestors (docutils' `len(node.section_hierarchy())`).
    pub(crate) fn section_depth(&self) -> usize {
        let mut depth = 0;
        for frame in self.stack.iter().rev() {
            if frame.element.kind == Kind::Section {
                depth += 1;
            }
            if frame.at.is_none() {
                break;
            }
        }
        depth
    }

    /// Closes sections until the current parent is at section depth
    /// `depth`.
    pub(crate) fn close_sections_to(&mut self, depth: usize) {
        while self.section_depth() > depth && self.top().kind == Kind::Section {
            self.close();
        }
    }

    // -- messages ---------------------------------------------------------

    /// A line number as the top-level state machine reports it: `offset`
    /// is looked up in its lines, with Python's indexing rules.
    fn top_line(&self, offset: isize) -> Option<usize> {
        let len = self.top_len as isize;
        if (0..len).contains(&offset) || (offset == len && len > 0) {
            Some((offset + 1) as usize)
        } else if (-len..0).contains(&offset) {
            Some((len + offset + 1) as usize)
        } else {
            None
        }
    }

    /// A `system_message` element.
    pub(crate) fn message(&self, level: i64, text: &str, children: Vec<Node>, at: At) -> Element {
        let line = match at {
            // A line the lookup cannot find is kept as given.
            At::Top(Some(line)) => self.top_line(line as isize - 1).or(Some(line)),
            At::Top(None) => self.top_line(self.top_offset),
            At::Exact(line) => line,
        };
        let mut msg = Element::new(Kind::SystemMessage)
            .with_attr("level", level)
            .with_attr("type", level_name(level));
        if let Some(line) = line {
            msg.attributes.set("line", line as i64);
        }
        if let Some(source) = &self.source {
            msg.attributes.set("source", source.as_str());
        }
        msg.children.push(paragraph(text).into());
        msg.children.extend(children);
        msg
    }

    // -- names and ids ----------------------------------------------------

    /// `node`'s key, assigning one if needed.
    fn key(&mut self, node: &mut Element) -> Key {
        if let Some(AttrValue::Int(k)) = node.attributes.get(KEY) {
            return *k;
        }
        self.next_key += 1;
        node.attributes.set(KEY, self.next_key);
        self.next_key
    }

    /// The element with `key`, among the open elements and their children.
    fn find(&mut self, key: Key) -> Option<&mut Element> {
        self.stack
            .iter_mut()
            .rev()
            .find_map(|f| find_in(&mut f.element, key))
    }

    /// Delivers a message to `msgnode`.
    fn deliver(&mut self, msg: Element, node: &mut Element, msgnode: Option<MsgNode>) {
        match msgnode {
            Some(MsgNode::Itself) => node.children.push(msg.into()),
            Some(MsgNode::Parent) => self.append(msg),
            None => {}
        }
    }

    /// Checks `node`'s ids, making one if it has none; returns the last.
    /// (docutils' `document.set_id`).
    pub(crate) fn set_id(
        &mut self,
        node: &mut Element,
        msgnode: Option<MsgNode>,
        line: Option<usize>,
    ) -> String {
        let key = self.key(node);
        if node.attributes.get_list("ids").is_empty() {
            let id = self.create_id(node);
            node.attributes.push("ids", id);
        }
        let ids = node.attributes.get_list("ids").to_vec();
        for id in &ids {
            let owner = *self.ids.entry(id.clone()).or_insert(key);
            if owner != key {
                let old_tag = self.find(owner).map(|e| e.starttag()).unwrap_or_default();
                let text = format!(
                    "Duplicate ID: \"{id}\" used by {old_tag} and {}",
                    starttag_visible(node)
                );
                let msg = self.message(ERROR, &text, Vec::new(), At::Exact(line));
                self.deliver(msg, node, msgnode);
            }
        }
        ids.last().cloned().unwrap_or_default()
    }

    /// A new id for `node` (docutils' `document.create_id`, with the
    /// default `id_prefix` and `auto_id_prefix` settings).
    fn create_id(&mut self, node: &Element) -> String {
        let mut base_id = String::new();
        for name in node.attributes.get_list("names") {
            base_id = make_id(name);
            if !base_id.is_empty() && !self.ids.contains_key(&base_id) {
                return base_id;
            }
        }
        let dupnames = node.attributes.get_list("dupnames");
        let prefix = if !base_id.is_empty() {
            format!("{base_id}-")
        } else if let Some(first) = dupnames.first()
            && !make_id(first).is_empty()
        {
            format!("{}-", make_id(first))
        } else {
            format!("{}-", make_id(node.kind.tag()))
        };
        loop {
            let n = self.id_counter.entry(prefix.clone()).or_insert(0);
            *n += 1;
            let id = format!("{prefix}{n}");
            if !self.ids.contains_key(&id) {
                return id;
            }
        }
    }

    /// Registers `node`'s names, handling conflicts (docutils'
    /// `document.note_names`).
    fn note_names(
        &mut self,
        node: &mut Element,
        msgnode: Option<MsgNode>,
        explicit: bool,
        line: Option<usize>,
    ) {
        let key = self.key(node);
        for name in node.attributes.get_list("names").to_vec() {
            match self.names.get(&name) {
                Some(&owner) if owner != Some(key) => {
                    self.set_duplicate_name(node, &name, owner, msgnode, explicit, line);
                }
                _ => {
                    self.names.insert(name.clone(), Some(key));
                    self.nametypes.entry(name).or_insert(explicit);
                }
            }
        }
    }

    /// Resolves a name conflict as the reStructuredText specification says
    /// (docutils' `document.set_duplicate_name`).
    fn set_duplicate_name(
        &mut self,
        node: &mut Element,
        name: &str,
        old: Option<Key>,
        msgnode: Option<MsgNode>,
        explicit: bool,
        line: Option<usize>,
    ) {
        let old_explicit = self.nametypes.get(name).copied().unwrap_or(false);
        self.nametypes
            .insert(name.to_owned(), old_explicit || explicit);
        let same_ref = old.and_then(|k| {
            let old = self.find(k)?;
            let same = |attr: &str| {
                node.attributes.get_str(attr).is_some()
                    && node.attributes.get_str(attr) == old.attributes.get_str(attr)
            };
            Some(same("refname") || same("refuri"))
        });
        let mut report = None;
        if same_ref == Some(true) {
            let reference = node
                .attributes
                .get_str("refuri")
                .filter(|s| !s.is_empty())
                .or(node.attributes.get_str("refname"))
                .unwrap_or_default();
            report = Some((
                INFO,
                format!("Duplicate name \"{name}\" for external target \"{reference}\"."),
            ));
            dupname(node, name);
        } else if explicit {
            if old_explicit {
                report = Some((
                    WARNING,
                    format!("Duplicate explicit target name: \"{name}\"."),
                ));
                dupname(node, name);
                if let Some(k) = old {
                    if let Some(old) = self.find(k) {
                        dupname(old, name);
                    }
                    self.names.insert(name.to_owned(), None);
                }
            } else {
                let key = self.key(node);
                self.names.insert(name.to_owned(), Some(key));
                if let Some(k) = old {
                    report = Some((
                        INFO,
                        format!("Target name overrides implicit target name \"{name}\"."),
                    ));
                    if let Some(old) = self.find(k) {
                        dupname(old, name);
                    }
                }
            }
        } else {
            report = Some((INFO, format!("Duplicate implicit target name: \"{name}\".")));
            dupname(node, name);
            if let Some(k) = old
                && !old_explicit
            {
                if let Some(old) = self.find(k) {
                    dupname(old, name);
                }
                self.names.insert(name.to_owned(), None);
            }
        }
        if let Some((level, text)) = report {
            let backrefs = if node.kind == Kind::Target && node.children.is_empty() {
                Vec::new()
            } else {
                vec![self.set_id(node, None, line)]
            };
            let mut msg = self.message(level, &text, Vec::new(), At::Exact(line));
            msg.attributes.set("backrefs", backrefs);
            self.deliver(msg, node, msgnode);
        }
    }

    /// Registers an implicit target, such as a section title.
    pub(crate) fn note_implicit_target(
        &mut self,
        node: &mut Element,
        msgnode: Option<MsgNode>,
        line: Option<usize>,
    ) {
        self.note_names(node, msgnode, false, line);
        self.set_id(node, msgnode, line);
    }

    /// Registers an explicit target: hyperlink targets, footnotes,
    /// citations.
    pub(crate) fn note_explicit_target(
        &mut self,
        node: &mut Element,
        msgnode: Option<MsgNode>,
        line: Option<usize>,
    ) {
        self.note_names(node, msgnode, true, line);
        self.set_id(node, msgnode, line);
    }

    /// Registers a substitution definition; a later one with the same name
    /// wins.
    pub(crate) fn note_substitution_def(
        &mut self,
        node: &mut Element,
        def_name: &str,
        line: Option<usize>,
    ) {
        let name = whitespace_normalize_name(def_name);
        if let Some(&old) = self.substitution_defs.get(&name) {
            let text = format!("Duplicate substitution definition name: \"{name}\".");
            let msg = self.message(ERROR, &text, Vec::new(), At::Exact(line));
            self.append(msg);
            if let Some(old) = self.find(old) {
                dupname(old, &name);
            }
        }
        let key = self.key(node);
        self.substitution_defs.insert(name, key);
    }
}

/// Moves `name` from `node`'s names to its dupnames.
fn dupname(node: &mut Element, name: &str) {
    node.attributes.push("dupnames", name);
    let mut names = node.attributes.get_list("names").to_vec();
    if let Some(i) = names.iter().position(|n| n == name) {
        names.remove(i);
    }
    node.attributes.set("names", names);
}

/// `node`'s start tag without hidden attributes.
fn starttag_visible(node: &Element) -> String {
    let mut copy = Element::new(node.kind.clone());
    copy.attributes = node.attributes.clone();
    copy.attributes.remove(KEY);
    copy.attributes.remove(LINE_INDENT);
    copy.starttag()
}

fn find_in(element: &mut Element, key: Key) -> Option<&mut Element> {
    if element.attributes.get(KEY) == Some(&AttrValue::Int(key)) {
        return Some(element);
    }
    element.children.iter_mut().find_map(|c| match c {
        Node::Element(e) => find_in(e, key),
        Node::Text(_) => None,
    })
}

fn strip_hidden(element: &mut Element) {
    element.attributes.remove(KEY);
    element.attributes.remove(LINE_INDENT);
    for child in &mut element.children {
        if let Node::Element(e) = child {
            strip_hidden(e);
        }
    }
}

/// Grows `element`'s span to cover its children's.
pub(crate) fn cover_children(element: &mut Element) {
    let mut span = element.span;
    for child in &element.children {
        if let Some(s) = child.span() {
            span = Some(span.map_or(s, |x: Span| x.to(s)));
        }
    }
    element.span = span;
}

/// A paragraph holding `text`, unparsed.
pub(crate) fn paragraph(text: &str) -> Element {
    Element::new(Kind::Paragraph).with_child(Text::new(text, None))
}

/// An element that keeps its text as is (docutils' `FixedTextElement`),
/// such as a literal block or a comment.
pub(crate) fn fixed(kind: Kind, text: &str, span: Option<Span>) -> Element {
    let mut e = Element::new(kind).with_attr("xml:space", "preserve");
    e.span = span;
    if !text.is_empty() {
        e.children.push(Text::new(text, span).into());
    }
    e
}
