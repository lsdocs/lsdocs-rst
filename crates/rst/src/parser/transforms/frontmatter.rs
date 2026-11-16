//! docutils' front matter transforms (`docutils.transforms.frontmatter`):
//! the document title and subtitle, and bibliographic fields.

use super::{has, key_of};
use crate::nodes::{AttrValue, Element, Kind, Node, Text};
use crate::parser::document::{At, Document, Key, LINE, RAW, WARNING};
use crate::parser::inline::astext;
use crate::util::{make_id, normalize_name, strip};

/// docutils' `PreBibliographic` elements: those that may come before the
/// bibliographic fields.
fn pre_bibliographic(n: &Node) -> bool {
    matches!(n, Node::Element(e) if matches!(
        e.kind,
        Kind::Comment
            | Kind::Pending
            | Kind::SubstitutionDefinition
            | Kind::Target
            | Kind::SystemMessage
            | Kind::Raw
            | Kind::Title
            | Kind::Subtitle
            | Kind::Meta
            | Kind::Decoration
    ))
}

/// The index of the section that may become the title: the only element
/// after the pre-bibliographic ones (docutils' `candidate_index`).
fn candidate_index(node: &Element) -> Option<usize> {
    let index = node.children.iter().position(|n| !pre_bibliographic(n))?;
    let is_section = matches!(&node.children[index], Node::Element(e) if e.kind == Kind::Section);
    (node.children.len() == index + 1 && is_section).then_some(index)
}

/// Copies `src`'s attributes onto `dst`, appending to list attributes
/// (docutils' `update_all_atts_concatenating`).
fn concatenate_attributes(dst: &mut Element, src: &Element) {
    for (name, value) in src.attributes.iter() {
        if name.starts_with('\0') {
            continue;
        }
        if matches!(name, "ids" | "classes" | "names" | "dupnames") {
            for v in src.attributes.get_list(name) {
                if !dst.attributes.get_list(name).contains(v) {
                    dst.attributes.push(name, v.clone());
                }
            }
        } else {
            dst.attributes.set(name.to_owned(), value.clone());
        }
    }
}

/// Points the names and ids that led to `old` at `new`.
fn rekey(doc: &mut Document, old: Option<Key>, new: Key) {
    let Some(old) = old else {
        return;
    };
    for k in doc.ids.values_mut() {
        if *k == old {
            *k = new;
        }
    }
    for k in doc.names.values_mut() {
        if *k == Some(old) {
            *k = Some(new);
        }
    }
}

fn into_element(n: Node) -> Element {
    match n {
        Node::Element(e) => e,
        Node::Text(_) => unreachable!("candidates are elements"),
    }
}

/// A lone top-level section becomes the document title, and a lone
/// subsection then the subtitle (docutils' `DocTitle`).
pub(super) fn doc_title(doc: &mut Document, root: &mut Element) {
    if doc.extensions.doctitle && promote_title(doc, root) {
        promote_subtitle(doc, root);
    }
    if !has(root, "title")
        && let Some(Node::Element(title)) = root.children.first()
        && title.kind == Kind::Title
    {
        let text = astext(title);
        root.attributes.set("title", text);
    }
}

fn promote_title(doc: &mut Document, node: &mut Element) -> bool {
    let Some(index) = candidate_index(node) else {
        return false;
    };
    let section = into_element(node.children.remove(index));
    concatenate_attributes(node, &section);
    let key = doc.key(node);
    rekey(doc, key_of(&section), key);
    let mut rest = section.children.into_iter();
    let before = std::mem::take(&mut node.children);
    node.children.extend(rest.next());
    node.children.extend(before);
    node.children.extend(rest);
    true
}

fn promote_subtitle(doc: &mut Document, node: &mut Element) -> bool {
    let Some(index) = candidate_index(node) else {
        return false;
    };
    let subsection = into_element(node.children.remove(index));
    let mut subtitle = Element::new(Kind::Subtitle);
    concatenate_attributes(&mut subtitle, &subsection);
    let key = doc.key(&mut subtitle);
    rekey(doc, key_of(&subsection), key);
    let mut rest = subsection.children.into_iter();
    if let Some(Node::Element(title)) = rest.next() {
        subtitle.children = title.children;
    }
    node.children.insert(1, subtitle.into());
    node.children.extend(rest);
    true
}

// -- docinfo ------------------------------------------------------------------

/// The bibliographic field names (English).
const BIBLIOGRAPHIC: [&str; 12] = [
    "author",
    "authors",
    "organization",
    "address",
    "contact",
    "version",
    "revision",
    "status",
    "date",
    "copyright",
    "dedication",
    "abstract",
];

