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
    /// Like `Top`, for a line number that may be out of range (docutils'
    /// arithmetic can go below 1).
    TopSigned(isize),
    /// Exactly this line (or no line).
    Exact(Option<usize>),
}

impl At {
    /// The line of a message about a node with line `line`: the node's
    /// line, or the current one if it has none (docutils'
    /// `base_node`).
    pub(crate) fn node(line: Option<usize>) -> At {
        match line {
            Some(line) => At::Exact(Some(line)),
            None => At::Top(None),
        }
    }
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
    /// Inline nodes parsed but not yet in the tree (they can be
    /// duplicates' earlier targets).
    pub(crate) scratch: Vec<Node>,
    /// Roles by lower-case name: custom ones, those looked up so far, and
    /// the default role under the empty name.
    pub(crate) roles: HashMap<String, super::roles::Role>,
    /// The `refname` and `refuri` of each element with registered names.
    refs: HashMap<Key, (Option<String>, Option<String>)>,
    /// Duplicate names for elements that were not in reach when found
    /// (held by a caller, not yet in the tree); applied at the end.
    deferred_dupnames: Vec<(Key, String)>,
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
            scratch: Vec::new(),
            roles: HashMap::new(),
            refs: HashMap::new(),
            deferred_dupnames: Vec::new(),
        }
    }

    /// Closes every open element and returns the finished root.
    pub(crate) fn finish(mut self) -> Element {
        while self.stack.len() > 1 {
            self.close();
        }
        let mut root = self.stack.pop().unwrap().element;
        for (key, name) in std::mem::take(&mut self.deferred_dupnames) {
            if let Some(node) = find_in(&mut root, key) {
                dupname(node, &name);
            }
        }
        strip_hidden(&mut root);
        root
    }

    // -- building the tree ------------------------------------------------

    /// The current parent: the innermost open element.
    pub(crate) fn top(&mut self) -> &mut Element {
        &mut self.stack.last_mut().unwrap().element
    }

    /// The document element.
    pub(crate) fn root(&mut self) -> &mut Element {
        &mut self.stack[0].element
    }

    /// Inserts `nodes` into the document element at `index`, keeping open
    /// elements' places right.
    pub(crate) fn insert_in_root(&mut self, index: usize, nodes: Vec<Node>) {
        let n = nodes.len();
        let index = index.min(self.stack[0].element.children.len());
        self.stack[0].element.children.splice(index..index, nodes);
        if let Some(frame) = self.stack.get_mut(1)
            && let Some(at) = frame.at.as_mut()
            && *at >= index
        {
            *at += n;
        }
    }

    /// Adds `content` to the document's header or footer (`kind`), making
    /// the decoration element if needed (docutils' `get_decoration`).
    pub(crate) fn add_to_decoration(&mut self, kind: Kind, content: Vec<Node>) {
        let position = |root: &Element| {
            root.children
                .iter()
                .position(|c| matches!(c, Node::Element(e) if e.kind == Kind::Decoration))
        };
        if position(&self.stack[0].element).is_none() {
            let index = self.stack[0]
                .element
                .children
                .iter()
                .position(|c| !matches!(c, Node::Element(e) if matches!(e.kind, Kind::Title | Kind::Subtitle | Kind::Rubric | Kind::Meta)))
                .unwrap_or(self.stack[0].element.children.len());
            self.insert_in_root(index, vec![Element::new(Kind::Decoration).into()]);
        }
        let i = position(&self.stack[0].element).unwrap();
        let Node::Element(decoration) = &mut self.stack[0].element.children[i] else {
            return;
        };
        let part = if kind == Kind::Header {
            if !matches!(decoration.children.first(), Some(Node::Element(e)) if e.kind == Kind::Header)
            {
                decoration
                    .children
                    .insert(0, Element::new(Kind::Header).into());
            }
            decoration.children.first_mut()
        } else {
            if !matches!(decoration.children.last(), Some(Node::Element(e)) if e.kind == Kind::Footer)
            {
                decoration.children.push(Element::new(Kind::Footer).into());
            }
            decoration.children.last_mut()
        };
        if let Some(Node::Element(part)) = part {
            part.children.extend(content);
        }
    }

    /// Whether a target name is registered.
    pub(crate) fn has_name(&self, name: &str) -> bool {
        self.names.contains_key(name)
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

    /// The number of children of the current parent, a position for
    /// [`Document::open_at`].
    pub(crate) fn position(&mut self) -> usize {
        self.top().children.len()
    }

    /// Opens `element` to be inserted at position `at` of the current
    /// parent when closed.
    pub(crate) fn open_at(&mut self, element: Element, at: usize) {
        self.stack.push(Frame {
            element,
            at: Some(at),
        });
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
            At::Top(Some(line)) => self
                .top_line(line as isize - 1)
                .map(|l| l as i64)
                .or(Some(line as i64)),
            At::TopSigned(line) => self
                .top_line(line - 1)
                .map(|l| l as i64)
                .or(Some(line as i64)),
            At::Top(None) => self.top_line(self.top_offset).map(|l| l as i64),
            At::Exact(line) => line.map(|l| l as i64),
        };
        let mut msg = Element::new(Kind::SystemMessage)
            .with_attr("level", level)
            .with_attr("type", level_name(level));
        if let Some(line) = line {
            msg.attributes.set("line", line);
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
        let in_scratch = self.scratch.iter_mut().find_map(|n| match n {
            Node::Element(e) => find_in(e, key),
            Node::Text(_) => None,
        });
        if in_scratch.is_some() {
            return in_scratch;
        }
        self.stack
            .iter_mut()
            .rev()
            .find_map(|f| find_in(&mut f.element, key))
    }

    /// Makes `name` a dupname of the element with `key`, now if it is in
    /// reach, else when the parse ends.
    fn dupname_key(&mut self, key: Key, name: &str) {
        match self.find(key) {
            Some(node) => dupname(node, name),
            None => self.deferred_dupnames.push((key, name.to_owned())),
        }
    }

    /// Delivers a message to `msgnode`.
    /// docutils only adds it to an element whose content model allows
    /// body elements; elsewhere the message is dropped.
    fn deliver(&mut self, msg: Element, node: &mut Element, msgnode: Option<MsgNode>) {
        match msgnode {
            Some(MsgNode::Itself) if accepts_body(&node.kind) => node.children.push(msg.into()),
            Some(MsgNode::Parent) if accepts_body(&self.top().kind) => self.append(msg),
            _ => {}
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
                let msg = self.message(ERROR, &text, Vec::new(), At::node(line));
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
        let reference = |attr: &str| node.attributes.get_str(attr).map(str::to_owned);
        self.refs
            .insert(key, (reference("refname"), reference("refuri")));
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
            let (refname, refuri) = self.refs.get(&k)?;
            let same = |attr: &str, old: &Option<String>| {
                node.attributes.get_str(attr).is_some()
                    && node.attributes.get_str(attr) == old.as_deref()
            };
            Some(same("refname", refname) || same("refuri", refuri))
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
                    self.dupname_key(k, name);
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
                    self.dupname_key(k, name);
                }
            }
        } else {
            report = Some((INFO, format!("Duplicate implicit target name: \"{name}\".")));
            dupname(node, name);
            if let Some(k) = old
                && !old_explicit
            {
                self.dupname_key(k, name);
                self.names.insert(name.to_owned(), None);
            }
        }
        if let Some((level, text)) = report {
            let backrefs = if node.kind == Kind::Target && node.children.is_empty() {
                Vec::new()
            } else {
                vec![self.set_id(node, None, line)]
            };
            let mut msg = self.message(level, &text, Vec::new(), At::node(line));
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
            let msg = self.message(ERROR, &text, Vec::new(), At::node(line));
            self.append(msg);
            self.dupname_key(old, &name);
        }
        let key = self.key(node);
        self.substitution_defs.insert(name, key);
    }
}

