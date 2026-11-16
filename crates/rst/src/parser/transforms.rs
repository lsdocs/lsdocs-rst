//! docutils' transforms (`docutils.transforms`), run after parsing in
//! docutils' priority order: what `publish_doctree` applies with default
//! settings, and the transforms `pending` elements ask for.
//!
//! Messages the transforms report go only to docutils' reporter, so they
//! are not in the tree; the `problematic` elements that replace bad
//! references point to them by id. A few transforms do put their
//! messages in the tree, as docutils does.

mod frontmatter;
mod parts;

use std::collections::{HashMap, HashSet};

use super::document::{At, Document, ERROR, KEY, Key, LINE, MsgNode, RAW, WARNING, find_in, fixed};
use crate::nodes::{AttrValue, Element, Kind, Node, Text};

// -- tree helpers -------------------------------------------------------------

fn key_of(e: &Element) -> Option<Key> {
    match e.attributes.get(KEY) {
        Some(AttrValue::Int(k)) => Some(*k),
        _ => None,
    }
}

/// The path (child indexes) to the element with `key`.
fn path_of(root: &Element, key: Key) -> Option<Vec<usize>> {
    fn walk(e: &Element, key: Key, path: &mut Vec<usize>) -> bool {
        for (i, child) in e.children.iter().enumerate() {
            if let Node::Element(c) = child {
                path.push(i);
                if key_of(c) == Some(key) || walk(c, key, path) {
                    return true;
                }
                path.pop();
            }
        }
        false
    }
    let mut path = Vec::new();
    walk(root, key, &mut path).then_some(path)
}

fn at_path<'a>(root: &'a mut Element, path: &[usize]) -> &'a mut Element {
    let mut e = root;
    for &i in path {
        e = match &mut e.children[i] {
            Node::Element(c) => c,
            Node::Text(_) => unreachable!("paths lead to elements"),
        };
    }
    e
}

fn get(root: &mut Element, key: Key) -> Option<&mut Element> {
    find_in(root, key)
}

