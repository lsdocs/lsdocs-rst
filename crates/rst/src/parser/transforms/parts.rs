//! docutils' document part transforms (`docutils.transforms.parts`), and
//! `references.TargetNotes`, which the `pending` elements of the
//! `contents`, `sectnum` and `target-notes` directives ask for.

use std::collections::HashMap;

use super::{at_path, collect, first_id, get, has, path_of, remove, replace_self, str_attr};
use crate::nodes::{AttrValue, Element, Kind, Node, Text};
use crate::parser::document::{Document, KEY, Key, MsgNode, NoteList};

fn detail_int(e: &Element, name: &str) -> Option<i64> {
    match e.attributes.get(&format!(".details.{name}")) {
        Some(AttrValue::Int(n)) => Some(*n),
        _ => None,
    }
}

fn detail_str(e: &Element, name: &str) -> String {
    e.attributes
        .get_str(&format!(".details.{name}"))
        .unwrap_or_default()
        .to_owned()
}

// -- sectnum ------------------------------------------------------------------

/// Numbers the sections (docutils' `parts.SectNum`).
pub(super) fn sectnum(_doc: &mut Document, root: &mut Element, key: Key) {
    let Some(pending) = get(root, key).cloned() else {
        return;
    };
    let numbering = Numbering {
        depth: detail_int(&pending, "depth").unwrap_or(i64::MAX),
        start: detail_int(&pending, "start").unwrap_or(1),
        prefix: detail_str(&pending, "prefix"),
        suffix: detail_str(&pending, "suffix"),
    };
    remove(root, key);
    numbering.number(root, &[], 0);
}

struct Numbering {
    depth: i64,
    start: i64,
    prefix: String,
    suffix: String,
}

impl Numbering {
    fn number(&self, node: &mut Element, prefix: &[String], depth: i64) {
        let depth = depth + 1;
        let mut sectnum = if prefix.is_empty() { self.start } else { 1 };
        for child in &mut node.children {
            let Node::Element(section) = child else {
                continue;
            };
            if section.kind != Kind::Section {
                continue;
            }
            let mut numbers = prefix.to_vec();
            numbers.push(sectnum.to_string());
            if let Some(Node::Element(title)) = section.children.first_mut() {
                let text = format!(
                    "{}{}{}\u{a0}\u{a0}\u{a0}",
                    self.prefix,
                    numbers.join("."),
                    self.suffix
                );
                let generated = Element::new(Kind::Generated)
                    .with_attr("classes", vec!["sectnum".to_owned()])
                    .with_child(Text::new(text, None));
                title.children.insert(0, generated.into());
                title.attributes.set("auto", 1i64);
            }
            if depth < self.depth {
                self.number(section, &numbers, depth);
            }
            sectnum += 1;
        }
    }
}

// -- contents -----------------------------------------------------------------

/// Builds a table of contents in place of the `contents` directive's
/// pending element (docutils' `parts.Contents`).
pub(super) fn contents(doc: &mut Document, root: &mut Element, key: Key) {
    let Some(path) = path_of(root, key) else {
        return;
    };
    let pending = at_path(root, &path).clone();
    let topic_path = &path[..path.len() - 1];
    let topic_key;
    let toc_id = {
        let topic = at_path(root, topic_path);
        topic_key = doc.key(topic);
        doc.set_id(topic, None, None)
    };
    let mut start = Vec::new();
    if has(&pending, ".details.local") {
        start = topic_path.to_vec();
        start.pop();
        while !start.is_empty() && at_path(root, &start).kind != Kind::Section {
            start.pop();
        }
    }
    let backlinks = match pending.attributes.get(".details.backlinks") {
        None => Some("entry".to_owned()),
        Some(AttrValue::Str(s)) => Some(s.clone()),
        Some(_) => None,
    };
    let toc = Toc {
        depth: detail_int(&pending, "depth").unwrap_or(i64::MAX),
        backlinks,
        toc_id,
    };
    match toc.build(doc, root, &start, 0) {
        Some(list) => replace_self(root, key, vec![list.into()]),
        None => remove(root, topic_key),
    }
}

struct Toc {
    depth: i64,
    backlinks: Option<String>,
    toc_id: String,
}

impl Toc {
    fn build(
        &self,
        doc: &mut Document,
        root: &mut Element,
        path: &[usize],
        level: i64,
    ) -> Option<Element> {
        let level = level + 1;
        let sections: Vec<usize> = at_path(root, path)
            .children
            .iter()
            .enumerate()
            .filter(|(_, n)| matches!(n, Node::Element(e) if e.kind == Kind::Section))
            .map(|(i, _)| i)
            .collect();
        let mut entries = Vec::new();
        let mut auto = false;
        for i in sections {
            let mut section_path = path.to_vec();
            section_path.push(i);
            let section = at_path(root, &section_path);
            let section_id = first_id(section).unwrap_or_default();
            let Some(Node::Element(title)) = section.children.first() else {
                continue;
            };
            auto = has(title, "auto");
            let has_reference = contains_reference(title);
            let mut reference = Element::new(Kind::Reference)
                .with_children(filter_copy(title))
                .with_attr("refid", section_id);
            let ref_id = doc.set_id_prefixed(&mut reference, None, None, Some("toc-entry"));
            let entry = Element::new(Kind::Paragraph).with_child(reference);
            let mut item = Element::new(Kind::ListItem).with_child(entry);
            if !has_reference {
                let refid = match self.backlinks.as_deref() {
                    Some("entry") => Some(ref_id),
                    Some("top") => Some(self.toc_id.clone()),
                    _ => None,
                };
                if let Some(refid) = refid {
                    let mut title_path = section_path.clone();
                    title_path.push(0);
                    at_path(root, &title_path).attributes.set("refid", refid);
                }
            }
            if level < self.depth
                && let Some(sub) = self.build(doc, root, &section_path, level)
            {
                item.children.push(sub.into());
            }
            entries.push(Node::from(item));
        }
        if entries.is_empty() {
            return None;
        }
        let mut list = Element::new(Kind::BulletList).with_children(entries);
        if auto {
            list.attributes.push("classes", "auto-toc");
        }
        Some(list)
    }
}