/// Whether elements of `kind` may contain body elements (their docutils
/// content model mentions `Body`).
fn accepts_body(kind: &Kind) -> bool {
    matches!(
        kind,
        Kind::Admonition
            | Kind::Attention
            | Kind::BlockQuote
            | Kind::Caution
            | Kind::Citation
            | Kind::Compound
            | Kind::Container
            | Kind::Danger
            | Kind::Definition
            | Kind::Description
            | Kind::Document
            | Kind::Entry
            | Kind::Error
            | Kind::FieldBody
            | Kind::Footer
            | Kind::Footnote
            | Kind::Header
            | Kind::Hint
            | Kind::Important
            | Kind::Legend
            | Kind::ListItem
            | Kind::Note
            | Kind::Section
            | Kind::Sidebar
            | Kind::SystemMessage
            | Kind::Tip
            | Kind::Topic
            | Kind::Warning
    )
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

/// Removes hidden attributes, and escape nulls from text (docutils keeps
/// them in Text nodes and removes them when printing).
fn strip_hidden(element: &mut Element) {
    element.attributes.remove(KEY);
    element.attributes.remove(LINE_INDENT);
    for child in &mut element.children {
        match child {
            Node::Element(e) => strip_hidden(e),
            Node::Text(t) => {
                if t.value.contains('\0') {
                    t.value = crate::util::unescape(&t.value);
                }
            }
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