/// A field list first in the document becomes the docinfo, with the
/// dedication and abstract topics (docutils' `DocInfo`).
pub(super) fn doc_info(doc: &mut Document, root: &mut Element) {
    let Some(index) = root.children.iter().position(|n| !pre_bibliographic(n)) else {
        return;
    };
    if !matches!(&root.children[index], Node::Element(e) if e.kind == Kind::FieldList) {
        return;
    }
    let biblioindex = root
        .children
        .iter()
        .position(|n| {
            !matches!(n, Node::Element(e) if matches!(
                e.kind,
                Kind::Title | Kind::Subtitle | Kind::Rubric | Kind::Decoration | Kind::Meta
            ))
        })
        .unwrap_or(root.children.len());
    let field_list = into_element(root.children.remove(index));
    let nodes = extract_bibliographic(doc, field_list);
    root.children.splice(biblioindex..biblioindex, nodes);
}

fn extract_bibliographic(doc: &mut Document, field_list: Element) -> Vec<Node> {
    let mut docinfo = Element::new(Kind::Docinfo);
    let mut topics: [Option<Element>; 2] = [None, None];
    for child in field_list.children {
        let Node::Element(mut field) = child else {
            continue;
        };
        let name = match field.children.first() {
            Some(Node::Element(n)) => match n.children.first() {
                Some(Node::Text(t)) => crate::util::unescape(&t.value),
                Some(Node::Element(e)) => astext(e),
                None => String::new(),
            },
            _ => String::new(),
        };
        let normed = normalize_name(&name);
        if biblio_field(doc, &mut field, &name, &normed, &mut docinfo, &mut topics).is_ok() {
            continue;
        }
        if let Some(body) = last_element(&mut field)
            && body.children.len() == 1
            && let Some(Node::Element(p)) = body.children.first_mut()
            && p.kind == Kind::Paragraph
        {
            clean_rcs_keywords(p);
        }
        let class = make_id(&normed);
        if !class.is_empty() {
            field.attributes.push("classes", class);
        }
        docinfo.children.push(field.into());
    }
    let mut nodes = Vec::new();
    if !docinfo.children.is_empty() {
        nodes.push(docinfo.into());
    }
    nodes.extend(topics.into_iter().flatten().map(Node::from));
    nodes
}

fn last_element(e: &mut Element) -> Option<&mut Element> {
    match e.children.last_mut() {
        Some(Node::Element(c)) => Some(c),
        _ => None,
    }
}

/// Adds a warning about `field` to its body.
fn field_warning(doc: &Document, field: &mut Element, text: &str) {
    let line = match field.attributes.get(LINE) {
        Some(AttrValue::Int(n)) => Some(*n as usize),
        _ => None,
    };
    let msg = doc.message(WARNING, text, Vec::new(), At::Exact(line));
    if let Some(body) = last_element(field) {
        body.children.push(msg.into());
    }
}

/// Moves a bibliographic field into the docinfo or a topic; `Err` leaves
/// it a plain field.
fn biblio_field(
    doc: &mut Document,
    field: &mut Element,
    name: &str,
    normed: &str,
    docinfo: &mut Element,
    topics: &mut [Option<Element>; 2],
) -> Result<(), ()> {
    if !(field.children.len() == 2
        && BIBLIOGRAPHIC.contains(&normed)
        && check_empty_biblio_field(doc, field, name))
    {
        return Err(());
    }
    let topic = match normed {
        "authors" => return extract_authors(doc, field, name, docinfo),
        "dedication" => 0,
        "abstract" => 1,
        _ => {
            if !check_compound_biblio_field(doc, field, name) {
                return Err(());
            }
            let Some(Node::Element(mut p)) = last_element(field).and_then(|b| b.children.pop())
            else {
                return Err(());
            };
            clean_rcs_keywords(&mut p);
            let kind = Kind::from_tag(normed);
            let mut e = Element::new(kind.clone()).with_children(p.children);
            if kind == Kind::Address {
                e.attributes.set("xml:space", "preserve");
            }
            docinfo.children.push(e.into());
            return Ok(());
        }
    };
    if topics[topic].is_some() {
        field_warning(
            doc,
            field,
            &format!("There can only be one \"{name}\" field."),
        );
        return Err(());
    }
    let label = if topic == 0 { "Dedication" } else { "Abstract" };
    let title = Element::new(Kind::Title).with_child(Text::new(label, None));
    let body = last_element(field).map(|b| std::mem::take(&mut b.children));
    let mut e = Element::new(Kind::Topic).with_attr("classes", vec![normed.to_owned()]);
    e.children.push(title.into());
    e.children.extend(body.unwrap_or_default());
    topics[topic] = Some(e);
    Ok(())
}

fn check_empty_biblio_field(doc: &Document, field: &mut Element, name: &str) -> bool {
    if last_element(field).is_some_and(|b| b.children.is_empty()) {
        let text = format!("Cannot extract empty bibliographic field \"{name}\".");
        field_warning(doc, field, &text);
        return false;
    }
    true
}

