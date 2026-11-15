//! Pseudo-XML: docutils' indented text form of a document tree, as printed
//! by `docutils --writer=pseudoxml` or `Node.pformat()`.
//!
//! Each element is a line `<tag name="value" ...>` with attributes sorted
//! by name, followed by its children indented four spaces; text is printed
//! line by line at its parent's indentation plus four. Empty basic list
//! attributes (`ids`, `classes`, `names`, `dupnames`) and an empty
//! `backrefs` are left out. List
//! values are joined with spaces, `\` and spaces in items escaped with `\`;
//! flags print as `1` or `0`. Nothing is XML-escaped.

use crate::nodes::{AttrValue, BASIC_ATTRIBUTES, Element, Node, Text};

const INDENT: &str = "    ";

impl Element {
    /// The element as pseudo-XML, exactly as docutils prints it.
    pub fn pformat(&self) -> String {
        let mut out = String::new();
        element(self, 0, &mut out);
        out
    }

    /// The element's start tag, e.g. `<section ids="intro" names="intro">`.
    pub fn starttag(&self) -> String {
        let mut tag = format!("<{}", self.kind.tag());
        for (name, value) in self.attributes.iter() {
            let value = match value {
                AttrValue::List(list)
                    if list.is_empty()
                        && (BASIC_ATTRIBUTES.contains(&name) || name == "backrefs") =>
                {
                    continue;
                }
                AttrValue::List(list) => list
                    .iter()
                    .map(|v| serial_escape(v))
                    .collect::<Vec<_>>()
                    .join(" "),
                AttrValue::Str(s) => s.clone(),
                AttrValue::Int(n) => n.to_string(),
                AttrValue::Bool(b) => u8::from(*b).to_string(),
            };
            tag.push_str(&format!(" {name}=\"{value}\""));
        }
        tag.push('>');
        tag
    }
}

impl Text {
    /// The text as pseudo-XML at indentation level 0.
    pub fn pformat(&self) -> String {
        let mut out = String::new();
        text(self, 0, &mut out);
        out
    }
}

impl Node {
    /// The node as pseudo-XML.
    pub fn pformat(&self) -> String {
        match self {
            Node::Element(e) => e.pformat(),
            Node::Text(t) => t.pformat(),
        }
    }
}

/// Escapes an item of a list attribute value.
fn serial_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace(' ', "\\ ")
}

fn element(e: &Element, level: usize, out: &mut String) {
    out.push_str(&INDENT.repeat(level));
    out.push_str(&e.starttag());
    out.push('\n');
    for child in &e.children {
        match child {
            Node::Element(child) => element(child, level + 1, out),
            Node::Text(t) => text(t, level + 1, out),
        }
    }
}

fn text(t: &Text, level: usize, out: &mut String) {
    let indent = INDENT.repeat(level);
    for line in crate::lines::split_lines(&t.value) {
        out.push_str(&indent);
        out.push_str(line);
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use crate::nodes::{Element, Kind, Text};

    #[test]
    fn prints_like_docutils() {
        let mut target = Element::new(Kind::Target);
        target.attributes.push("names", "a \"b\" & c");
        target.attributes.push("ids", "a-b-c");
        target
            .attributes
            .set("refuri", "https://e.org/?a=1&b=\"2\"<3>");
        let mut doc = Element::new(Kind::Document)
            .with_attr("source", "test data")
            .with_child(target)
            .with_child(Element::new(Kind::Paragraph).with_child(Text::new("Hello \nworld", None)))
            .with_child(Element::new(Kind::LiteralBlock).with_attr("xml:space", "preserve"));
        doc.attributes.set("classes", Vec::<String>::new());
        assert_eq!(
            doc.pformat(),
            "<document source=\"test data\">\n\
             \x20   <target ids=\"a-b-c\" names=\"a\\ \"b\"\\ &\\ c\" refuri=\"https://e.org/?a=1&b=\"2\"<3>\">\n\
             \x20   <paragraph>\n\
             \x20       Hello \n\
             \x20       world\n\
             \x20   <literal_block xml:space=\"preserve\">\n"
        );
    }

    #[test]
    fn flags_and_numbers() {
        let e = Element::new(Kind::Footnote)
            .with_attr("auto", 1i64)
            .with_attr("anonymous", true);
        assert_eq!(e.starttag(), "<footnote anonymous=\"1\" auto=\"1\">");
    }
}