/// The elements matching `pred`, in document order, given keys if they
/// have none.
fn collect(doc: &mut Document, root: &mut Element, pred: &dyn Fn(&Element) -> bool) -> Vec<Key> {
    fn walk(
        doc: &mut Document,
        e: &mut Element,
        pred: &dyn Fn(&Element) -> bool,
        out: &mut Vec<Key>,
    ) {
        for child in &mut e.children {
            if let Node::Element(c) = child {
                if pred(c) {
                    out.push(doc.key(c));
                }
                walk(doc, c, pred, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(doc, root, pred, &mut out);
    out
}

fn has(e: &Element, attr: &str) -> bool {
    e.attributes.get(attr).is_some()
}

fn str_attr(e: &Element, attr: &str) -> Option<String> {
    e.attributes.get_str(attr).map(str::to_owned)
}

fn first_id(e: &Element) -> Option<String> {
    e.attributes.get_list("ids").first().cloned()
}

const RESOLVED: &str = "\0resolved";

fn resolved(e: &Element) -> bool {
    has(e, RESOLVED)
}

fn set_resolved(e: &mut Element) {
    e.attributes.set(RESOLVED, true);
}

/// docutils' `replace_self`: `new` takes `key`'s place; the first new
/// element gets the old one's ids, names, classes and dupnames.
fn replace_self(root: &mut Element, key: Key, mut new: Vec<Node>) {
    let Some(path) = path_of(root, key) else {
        return;
    };
    let Some((index, parent_path)) = path.split_last() else {
        return;
    };
    let parent = at_path(root, parent_path);
    let Node::Element(old) = parent.children[*index].clone() else {
        return;
    };
    if let Some(Node::Element(first)) = new.first_mut() {
        for attr in ["ids", "classes", "names", "dupnames"] {
            for v in old.attributes.get_list(attr) {
                if !first.attributes.get_list(attr).contains(v) {
                    first.attributes.push(attr, v.clone());
                }
            }
        }
    }
    parent.children.splice(*index..=*index, new);
}

fn remove(root: &mut Element, key: Key) {
    replace_self(root, key, Vec::new());
}

/// A `problematic` element for a reference, pointing to `msgid`.
fn problematic(doc: &mut Document, raw: &str, msgid: &str) -> Element {
    let mut prb = Element::new(Kind::Problematic).with_attr("refid", msgid);
    if !raw.is_empty() {
        prb.children.push(Text::new(raw, None).into());
    }
    let _ = doc.key(&mut prb);
    prb
}

/// Runs `f` on the element with `key`: in the tree, or else on the copy of
/// a noted element that never reached it, which docutils' transforms
/// still change (the copy keeps the changes).
fn with_node<R>(
    doc: &mut Document,
    root: &mut Element,
    key: Key,
    f: impl FnOnce(&mut Document, &mut Element) -> R,
) -> Option<R> {
    if let Some(e) = find_in(root, key) {
        return Some(f(doc, e));
    }
    let mut copy = doc.notes.snapshots.remove(&key)?;
    let result = f(doc, &mut copy);
    doc.notes.snapshots.insert(key, copy);
    Some(result)
}

/// Replaces the references `keys` (those not passing `skip`) with
/// problematic elements linked to a new error `message`.
fn report_problem(
    doc: &mut Document,
    root: &mut Element,
    message: &str,
    keys: &[Key],
    skip: &dyn Fn(&Element) -> bool,
) {
    let mut msg = doc.message(ERROR, message, Vec::new(), At::Exact(None));
    let msgid = doc.set_id(&mut msg, None, None);
    for &key in keys {
        let Some((skipped, raw, span)) = with_node(doc, root, key, |_, e| {
            (skip(e), str_attr(e, RAW).unwrap_or_default(), e.span)
        }) else {
            continue;
        };
        if skipped {
            continue;
        }
        let mut prb = problematic(doc, &raw, &msgid);
        prb.span = span;
        let prbid = doc.set_id(&mut prb, None, None);
        msg.attributes.push("backrefs", prbid);
        replace_self(root, key, vec![prb.into()]);
    }
    doc.transform_messages.push(msg);
}

// -- running ------------------------------------------------------------------

/// Applies the transforms.
pub(crate) fn apply(doc: &mut Document, root: &mut Element) {
    let mut queue: Vec<(u32, usize, Option<Key>)> = Vec::new();
    for (i, &(priority, key)) in doc.notes.pending.iter().enumerate() {
        queue.push((priority, i, Some(key)));
    }
    let base = queue.len();
    for (i, priority) in [
        220, 260, 320, 340, 350, 440, 460, 620, 640, 660, 740, 830, 850,
    ]
    .into_iter()
    .enumerate()
    {
        queue.push((priority, base + i, None));
    }
    queue.sort();
    for (priority, _, pending) in queue {
        match (priority, pending) {
            (210, Some(key)) => class_attribute(doc, root, key),
            (540, Some(key)) => parts::target_notes(doc, root, key),
            (710, Some(key)) => parts::sectnum(doc, root, key),
            (720, Some(key)) => parts::contents(doc, root, key),
            (_, Some(_)) => {}
            (220, None) => substitutions(doc, root),
            (260, None) => propagate_targets(doc, root),
            (320, None) => frontmatter::doc_title(doc, root),
            (340, None) => frontmatter::doc_info(doc, root),
            (350, None) => frontmatter::section_subtitles(doc, root),
            (440, None) => anonymous_hyperlinks(doc, root),
            (460, None) => indirect_hyperlinks(doc, root),
            (620, None) => footnotes(doc, root),
            (640, None) => external_targets(doc, root),
            (660, None) => internal_targets(doc, root),
            (740, None) => strip_comments(doc, root),
            (830, None) => transitions(doc, root),
            (850, None) => dangling_references(doc, root),
            _ => {}
        }
    }
}

// -- misc ---------------------------------------------------------------------

/// Removes comments, with `strip_comments` (docutils' `StripComments`).
fn strip_comments(doc: &mut Document, root: &mut Element) {
    fn strip(e: &mut Element) {
        e.children
            .retain(|c| !matches!(c, Node::Element(c) if c.kind == Kind::Comment));
        for child in &mut e.children {
            if let Node::Element(c) = child {
                strip(c);
            }
        }
    }
    if doc.extensions.settings.strip_comments {
        strip(root);
    }
}

fn is_invisible(kind: &Kind) -> bool {
    matches!(
        kind,
        Kind::Comment | Kind::Pending | Kind::SubstitutionDefinition | Kind::Target
    )
}

/// The `class` directive's pending element: its classes go on the next
/// element (docutils' `misc.ClassAttribute`).
fn class_attribute(doc: &mut Document, root: &mut Element, key: Key) {
    let Some(path) = path_of(root, key) else {
        return;
    };
    let pending = at_path(root, &path).clone();
    let classes = pending.attributes.get_list(".details.class").to_vec();
    // Following siblings, then those of each ancestor.
    let mut p = path.clone();
    while let Some(index) = p.pop() {
        let parent = at_path(root, &p);
        for i in index + 1..parent.children.len() {
            if let Node::Element(e) = &mut parent.children[i] {
                if is_invisible(&e.kind) || e.kind == Kind::SystemMessage {
                    continue;
                }
                for c in &classes {
                    e.attributes.push("classes", c.clone());
                }
                remove(root, key);
                return;
            }
        }
    }
    let raw = str_attr(&pending, RAW).unwrap_or_default();
    let line = match pending.attributes.get(LINE) {
        Some(AttrValue::Int(n)) => Some(*n as usize),
        _ => None,
    };
    let directive = str_attr(&pending, ".details.directive").unwrap_or_default();
    let mut error = doc.message(
        ERROR,
        &format!("No suitable element following \"{directive}\" directive"),
        vec![fixed(Kind::LiteralBlock, &raw, None).into()],
        // Without a line, docutils' reporter takes where the top-level
        // parser stopped.
        At::Top(line),
    );
    if !has(&error, "line") {
        // docutils passes `line=None`, which pseudo-XML prints as "True".
        error.attributes.set("line", "True");
    }
    replace_self(root, key, vec![error.into()]);
}

/// Transitions must sit between body elements of a document or section
/// (docutils' `misc.Transitions`).
fn transitions(doc: &mut Document, root: &mut Element) {
    let keys = collect(doc, root, &|e| e.kind == Kind::Transition);
    for key in keys {
        visit_transition(doc, root, key);
    }
}

fn ignored_sibling(n: &Node) -> bool {
    matches!(n, Node::Element(e) if matches!(
        e.kind,
        Kind::Decoration | Kind::Meta | Kind::Pending | Kind::SubstitutionDefinition | Kind::Subtitle | Kind::Target | Kind::Title
    ))
}

fn visit_transition(doc: &mut Document, root: &mut Element, key: Key) {
    let Some(path) = path_of(root, key) else {
        return;
    };
    let Some((&index, parent_path)) = path.split_last() else {
        return;
    };
    let parent = at_path(root, parent_path).clone();
    if !matches!(parent.kind, Kind::Document | Kind::Section) {
        warn_transition(
            doc,
            root,
            key,
            "Transition only valid as child of <document> or <section>.",
        );
        return;
    }
    let previous_is_transition = index > 0
        && matches!(&parent.children[index - 1], Node::Element(e) if e.kind == Kind::Transition);
    let begins = parent.children[..index].iter().all(ignored_sibling);
    let ends = parent.children[index + 1..].iter().all(ignored_sibling);
    if ends {
        // Move the transition after the nearest ancestor with a next sibling.
        let mut moved = false;
        let mut sibling_path = parent_path.to_vec();
        while let Some(sib_index) = sibling_path.pop() {
            let grand = at_path(root, &sibling_path);
            if sib_index + 1 < grand.children.len() {
                let node = {
                    let parent = at_path(root, parent_path);
                    parent.children.remove(index)
                };
                let grand = at_path(root, &sibling_path);
                grand.children.insert(sib_index + 1, node);
                moved = true;
                break;
            }
        }
        if !moved {
            warn_transition(doc, root, key, "Transition at the end of the document.");
        }
    }
    if begins {
        let tag = match path_of(root, key).and_then(|p| {
            let (_, pp) = p.split_last()?;
            Some(at_path(root, pp).kind.tag().to_owned())
        }) {
            Some(t) => t,
            None => return,
        };
        warn_transition(
            doc,
            root,
            key,
            &format!("Transition at the start of the {tag}."),
        );
    } else if previous_is_transition {
        warn_transition(
            doc,
            root,
            key,
            "At least one body element should separate transitions.",
        );
    }
}

fn warn_transition(doc: &mut Document, root: &mut Element, key: Key, text: &str) {
    let Some(path) = path_of(root, key) else {
        return;
    };
    let line = match at_path(root, &path).attributes.get(LINE) {
        Some(AttrValue::Int(n)) => Some(*n as usize),
        _ => None,
    };
    let warning = doc.message(WARNING, text, Vec::new(), At::node(line));
    let Some((&index, parent_path)) = path.split_last() else {
        return;
    };
    let parent = at_path(root, parent_path);
    if matches!(
        parent.kind,
        Kind::Document | Kind::Section | Kind::Topic | Kind::Sidebar
    ) {
        parent.children.insert(index + 1, warning.into());
    }
}

// -- references -------------------------------------------------------------------

/// Substitution references are replaced by the definitions' contents
/// (docutils' `references.Substitutions`).
fn substitutions(doc: &mut Document, root: &mut Element) {
    let mut refs = collect(doc, root, &|e| e.kind == Kind::SubstitutionReference);
    let mut nested: HashMap<String, Vec<String>> = HashMap::new();
    let mut origin: HashMap<Key, Key> = HashMap::new();
    let mut i = 0;
    while i < refs.len() {
        let key = refs[i];
        i += 1;
        let Some(r) = get(root, key) else {
            continue;
        };
        let refname = str_attr(r, "refname").unwrap_or_default();
        let raw = str_attr(r, RAW).unwrap_or_default();
        let name = if doc.substitution_defs.contains_key(&refname) {
            Some(refname.clone())
        } else {
            doc.notes
                .substitution_names
                .get(&refname.to_lowercase())
                .cloned()
        };
        let Some(name) = name else {
            report_problem(
                doc,
                root,
                &format!("Undefined substitution referenced: \"{refname}\"."),
                &[key],
                &|_| false,
            );
            let _ = raw;
            continue;
        };
        let def_key = doc.substitution_defs[&name];
        let Some(def) = get(root, def_key).cloned() else {
            continue;
        };
        // Trim the text around the reference.
        let Some(path) = path_of(root, key) else {
            continue;
        };
        let Some((&index, parent_path)) = path.split_last() else {
            continue;
        };
        {
            let parent = at_path(root, parent_path);
            let trim = |a: &str| has(&def, a) || has(&def, "trim");
            if trim("ltrim")
                && index > 0
                && let Node::Text(t) = &mut parent.children[index - 1]
            {
                t.value = crate::util::rstrip(&t.value).to_owned();
            }
            if trim("rtrim")
                && let Some(Node::Text(t)) = parent.children.get_mut(index + 1)
            {
                t.value = crate::util::lstrip(&t.value).to_owned();
            }
        }
        // A fresh copy, with fresh keys for nested references.
        let mut copy = def.clone();
        let mut circular = false;
        let mut new_nested = Vec::new();
        fresh_keys(doc, &mut copy, &mut new_nested);
        for nested_key in &new_nested {
            let Some(nested_ref) = find_in(&mut copy, *nested_key) else {
                continue;
            };
            let nested_refname = str_attr(nested_ref, "refname")
                .unwrap_or_default()
                .to_lowercase();
            let Some(nested_name) = doc.notes.substitution_names.get(&nested_refname).cloned()
            else {
                continue;
            };
            let seen = nested.entry(nested_name).or_default();
            if seen.contains(&name) {
                circular = true;
                break;
            }
            seen.push(name.clone());
            origin.insert(*nested_key, key);
            refs.push(*nested_key);
        }
        if circular {
            let parent = at_path(root, parent_path).clone();
            if parent.kind == Kind::SubstitutionDefinition {
                let raw = str_attr(&parent, RAW).unwrap_or_default();
                let msg = doc.message(
                    ERROR,
                    "Circular substitution definition detected:",
                    vec![fixed(Kind::LiteralBlock, &raw, None).into()],
                    At::Exact(None),
                );
                if let Some(pkey) = key_of(&parent) {
                    replace_self(root, pkey, vec![msg.into()]);
                }
            } else {
                report_problem(
                    doc,
                    root,
                    &format!("Circular substitution definition referenced: \"{refname}\"."),
                    &[key],
                    &|_| false,
                );
            }
            continue;
        }
        let mut children = copy.children;
        for child in &mut children {
            if let Node::Element(e) = child
                && matches!(
                    e.kind,
                    Kind::Reference | Kind::FootnoteReference | Kind::CitationReference
                )
                && has(e, "refname")
            {
                doc.note_refname(e);
            }
        }
        replace_self(root, key, children);
    }
}

/// Gives the elements in `e` new keys, listing substitution references.
fn fresh_keys(doc: &mut Document, e: &mut Element, subrefs: &mut Vec<Key>) {
    for child in &mut e.children {
        if let Node::Element(c) = child {
            c.attributes.remove(KEY);
            if c.kind == Kind::SubstitutionReference {
                subrefs.push(doc.key(c));
            }
            fresh_keys(doc, c, subrefs);
        }
    }
}

/// Block-level targets pass their ids and names to the next element
/// (docutils' `references.PropagateTargets`).
fn propagate_targets(doc: &mut Document, root: &mut Element) {
    let targets = collect(doc, root, &|e| e.kind == Kind::Target);
    for key in targets {
        let Some(path) = path_of(root, key) else {
            continue;
        };
        let Some((&index, parent_path)) = path.split_last() else {
            continue;
        };
        let parent_kind = at_path(root, parent_path).kind.clone();
        let target = at_path(root, &path).clone();
        if is_text_element(&parent_kind)
            || has(&target, "refid")
            || has(&target, "refuri")
            || has(&target, "refname")
        {
            continue;
        }
        // The next node in document order, skipping system messages.
        let mut next: Option<Vec<usize>> = None;
        let mut p = parent_path.to_vec();
        let mut i = index;
        loop {
            let parent = at_path(root, &p);
            if i + 1 < parent.children.len() {
                let mut candidate = p.clone();
                candidate.push(i + 1);
                match &parent.children[i + 1] {
                    Node::Element(e) if e.kind == Kind::SystemMessage => {
                        i += 1;
                        continue;
                    }
                    _ => {
                        next = Some(candidate);
                        break;
                    }
                }
            }
            match p.pop() {
                Some(up) => i = up,
                None => break,
            }
        }
        let Some(next_path) = next else {
            continue;
        };
        let Some((&ni, np)) = next_path.split_last() else {
            continue;
        };
        let next_kind = match &at_path(root, np).children[ni] {
            Node::Element(e) => e.kind.clone(),
            Node::Text(_) => continue,
        };
        if (is_invisible(&next_kind) || matches!(next_kind, Kind::Footnote | Kind::Citation))
            && next_kind != Kind::Target
        {
            continue;
        }
        let ids = target.attributes.get_list("ids").to_vec();
        let names = target.attributes.get_list("names").to_vec();
        let next_key = {
            let next = at_path(root, &next_path);
            for id in &ids {
                next.attributes.push("ids", id.clone());
            }
            for name in &names {
                next.attributes.push("names", name.clone());
            }
            doc.key(next)
        };
        for id in &ids {
            doc.ids.insert(id.clone(), next_key);
        }
        if parent_kind == Kind::Figure && next_kind == Kind::Caption {
            remove(root, key);
            continue;
        }
        let target = at_path(root, &path);
        if let Some(first) = ids.first() {
            target.attributes.set("refid", first.clone());
        }
        target.attributes.set("ids", Vec::<String>::new());
        target.attributes.set("names", Vec::<String>::new());
        doc.note_refid(target);
    }
}

/// docutils' `TextElement` kinds.
fn is_text_element(kind: &Kind) -> bool {
    matches!(
        kind,
        Kind::Abbreviation
            | Kind::Acronym
            | Kind::Address
            | Kind::Attribution
            | Kind::Author
            | Kind::Caption
            | Kind::CitationReference
            | Kind::Classifier
            | Kind::Comment
            | Kind::Contact
            | Kind::Copyright
            | Kind::Date
            | Kind::DoctestBlock
            | Kind::Emphasis
            | Kind::FieldName
            | Kind::FootnoteReference
            | Kind::Generated
            | Kind::Inline
            | Kind::Label
            | Kind::Line
            | Kind::Literal
            | Kind::LiteralBlock
            | Kind::Math
            | Kind::MathBlock
            | Kind::OptionArgument
            | Kind::OptionString
            | Kind::Organization
            | Kind::Paragraph
            | Kind::Problematic
            | Kind::Raw
            | Kind::Reference
            | Kind::Revision
            | Kind::Rubric
            | Kind::Status
            | Kind::Strong
            | Kind::Subscript
            | Kind::SubstitutionDefinition
            | Kind::SubstitutionReference
            | Kind::Subtitle
            | Kind::Superscript
            | Kind::Target
            | Kind::Term
            | Kind::Title
            | Kind::TitleReference
            | Kind::Version
    )
}

/// Anonymous references take anonymous targets in order (docutils'
/// `references.AnonymousHyperlinks`).
fn anonymous_hyperlinks(doc: &mut Document, root: &mut Element) {
    let anonymous = |kind: Kind| {
        move |e: &Element| {
            e.kind == kind && matches!(e.attributes.get("anonymous"), Some(AttrValue::Bool(true)))
        }
    };
    let refs = collect(doc, root, &anonymous(Kind::Reference));
    let targets = collect(doc, root, &anonymous(Kind::Target));
    if refs.len() != targets.len() {
        report_problem(
            doc,
            root,
            &format!(
                "Anonymous hyperlink mismatch: {} references but {} targets.\nSee \"backrefs\" attribute for IDs.",
                refs.len(),
                targets.len()
            ),
            &refs,
            &|_| false,
        );
        return;
    }
    for (rk, tk) in refs.into_iter().zip(targets) {
        let Some(r) = get(root, rk) else {
            continue;
        };
        if has(r, "refid") || has(r, "refuri") {
            continue;
        }
        let mut target_key = tk;
        let mut seen = HashSet::new();
        loop {
            if !seen.insert(target_key) {
                break;
            }
            let Some(t) = get(root, target_key).cloned() else {
                break;
            };
            if let Some(uri) = str_attr(&t, "refuri") {
                let Some(r) = get(root, rk) else {
                    return;
                };
                r.attributes.set("refuri", uri);
                set_resolved(r);
                break;
            }
            match first_id(&t) {
                None => {
                    let Some(next) = str_attr(&t, "refid").and_then(|id| doc.ids.get(&id).copied())
                    else {
                        break;
                    };
                    target_key = next;
                }
                Some(id) => {
                    let Some(r) = get(root, rk) else {
                        return;
                    };
                    r.attributes.set("refid", id);
                    doc.note_refid(r);
                    break;
                }
            }
        }
    }
}

/// Targets that refer to other targets by name get those targets'
/// destinations (docutils' `references.IndirectHyperlinks`).
fn indirect_hyperlinks(doc: &mut Document, root: &mut Element) {
    let targets = doc.notes.indirect_targets.clone();
    for key in targets {
        let unresolved = target_or_copy(doc, root, key).is_some_and(|t| !resolved(&t));
        if unresolved {
            let mut active = HashSet::new();
            resolve_indirect_target(doc, root, key, &mut active);
        }
        resolve_indirect_references(doc, root, key);
    }
}

/// A noted target: in the tree, or the copy taken when it was noted.
fn target_or_copy(doc: &Document, root: &mut Element, key: Key) -> Option<Element> {
    get(root, key)
        .cloned()
        .or_else(|| doc.notes.snapshots.get(&key).cloned())
}

fn resolve_indirect_target(
    doc: &mut Document,
    root: &mut Element,
    key: Key,
    active: &mut HashSet<Key>,
) {
    let Some(target) = target_or_copy(doc, root, key) else {
        return;
    };
    let refname = str_attr(&target, "refname");
    let mut refid = str_attr(&target, "refid");
    let reftarget = match &refid {
        Some(id) => doc.ids.get(id).copied(),
        None => {
            let name = refname.clone().unwrap_or_default();
            let found = doc.names.get(&name).copied().flatten();
            refid = doc.nameids.get(&name).cloned().flatten();
            if let Some(k) = found
                && refid.is_none()
                && let Some(t) = get(root, k)
            {
                refid = Some(doc.set_id(t, None, None));
            }
            found
        }
    };
    let Some(rk) = reftarget else {
        nonexistent_indirect_target(doc, root, key);
        return;
    };
    let Some(reft) = get(root, rk).cloned() else {
        nonexistent_indirect_target(doc, root, key);
        return;
    };
    if reft.kind == Kind::Target && !resolved(&reft) && has(&reft, "refname") {
        if active.contains(&key) {
            indirect_target_error(doc, root, key, "forming a circular reference");
            return;
        }
        active.insert(key);
        resolve_indirect_target(doc, root, rk, active);
        active.remove(&key);
    }
    let reft = get(root, rk).cloned().unwrap_or(reft);
    if let Some(uri) = str_attr(&reft, "refuri") {
        let Some(t) = get(root, key) else {
            return;
        };
        t.attributes.set("refuri", uri);
        t.attributes.remove("refid");
    } else if let Some(id) = str_attr(&reft, "refid") {
        let Some(t) = get(root, key) else {
            return;
        };
        t.attributes.set("refid", id);
        doc.note_refid(t);
    } else if !reft.attributes.get_list("ids").is_empty() {
        let Some(t) = get(root, key) else {
            return;
        };
        t.attributes.set("refid", refid.unwrap_or_default());
        doc.note_refid(t);
    } else {
        nonexistent_indirect_target(doc, root, key);
        return;
    }
    let Some(t) = get(root, key) else {
        return;
    };
    if refname.is_some() {
        t.attributes.remove("refname");
    }
    set_resolved(t);
}

fn nonexistent_indirect_target(doc: &mut Document, root: &mut Element, key: Key) {
    let refname = target_or_copy(doc, root, key)
        .and_then(|t| str_attr(&t, "refname"))
        .unwrap_or_default();
    if matches!(doc.names.get(&refname), Some(None)) {
        indirect_target_error(
            doc,
            root,
            key,
            "which is a duplicate, and cannot be used as a unique reference",
        );
    } else {
        indirect_target_error(doc, root, key, "which does not exist");
    }
}

fn indirect_target_error(doc: &mut Document, root: &mut Element, key: Key, explanation: &str) {
    let Some(target) = target_or_copy(doc, root, key) else {
        return;
    };
    let names = target.attributes.get_list("names").to_vec();
    let ids = target.attributes.get_list("ids").to_vec();
    let mut naming = String::new();
    if let Some(n) = names.first() {
        naming = format!("\"{n}\" ");
    }
    let mut reflist: Vec<Key> = Vec::new();
    for name in &names {
        reflist.extend(doc.notes.refnames.get(name).cloned().unwrap_or_default());
    }
    for id in &ids {
        reflist.extend(doc.notes.refids.get(id).cloned().unwrap_or_default());
    }
    if let Some(id) = ids.first() {
        naming.push_str(&format!("(id=\"{id}\")"));
    }
    let mut unique = Vec::new();
    for k in reflist {
        if !unique.contains(&k) {
            unique.push(k);
        }
    }
    let refname = str_attr(&target, "refname").unwrap_or_default();
    report_problem(
        doc,
        root,
        &format!(
            "Indirect hyperlink target {naming}refers to target \"{refname}\", {explanation}."
        ),
        &unique,
        &|_| false,
    );
    if let Some(t) = get(root, key) {
        set_resolved(t);
    }
}

fn resolve_indirect_references(doc: &mut Document, root: &mut Element, key: Key) {
    let Some(target) = get(root, key).cloned() else {
        return;
    };
    let (attname, value, note) = if let Some(id) = str_attr(&target, "refid") {
        ("refid", id, true)
    } else if let Some(uri) = str_attr(&target, "refuri") {
        ("refuri", uri, false)
    } else {
        return;
    };
    let mut lists: Vec<(String, Vec<Key>)> = Vec::new();
    for name in target.attributes.get_list("names") {
        lists.push((
            "refname".into(),
            doc.notes.refnames.get(name).cloned().unwrap_or_default(),
        ));
    }
    for id in target.attributes.get_list("ids") {
        lists.push((
            "refid".into(),
            doc.notes.refids.get(id).cloned().unwrap_or_default(),
        ));
    }
    for (old_attr, reflist) in lists {
        for rk in reflist {
            let Some(r) = get(root, rk) else {
                continue;
            };
            if resolved(r) {
                continue;
            }
            r.attributes.remove(&old_attr);
            r.attributes.set(attname, value.clone());
            if note {
                doc.note_refid(r);
            }
            let Some(r) = get(root, rk) else {
                return;
            };
            set_resolved(r);
            if r.kind == Kind::Target {
                resolve_indirect_references(doc, root, rk);
            }
        }
    }
}

/// References to targets with a URI get the URI (docutils'
/// `references.ExternalTargets`).
fn external_targets(doc: &mut Document, root: &mut Element) {
    let targets = collect(doc, root, &|e| e.kind == Kind::Target && has(e, "refuri"));
    for key in targets {
        let Some(t) = get(root, key).cloned() else {
            continue;
        };
        let uri = str_attr(&t, "refuri").unwrap_or_default();
        for name in t.attributes.get_list("names") {
            for rk in doc.notes.refnames.get(name).cloned().unwrap_or_default() {
                let Some(r) = get(root, rk) else {
                    continue;
                };
                if resolved(r) {
                    continue;
                }
                r.attributes.remove("refname");
                r.attributes.set("refuri", uri.clone());
                set_resolved(r);
            }
        }
    }
}

/// References to internal targets get their ids (docutils'
/// `references.InternalTargets`).
fn internal_targets(doc: &mut Document, root: &mut Element) {
    let targets = collect(doc, root, &|e| {
        e.kind == Kind::Target && !has(e, "refuri") && !has(e, "refid")
    });
    for key in targets {
        let Some(t) = get(root, key).cloned() else {
            continue;
        };
        for name in t.attributes.get_list("names") {
            let refid = doc.nameids.get(name).cloned().flatten();
            for rk in doc.notes.refnames.get(name).cloned().unwrap_or_default() {
                let Some(r) = get(root, rk) else {
                    continue;
                };
                if resolved(r) {
                    continue;
                }
                if let Some(id) = &refid {
                    r.attributes.remove("refname");
                    r.attributes.set("refid", id.clone());
                }
                set_resolved(r);
            }
        }
    }
}

/// References still unresolved: by a name's id, else an error (docutils'
/// `references.DanglingReferences`).
fn dangling_references(doc: &mut Document, root: &mut Element) {
    let refs = collect(doc, root, &|e| {
        matches!(
            e.kind,
            Kind::Reference | Kind::FootnoteReference | Kind::CitationReference
        )
    });
    for key in refs {
        let Some(r) = get(root, key).cloned() else {
            continue;
        };
        if resolved(&r) || !has(&r, "refname") {
            continue;
        }
        let refname = str_attr(&r, "refname").unwrap_or_default();
        match doc.nameids.get(&refname).cloned() {
            Some(Some(id)) if !id.is_empty() => {
                let Some(r) = get(root, key) else {
                    return;
                };
                r.attributes.remove("refname");
                r.attributes.set("refid", id);
                set_resolved(r);
                continue;
            }
            _ => {}
        }
        let duplicate = matches!(doc.nameids.get(&refname), Some(None));
        let mut details = Vec::new();
        let text = if duplicate {
            format!("Duplicate target name, cannot be used as a unique reference: \"{refname}\".")
        } else {
            if refname.contains('<') || refname.contains('>') {
                let mut hint = "Did you want to embed a URI or alias?".to_owned();
                if !refname.contains('<') {
                    hint.push_str("\nOpening bracket missing.");
                } else if !refname.contains(" <") {
                    hint.push_str("\nThe embedded reference must be preceded by whitespace.");
                }
                if !refname.contains('>') {
                    hint.push_str("\nClosing bracket missing.");
                } else if !refname.ends_with('>') {
                    hint.push_str(
                        "\nThe embedded reference must be the last text before the end string.",
                    );
                }
                if refname.contains("< ") || refname.contains(" >") {
                    hint.push_str("\nWhitespace around the embedded reference is not allowed.");
                }
                details.push(Node::from(super::document::paragraph(&hint)));
            }
            format!("Unknown target name: \"{refname}\".")
        };
        let mut msg = doc.message(ERROR, &text, details, At::Exact(None));
        let msgid = doc.set_id(&mut msg, None, None);
        let raw = str_attr(&r, RAW).unwrap_or_default();
        let mut prb = problematic(doc, &raw, &msgid);
        prb.span = r.span;
        match first_id(&r) {
            Some(id) => msg.attributes.push("backrefs", id),
            None => {
                let prbid = doc.set_id(&mut prb, None, None);
                msg.attributes.push("backrefs", prbid);
            }
        }
        replace_self(root, key, vec![prb.into()]);
        doc.transform_messages.push(msg);
    }
}

// -- footnotes ----------------------------------------------------------------

const SYMBOLS: [&str; 10] = [
    "*", "\u{2020}", "\u{2021}", "\u{a7}", "\u{b6}", "#", "\u{2660}", "\u{2665}", "\u{2666}",
    "\u{2663}",
];

/// Numbers auto-numbered footnotes, assigns symbols, and links footnote
/// and citation references (docutils' `references.Footnotes`).
fn footnotes(doc: &mut Document, root: &mut Element) {
    let labels = number_footnotes(doc, root);
    number_footnote_references(doc, root, &labels);
    symbolize_footnotes(doc, root);
    let lists = [
        (doc.notes.footnotes.clone(), doc.notes.footnote_refs.clone()),
        (doc.notes.citations.clone(), doc.notes.citation_refs.clone()),
    ];
    for (notes, refs) in lists {
        for nk in notes {
            let Some((id, names)) = with_node(doc, root, nk, |_, n| {
                (
                    first_id(n).unwrap_or_default(),
                    n.attributes.get_list("names").to_vec(),
                )
            }) else {
                continue;
            };
            for label in names {
                for &rk in refs.get(&label).map(Vec::as_slice).unwrap_or(&[]) {
                    let rid = with_node(doc, root, rk, |_, r| {
                        if resolved(r) {
                            return None;
                        }
                        r.attributes.remove("refname");
                        r.attributes.set("refid", id.clone());
                        set_resolved(r);
                        Some(first_id(r).unwrap_or_default())
                    });
                    if let Some(Some(rid)) = rid {
                        with_node(doc, root, nk, |_, n| n.attributes.push("backrefs", rid));
                    }
                }
            }
        }
    }
}

/// Labels auto-numbered footnotes and the references to labelled ones;
/// returns the labels of the unlabelled ones.
fn number_footnotes(doc: &mut Document, root: &mut Element) -> Vec<String> {
    let mut labels = Vec::new();
    let mut number = 1;
    for fkey in doc.notes.autofootnotes.clone() {
        let label = loop {
            let label = number.to_string();
            number += 1;
            if !doc.nameids.contains_key(&label) {
                break label;
            }
        };
        let Some((names, fid)) = with_node(doc, root, fkey, |_, f| {
            f.children.insert(
                0,
                Element::new(Kind::Label)
                    .with_child(Text::new(label.clone(), None))
                    .into(),
            );
            (
                f.attributes.get_list("names").to_vec(),
                first_id(f).unwrap_or_default(),
            )
        }) else {
            continue;
        };
        for name in &names {
            let refs = doc
                .notes
                .footnote_refs
                .get(name)
                .cloned()
                .unwrap_or_default();
            for rk in refs {
                let rid = with_node(doc, root, rk, |doc, r| {
                    r.children.push(Text::new(label.clone(), None).into());
                    r.attributes.remove("refname");
                    r.attributes.set("refid", fid.clone());
                    let rid = first_id(r).unwrap_or_default();
                    doc.note_refid(r);
                    set_resolved(r);
                    rid
                });
                if let Some(rid) = rid {
                    with_node(doc, root, fkey, |_, f| f.attributes.push("backrefs", rid));
                }
            }
        }
        let unlabelled = with_node(doc, root, fkey, |doc, f| {
            let unnamed = f.attributes.get_list("names").is_empty()
                && f.attributes.get_list("dupnames").is_empty();
            if unnamed {
                f.attributes.push("names", label.clone());
                doc.note_explicit_target(f, Some(MsgNode::Itself), None);
            }
            unnamed
        });
        if unlabelled == Some(true) {
            labels.push(label);
        }
    }
    labels
}

/// Labels the references to unlabelled auto-numbered footnotes, in order.
fn number_footnote_references(doc: &mut Document, root: &mut Element, labels: &[String]) {
    let refs = doc.notes.autofootnote_refs.clone();
    let mut i = 0;
    for &rk in &refs {
        let Some(done) = with_node(doc, root, rk, |_, r| resolved(r) || has(r, "refid")) else {
            continue;
        };
        if done {
            continue;
        }
        let Some(label) = labels.get(i).cloned() else {
            let count = labels.len();
            let s = if count > 1 { "s" } else { "" };
            // docutils restarts at the number of labels used.
            report_problem(
                doc,
                root,
                &format!(
                    "Too many autonumbered footnote references: only {count} corresponding footnote{s} available."
                ),
                &refs[i..],
                &|e| resolved(e) || has(e, "refname"),
            );
            break;
        };
        let id = doc
            .nameids
            .get(&label)
            .cloned()
            .flatten()
            .unwrap_or_default();
        let rid = with_node(doc, root, rk, |doc, r| {
            r.children.push(Text::new(label, None).into());
            r.attributes.set("refid", id.clone());
            let rid = first_id(r).unwrap_or_default();
            doc.note_refid(r);
            set_resolved(r);
            rid
        });
        if let (Some(rid), Some(fk)) = (rid, doc.ids.get(&id).copied()) {
            with_node(doc, root, fk, |_, f| f.attributes.push("backrefs", rid));
        }
        i += 1;
    }
}

/// Labels symbol footnotes and their references, in order.
fn symbolize_footnotes(doc: &mut Document, root: &mut Element) {
    let footnotes = doc.notes.symbol_footnotes.clone();
    let mut labels = Vec::new();
    for (start, &fkey) in footnotes.iter().enumerate() {
        let (reps, index) = (start / SYMBOLS.len(), start % SYMBOLS.len());
        let label = SYMBOLS[index].repeat(reps + 1);
        labels.push(label.clone());
        with_node(doc, root, fkey, |doc, f| {
            f.children.insert(
                0,
                Element::new(Kind::Label)
                    .with_child(Text::new(label, None))
                    .into(),
            );
            doc.set_id(f, None, None);
        });
    }
    let refs = doc.notes.symbol_footnote_refs.clone();
    for (i, &rk) in refs.iter().enumerate() {
        let Some(label) = labels.get(i).cloned() else {
            report_problem(
                doc,
                root,
                &format!(
                    "Too many symbol footnote references: only {} corresponding footnotes available.",
                    labels.len()
                ),
                &refs[i..],
                &|e| resolved(e) || has(e, "refid"),
            );
            break;
        };
        let fid = with_node(doc, root, footnotes[i], |_, f| first_id(f))
            .flatten()
            .unwrap_or_default();
        let rid = with_node(doc, root, rk, |doc, r| {
            r.children.push(Text::new(label, None).into());
            r.attributes.set("refid", fid);
            let rid = first_id(r).unwrap_or_default();
            doc.note_refid(r);
            rid
        });
        if let Some(rid) = rid {
            with_node(doc, root, footnotes[i], |_, f| {
                f.attributes.push("backrefs", rid)
            });
        }
    }
}