fn check_compound_biblio_field(doc: &Document, field: &mut Element, name: &str) -> bool {
    let Some(body) = last_element(field) else {
        return false;
    };
    let is = |n: &Node, kind: Kind| matches!(n, Node::Element(e) if e.kind == kind);
    if body.children.len() == 1 && is(&body.children[0], Kind::Paragraph) {
        return true;
    }
    // A one-line body taken for an enumerated list ("A. Name") is
    // reparsed as text.
    let raw = body.attributes.get_str(RAW).unwrap_or_default().to_owned();
    if is(&body.children[0], Kind::EnumeratedList) && !strip(&raw).contains('\n') {
        let reparsed = crate::parse(&format!("\\{raw}"));
        if reparsed.children.len() == 1 && is(&reparsed.children[0], Kind::Paragraph) {
            body.children = reparsed.children;
            return true;
        }
    }
    let content: Vec<String> = body
        .children
        .iter()
        .filter_map(|n| match n {
            Node::Element(e) if e.kind != Kind::SystemMessage => {
                Some(format!("<{}>", e.kind.tag()))
            }
            _ => None,
        })
        .collect();
    let content = if content.len() > 1 {
        format!("[{}]", content.join(", "))
    } else {
        format!(
            "a {}",
            content.first().map(String::as_str).unwrap_or_default()
        )
    };
    let text = format!(
        "Bibliographic field \"{name}\"\nmust contain a single <paragraph>, not {content}."
    );
    field_warning(doc, field, &text);
    false
}

fn extract_authors(
    doc: &Document,
    field: &mut Element,
    name: &str,
    docinfo: &mut Element,
) -> Result<(), ()> {
    let authors = match &field.children[1] {
        Node::Element(body) => {
            if body.children.len() == 1 {
                match &body.children[0] {
                    Node::Element(e) if e.kind == Kind::Paragraph => {
                        authors_from_one_paragraph(body)
                    }
                    Node::Element(e) if e.kind == Kind::BulletList => authors_from_bullet_list(e),
                    _ => None,
                }
            } else {
                authors_from_paragraphs(body)
            }
        }
        Node::Text(_) => None,
    };
    let authors: Vec<Node> = authors
        .unwrap_or_default()
        .into_iter()
        .filter(|a| !a.is_empty())
        .map(|a| Element::new(Kind::Author).with_children(a).into())
        .collect();
    if !authors.is_empty() {
        docinfo
            .children
            .push(Element::new(Kind::Authors).with_children(authors).into());
        return Ok(());
    }
    let text = format!(
        "Cannot extract \"{name}\" from bibliographic field:\nBibliographic field \"{name}\" must contain either\n a single paragraph (with author names separated by a character from the set \";,\"),\n multiple paragraphs (one per author),\n or a bullet list with one author name per item.\nNote: Leading initials can cause (mis)recognizing names as enumerated list."
    );
    field_warning(doc, field, &text);
    Err(())
}

fn all_text(e: &Element, out: &mut String) {
    for child in &e.children {
        match child {
            Node::Text(t) => out.push_str(&t.value),
            Node::Element(c) => all_text(c, out),
        }
    }
}

fn authors_from_one_paragraph(body: &Element) -> Option<Vec<Vec<Node>>> {
    let mut text = String::new();
    all_text(body, &mut text);
    if text.is_empty() {
        return None;
    }
    let mut names = Vec::new();
    for sep in [';', ','] {
        names = split_unescaped(&text, sep);
        if names.len() > 1 {
            break;
        }
    }
    Some(
        names
            .iter()
            .map(|n| strip(n))
            .filter(|n| !n.is_empty())
            .map(|n| vec![Text::new(n, None).into()])
            .collect(),
    )
}

/// `text` split at `sep`, except where it is escaped.
fn split_unescaped(text: &str, sep: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut part = String::new();
    let mut prev = None;
    for c in text.chars() {
        if c == sep && prev != Some('\0') {
            parts.push(std::mem::take(&mut part));
        } else {
            part.push(c);
        }
        prev = Some(c);
    }
    parts.push(part);
    parts
}

fn authors_from_bullet_list(list: &Element) -> Option<Vec<Vec<Node>>> {
    let mut authors = Vec::new();
    for item in &list.children {
        let Node::Element(item) = item else {
            return None;
        };
        if item.kind == Kind::Comment {
            continue;
        }
        match item.children.as_slice() {
            [Node::Element(p)] if p.kind == Kind::Paragraph => authors.push(p.children.clone()),
            _ => return None,
        }
    }
    (!authors.is_empty()).then_some(authors)
}

fn authors_from_paragraphs(body: &Element) -> Option<Vec<Vec<Node>>> {
    let mut authors = Vec::new();
    for item in &body.children {
        match item {
            Node::Element(e) if e.kind == Kind::Paragraph => authors.push(e.children.clone()),
            Node::Element(e) if e.kind == Kind::Comment => {}
            _ => return None,
        }
    }
    Some(authors)
}