fn contains_reference(e: &Element) -> bool {
    e.children.iter().any(|c| match c {
        Node::Element(c) => c.kind == Kind::Reference || contains_reference(c),
        Node::Text(_) => false,
    })
}

/// A copy of a title's content for a contents entry, without footnote and
/// citation references, and with references, targets and problematic
/// elements reduced to their content (docutils' `ContentsFilter`).
fn filter_copy(e: &Element) -> Vec<Node> {
    let mut out = Vec::new();
    for child in &e.children {
        match child {
            Node::Text(t) => out.push(Node::Text(t.clone())),
            Node::Element(c) => match c.kind {
                Kind::CitationReference | Kind::FootnoteReference => {}
                Kind::Image => {
                    if let Some(alt) = c.attributes.get_str("alt") {
                        out.push(Text::new(alt, None).into());
                    }
                }
                Kind::Problematic | Kind::Reference | Kind::Target => {
                    out.extend(filter_copy(c));
                }
                _ => {
                    let mut copy = Element::new(c.kind.clone());
                    copy.attributes = c.attributes.clone();
                    copy.attributes.remove(KEY);
                    copy.span = c.span;
                    copy.children = filter_copy(c);
                    out.push(copy.into());
                }
            },
        }
    }
    out
}

// -- target notes -------------------------------------------------------------

/// Footnotes for the URIs of external targets and anonymous references,
/// referenced after each reference (docutils' `references.TargetNotes`).
pub(super) fn target_notes(doc: &mut Document, root: &mut Element, key: Key) {
    let Some(pending) = get(root, key).cloned() else {
        return;
    };
    let mut notes = TargetNotes {
        classes: pending.attributes.get_list(".details.class").to_vec(),
        by_uri: HashMap::new(),
        footnotes: Vec::new(),
    };
    let targets = collect(doc, root, &|e| e.kind == Kind::Target);
    for tk in targets {
        let Some(target) = get(root, tk).cloned() else {
            continue;
        };
        if !has(&target, "refuri") {
            continue;
        }
        let mut refs = Vec::new();
        for name in target.attributes.get_list("names") {
            refs.extend(doc.notes.refnames.get(name).cloned().unwrap_or_default());
        }
        if refs.is_empty() {
            continue;
        }
        let uri = str_attr(&target, "refuri").unwrap_or_default();
        notes.footnote(doc, root, &uri, &refs);
    }
    let anonymous = collect(doc, root, &|e| {
        e.kind == Kind::Reference
            && matches!(e.attributes.get("anonymous"), Some(AttrValue::Bool(true)))
    });
    for rk in anonymous {
        let Some(uri) = get(root, rk).and_then(|r| str_attr(r, "refuri")) else {
            continue;
        };
        notes.footnote(doc, root, &uri, &[rk]);
    }
    let footnotes = notes.footnotes.into_iter().map(Node::from).collect();
    replace_self(root, key, footnotes);
}

struct TargetNotes {
    classes: Vec<String>,
    by_uri: HashMap<String, usize>,
    footnotes: Vec<Element>,
}

impl TargetNotes {
    /// The footnote for `uri`, made if needed, referenced after `refs`.
    fn footnote(&mut self, doc: &mut Document, root: &mut Element, uri: &str, refs: &[Key]) {
        let name = match self.by_uri.get(uri) {
            Some(&i) => self.footnotes[i].attributes.get_list("names")[0].clone(),
            None => {
                let mut footnote = Element::new(Kind::Footnote);
                let id = doc.set_id(&mut footnote, None, None);
                let name = format!("TARGET_NOTE: {id}");
                footnote.attributes.set("auto", 1i64);
                footnote.attributes.set("names", vec![name.clone()]);
                let reference = Element::new(Kind::Reference)
                    .with_attr("refuri", uri)
                    .with_child(Text::new(uri, None));
                footnote
                    .children
                    .push(Element::new(Kind::Paragraph).with_child(reference).into());
                doc.note_footnote_kind(&mut footnote, NoteList::AutoFootnote);
                doc.note_explicit_target(&mut footnote, Some(MsgNode::Itself), None);
                self.by_uri.insert(uri.to_owned(), self.footnotes.len());
                self.footnotes.push(footnote);
                name
            }
        };
        for &rk in refs {
            let Some(path) = path_of(root, rk) else {
                continue;
            };
            if at_path(root, &path).kind == Kind::Target {
                continue;
            }
            let mut refnode = Element::new(Kind::FootnoteReference)
                .with_attr("refname", name.as_str())
                .with_attr("auto", 1i64);
            for c in &self.classes {
                refnode.attributes.push("classes", c.clone());
            }
            doc.note_footnote_kind(&mut refnode, NoteList::AutoFootnoteRef);
            doc.note_footnote_kind(&mut refnode, NoteList::FootnoteRef);
            let space: Node = if self.classes.is_empty() {
                Text::new(" ", None).into()
            } else {
                Element::new(Kind::Inline)
                    .with_attr("classes", self.classes.clone())
                    .with_child(Text::new(" ", None))
                    .into()
            };
            let Some((&index, parent_path)) = path.split_last() else {
                continue;
            };
            let parent = at_path(root, parent_path);
            parent
                .children
                .splice(index + 1..index + 1, [space, refnode.into()]);
        }
    }
}
