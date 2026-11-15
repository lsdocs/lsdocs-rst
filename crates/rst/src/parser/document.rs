//! Document-wide parse state: the elements being built, the names and ids
//! registered so far (docutils' `document.note_*` bookkeeping), section
//! title styles, and the reporter that makes `system_message` elements.

use std::collections::HashMap;

use crate::nodes::{AttrValue, Element, Kind, Node, Span, Text};
use crate::util::{make_id, whitespace_normalize_name};

/// Identifies an element that has registered names or ids, so a later
/// duplicate can update it wherever it is in the tree. Stored in a hidden
/// attribute until the parse ends.
pub(crate) type Key = i64;

/// The hidden attribute holding an element's [`Key`].
pub(crate) const KEY: &str = "\0key";

/// A hidden attribute: an element's raw source (docutils' `rawsource`),
/// for the `problematic` element that replaces an unresolved reference.
pub(crate) const RAW: &str = "\0raw";

/// A hidden attribute: an element's line number (docutils' `line`).
pub(crate) const LINE: &str = "\0line";

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
#[derive(Debug, Clone)]
pub(crate) enum At {
    /// An absolute line number (or the current line, if `None`) looked up
    /// through the top-level state machine, as docutils' reporter does.
    Top(Option<usize>),
    /// Like `Top`, for a line number that may be out of range (docutils'
    /// arithmetic can go below 1).
    TopSigned(isize),
    /// Exactly this line (or no line).
    Exact(Option<usize>),
    /// This source (`None` for the document) and line.
    Source(Option<std::rc::Rc<str>>, Option<usize>),
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
pub(crate) const SEVERE: i64 = 4;

fn level_name(level: i64) -> &'static str {
    match level {
        0 => "DEBUG",
        1 => "INFO",
        2 => "WARNING",
        3 => "ERROR",
        _ => "SEVERE",
    }
}

/// An `include` directive's `start-line`, `end-line`, `start-after` and
/// `end-before` options.
pub(crate) type IncludeClip = (Option<i64>, Option<i64>, Option<String>, Option<String>);

/// The document being parsed.
pub(crate) struct Document {
    stack: Vec<Frame>,
    source: Option<String>,
    /// The top-level state machine's lines, where docutils' reporter
    /// looks up the source and line of a message.
    pub(crate) top_lines: Option<super::machine::Lines>,
    /// The top-level state machine's current line offset.
    pub(crate) top_offset: isize,
    /// The line of the state machine that moved last (docutils'
    /// `document.current_line`), and its source (`None` for the document
    /// itself).
    pub(crate) current_line: Option<usize>,
    pub(crate) current_source: Option<std::rc::Rc<str>>,
    /// The files being included, outermost first, with their clipping
    /// options (docutils' `include_log`).
    pub(crate) include_log: Vec<(String, IncludeClip)>,
    pub(crate) title_styles: Vec<Style>,
    pub(crate) names: HashMap<String, Option<Key>>,
    /// The id each name refers to (`None` for a duplicate name).
    pub(crate) nameids: HashMap<String, Option<String>>,
    /// What docutils notes during parsing for its transforms.
    pub(crate) notes: Notes,
    nametypes: HashMap<String, bool>,
    pub(crate) ids: HashMap<String, Key>,
    id_counter: HashMap<String, usize>,
    pub(crate) substitution_defs: HashMap<String, Key>,
    next_key: Key,
    /// Inline nodes parsed but not yet in the tree (they can be
    /// duplicates' earlier targets).
    pub(crate) scratch: Vec<Node>,
    /// Roles by lower-case name: custom ones, those looked up so far, and
    /// the default role under the empty name.
    pub(crate) roles: HashMap<String, super::roles::Role>,
    /// What the caller added.
    pub(crate) extensions: std::sync::Arc<super::Extensions>,
    /// The `refname` and `refuri` of each element with registered names.
    refs: HashMap<Key, (Option<String>, Option<String>)>,
    /// Duplicate names for elements that were not in reach when found
    /// (held by a caller, not yet in the tree); applied at the end.
    deferred_dupnames: Vec<(Key, String)>,
}

impl Document {
    /// The document's source name.
    pub(crate) fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    pub(crate) fn new(root: Element, source: Option<String>) -> Self {
        Self {
            stack: vec![Frame {
                element: root,
                at: None,
            }],
            source,
            top_lines: None,
            top_offset: -1,
            current_line: None,
            current_source: None,
            include_log: Vec::new(),
            title_styles: Vec::new(),
            names: HashMap::new(),
            nameids: HashMap::new(),
            notes: Notes::default(),
            nametypes: HashMap::new(),
            ids: HashMap::new(),
            id_counter: HashMap::new(),
            substitution_defs: HashMap::new(),
            next_key: 0,
            scratch: Vec::new(),
            roles: HashMap::new(),
            extensions: Default::default(),
            refs: HashMap::new(),
            deferred_dupnames: Vec::new(),
        }
    }