// -- RCS keywords -------------------------------------------------------------

/// Replaces RCS keywords such as `$Date: ... $` in a paragraph of plain
/// text by their values (docutils' `clean_rcs_keywords`).
fn clean_rcs_keywords(p: &mut Element) {
    let [Node::Text(t)] = p.children.as_mut_slice() else {
        return;
    };
    let chars: Vec<char> = t.value.chars().collect();
    for pattern in [rcs_date as Matcher, rcs_file, rcs_keyword] {
        if let Some(text) = sub_all(&chars, pattern) {
            t.value = text;
            return;
        }
    }
}

/// Matches a pattern at a position: the end and the replacement.
type Matcher = fn(&[char], usize) -> Option<(usize, String)>;

/// Replaces every match of `pattern`; `None` if there is none.
fn sub_all(chars: &[char], pattern: Matcher) -> Option<String> {
    let mut out = String::new();
    let mut found = false;
    let mut i = 0;
    while i < chars.len() {
        if let Some((end, replacement)) = pattern(chars, i) {
            out.push_str(&replacement);
            found = true;
            i = end;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    found.then_some(out)
}

/// Whether `chars[at..]` starts with `prefix`, ignoring ASCII case.
fn starts_with_ci(chars: &[char], at: usize, prefix: &str) -> bool {
    let n = prefix.chars().count();
    at + n <= chars.len()
        && chars[at..at + n]
            .iter()
            .zip(prefix.chars())
            .all(|(a, b)| a.eq_ignore_ascii_case(&b))
}

fn is_digit(c: char) -> bool {
    c.is_ascii_digit()
}

/// `\$Date: (\d\d\d\d)[-/](\d\d)[-/](\d\d)[ T][\d:]+[^$]* \$`, ignoring
/// case, to `\1-\2-\3`.
fn rcs_date(chars: &[char], at: usize) -> Option<(usize, String)> {
    if !starts_with_ci(chars, at, "$date: ") {
        return None;
    }
    let mut i = at + 7;
    let mut groups = Vec::new();
    for (n, sep) in [(4, true), (2, true), (2, false)] {
        let digits = chars.get(i..i + n)?;
        if !digits.iter().all(|&c| is_digit(c)) {
            return None;
        }
        groups.push(digits.iter().collect::<String>());
        i += n;
        let c = *chars.get(i)?;
        let ok = if sep {
            c == '-' || c == '/'
        } else {
            c == ' ' || c == 'T' || c == 't'
        };
        if !ok {
            return None;
        }
        i += 1;
    }
    let start = i;
    while chars.get(i).is_some_and(|&c| is_digit(c) || c == ':') {
        i += 1;
    }
    if i == start {
        return None;
    }
    // `[^$]* \$`: up to the next `$`, which must follow a space.
    let dollar = i + chars[i..].iter().position(|&c| c == '$')?;
    if dollar == i || chars[dollar - 1] != ' ' {
        return None;
    }
    Some((dollar + 1, groups.join("-")))
}

/// `\$RCSfile: (.+),v \$`, ignoring case, to `\1`.
fn rcs_file(chars: &[char], at: usize) -> Option<(usize, String)> {
    if !starts_with_ci(chars, at, "$rcsfile: ") {
        return None;
    }
    greedy_to(chars, at + 10, ",v $", true)
}

/// `\$[a-zA-Z]+: (.+) \$` to `\1`.
fn rcs_keyword(chars: &[char], at: usize) -> Option<(usize, String)> {
    if chars[at] != '$' {
        return None;
    }
    let mut i = at + 1;
    while chars.get(i).is_some_and(|c| c.is_ascii_alphabetic()) {
        i += 1;
    }
    if i == at + 1 || chars.get(i) != Some(&':') || chars.get(i + 1) != Some(&' ') {
        return None;
    }
    greedy_to(chars, i + 2, " $", false)
}

/// `(.+)` then `end`, greedy within the line: the group and where the
/// match ends.
fn greedy_to(chars: &[char], start: usize, end: &str, ci: bool) -> Option<(usize, String)> {
    let line_end = start
        + chars[start..]
            .iter()
            .position(|&c| c == '\n')
            .unwrap_or(chars.len() - start);
    let n = end.chars().count();
    let mut j = line_end.checked_sub(n)?;
    loop {
        if j > start {
            let matches = if ci {
                starts_with_ci(chars, j, end)
            } else {
                chars[j..j + n].iter().copied().eq(end.chars())
            };
            if matches {
                return Some((j + n, chars[start..j].iter().collect()));
            }
        }
        if j <= start + 1 {
            return None;
        }
        j -= 1;
    }
}