    /// Closes every open element, runs docutils' transforms if asked, and
    /// returns the finished root.
    pub(crate) fn finish(mut self, transforms: bool) -> Element {
        while self.stack.len() > 1 {
            self.close();
        }
        let mut root = self.stack.pop().unwrap().element;
        for (key, name) in std::mem::take(&mut self.deferred_dupnames) {
            if let Some(node) = find_in(&mut root, key) {
                dupname(node, &name);
            } else if let Some(copy) = self.notes.snapshots.get_mut(&key) {
                dupname(copy, &name);
            }
        }
        if transforms {
            super::transforms::apply(&mut self, &mut root);
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

    /// Whether the current parent is in the document tree, not in an
    /// element still detached (docutils sets the line of a node added to
    /// such a parent).
    pub(crate) fn in_document(&self) -> bool {
        self.stack.iter().skip(1).all(|f| f.at.is_some())
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

    /// The source (`None` for the document) and line of absolute line
    /// offset `offset`, as the top-level state machine finds them
    /// (docutils' `get_source_and_line`, with Python's indexing rules).
    fn source_and_line(&self, offset: isize) -> Option<(Option<std::rc::Rc<str>>, i64)> {
        let lines = super::machine::LineList::get(self.top_lines.as_ref()?);
        let len = lines.len() as isize;
        let (index, past_end) = if (0..len).contains(&offset) {
            (offset, 0)
        } else if (-len..0).contains(&offset) {
            (len + offset, 0)
        } else if offset == len && len > 0 {
            // "Just past the end": the last line's, plus one.
            (len - 1, 1)
        } else {
            return None;
        };
        let origin = &lines[index as usize].origin;
        Some((origin.source.clone(), origin.offset as i64 + 1 + past_end))
    }

    /// A `system_message` element.
    pub(crate) fn message(&self, level: i64, text: &str, children: Vec<Node>, at: At) -> Element {
        // A line the lookup cannot find is kept as given.
        let lookup = |line: isize| match self.source_and_line(line - 1) {
            Some((source, line)) => (source, Some(line)),
            None => (None, Some(line as i64)),
        };
        let (source, line) = match at {
            At::Top(Some(line)) => lookup(line as isize),
            At::TopSigned(line) => lookup(line),
            At::Top(None) => match self.source_and_line(self.top_offset) {
                Some((source, line)) => (source, Some(line)),
                None => (None, None),
            },
            At::Exact(line) => (None, line.map(|l| l as i64)),
            At::Source(source, line) => (source, line.map(|l| l as i64)),
        };
        let mut msg = Element::new(Kind::SystemMessage)
            .with_attr("level", level)
            .with_attr("type", level_name(level));
        if let Some(line) = line {
            msg.attributes.set("line", line);
        }
        if let Some(source) = source.as_deref().or(self.source.as_deref()) {
            msg.attributes.set("source", source);
        }
        msg.children.push(paragraph(text).into());
        msg.children.extend(children);
        msg
    }

    // -- names and ids ----------------------------------------------------

    /// `node`'s key, assigning one if needed.
    pub(crate) fn key(&mut self, node: &mut Element) -> Key {
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
        self.set_id_prefixed(node, msgnode, line, None)
    }

    /// `set_id`, with a prefix for a generated id other than the tag name
    /// (docutils' `suggested_prefix`).
    pub(crate) fn set_id_prefixed(
        &mut self,
        node: &mut Element,
        msgnode: Option<MsgNode>,
        line: Option<usize>,
        suggested_prefix: Option<&str>,
    ) -> String {
        let key = self.key(node);
        if node.attributes.get_list("ids").is_empty() {
            let id = self.create_id(node, suggested_prefix);
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
        let last = ids.last().cloned().unwrap_or_default();
        for name in node.attributes.get_list("names") {
            self.nameids.insert(name.clone(), Some(last.clone()));
        }
        last
    }

    /// A new id for `node` (docutils' `document.create_id`, with the
    /// default `id_prefix` and `auto_id_prefix` settings).
    fn create_id(&mut self, node: &Element, suggested_prefix: Option<&str>) -> String {
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
            match suggested_prefix {
                Some(p) => format!("{p}-"),
                None => format!("{}-", make_id(node.kind.tag())),
            }
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
                    self.nameids.insert(name.to_owned(), None);
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
                self.nameids.insert(name.to_owned(), None);
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

    /// Notes a reference by name (docutils' `note_refname`).
    pub(crate) fn note_refname(&mut self, node: &mut Element) {
        let key = self.key(node);
        if let Some(name) = node.attributes.get_str("refname") {
            self.notes
                .refnames
                .entry(name.to_owned())
                .or_default()
                .push(key);
        }
    }

    /// Notes a reference by id (docutils' `note_refid`).
    pub(crate) fn note_refid(&mut self, node: &mut Element) {
        let key = self.key(node);
        if let Some(id) = node.attributes.get_str("refid") {
            self.notes
                .refids
                .entry(id.to_owned())
                .or_default()
                .push(key);
        }
    }

    /// Notes a target pointing to another target by name.
    pub(crate) fn note_indirect_target(&mut self, node: &mut Element) {
        let key = self.key(node);
        self.notes.indirect_targets.push(key);
        self.notes.snapshots.insert(key, node.clone());
        if !node.attributes.get_list("names").is_empty() {
            self.note_refname(node);
        }
    }

    /// Notes a footnote or citation, or a reference to one, in `list`.
    pub(crate) fn note_footnote_kind(&mut self, node: &mut Element, list: NoteList) {
        let key = self.key(node);
        match list {
            NoteList::AutoFootnote => {
                self.set_id(node, None, None);
                self.notes.autofootnotes.push(key);
            }
            NoteList::SymbolFootnote => {
                self.set_id(node, None, None);
                self.notes.symbol_footnotes.push(key);
            }
            NoteList::Footnote => {
                self.set_id(node, None, None);
                self.notes.footnotes.push(key);
            }
            NoteList::Citation => self.notes.citations.push(key),
            NoteList::AutoFootnoteRef => {
                self.set_id(node, None, None);
                self.notes.autofootnote_refs.push(key);
            }
            NoteList::SymbolFootnoteRef => {
                self.set_id(node, None, None);
                self.notes.symbol_footnote_refs.push(key);
            }
            NoteList::FootnoteRef | NoteList::CitationRef => {
                self.set_id(node, None, None);
                let name = node
                    .attributes
                    .get_str("refname")
                    .unwrap_or_default()
                    .to_owned();
                let map = if list == NoteList::FootnoteRef {
                    &mut self.notes.footnote_refs
                } else {
                    &mut self.notes.citation_refs
                };
                map.entry(name).or_default().push(key);
                self.note_refname(node);
            }
        }
        // docutils' transforms still change a noted node that never reaches
        // the tree, so they work on a copy of it.
        self.notes.snapshots.insert(key, node.clone());
    }

    /// Refreshes the copy of a noted element (see `note_footnote_kind`)
    /// once it is complete.
    pub(crate) fn update_snapshot(&mut self, node: &Element) {
        if let Some(AttrValue::Int(key)) = node.attributes.get(KEY)
            && let Some(copy) = self.notes.snapshots.get_mut(key)
        {
            *copy = node.clone();
        }
    }

    /// Notes a `pending` element for the transform at `priority`.
    pub(crate) fn note_pending(&mut self, node: &mut Element, priority: u32) {
        let key = self.key(node);
        self.notes.pending.push((priority, key));
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
        self.notes
            .substitution_names
            .insert(crate::util::normalize_name(&name), name.clone());
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

/// The lists of [`Document::note_footnote_kind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoteList {
    AutoFootnote,
    SymbolFootnote,
    Footnote,
    Citation,
    AutoFootnoteRef,
    SymbolFootnoteRef,
    FootnoteRef,
    CitationRef,
}

/// What docutils' `document.note_*` methods record for the transforms,
/// as element keys in the order noted.
#[derive(Debug, Default)]
pub(crate) struct Notes {
    pub(crate) refnames: HashMap<String, Vec<Key>>,
    pub(crate) refids: HashMap<String, Vec<Key>>,
    pub(crate) indirect_targets: Vec<Key>,
    pub(crate) autofootnotes: Vec<Key>,
    pub(crate) symbol_footnotes: Vec<Key>,
    pub(crate) footnotes: Vec<Key>,
    pub(crate) citations: Vec<Key>,
    pub(crate) autofootnote_refs: Vec<Key>,
    pub(crate) symbol_footnote_refs: Vec<Key>,
    pub(crate) footnote_refs: HashMap<String, Vec<Key>>,
    pub(crate) citation_refs: HashMap<String, Vec<Key>>,
    /// Case-insensitive substitution names.
    pub(crate) substitution_names: HashMap<String, String>,
    /// Pending elements and their transforms' priorities.
    pub(crate) pending: Vec<(u32, Key)>,
    /// Copies of noted indirect targets: docutils reports a broken one
    /// even if it never reached the tree (say, in a discarded title).
    pub(crate) snapshots: HashMap<Key, Element>,
}

/// Moves `name` from `node`'s names to its dupnames.
pub(crate) fn dupname(node: &mut Element, name: &str) {
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

pub(crate) fn find_in(element: &mut Element, key: Key) -> Option<&mut Element> {
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
    let hidden: Vec<String> = element
        .attributes
        .iter()
        .filter(|(name, _)| name.starts_with('\0'))
        .map(|(name, _)| name.to_owned())
        .collect();
    for name in hidden {
        element.attributes.remove(&name);
    }
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
