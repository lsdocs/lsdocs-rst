//! The document tree: elements and text, mirroring docutils' doctree.
//!
//! As in docutils, every element is the same shape (a kind, attributes and
//! children) rather than a struct per kind, so the tree can hold any node a
//! directive produces and prints exactly like docutils' pseudo-XML. Element
//! kinds are the docutils node class names; [`Kind::Other`] holds kinds
//! this crate does not define, such as those added by extensions.

use std::collections::BTreeMap;
use std::fmt;

/// A byte range of the parsed text: `start` inclusive, `end` exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    /// Offset of the first byte.
    pub start: usize,
    /// Offset one past the last byte.
    pub end: usize,
}

impl Span {
    /// The span from `start` to `end`.
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// The smallest span covering both.
    pub fn to(self, other: Span) -> Span {
        Span::new(self.start.min(other.start), self.end.max(other.end))
    }
}

/// Defines [`Kind`] with each variant's docutils tag name.
macro_rules! kinds {
    ($( $(#[$doc:meta])* $variant:ident = $tag:literal, )*) => {
        /// The kind of an [`Element`]: a docutils node class.
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        #[non_exhaustive]
        pub enum Kind {
            $( $(#[$doc])* $variant, )*
            /// A kind this crate does not define, by tag name.
            Other(String),
        }

        impl Kind {
            /// The docutils tag name, e.g. `"bullet_list"`.
            pub fn tag(&self) -> &str {
                match self {
                    $( Kind::$variant => $tag, )*
                    Kind::Other(tag) => tag,
                }
            }

            /// The kind with the docutils tag name `tag`.
            pub fn from_tag(tag: &str) -> Kind {
                match tag {
                    $( $tag => Kind::$variant, )*
                    other => Kind::Other(other.to_owned()),
                }
            }
        }
    };
}

kinds! {
    /// The root.
    Document = "document",
    /// A section: a title and its content.
    Section = "section",
    /// A section or document title.
    Title = "title",
    /// A document subtitle.
    Subtitle = "subtitle",
    /// Bibliographic fields.
    Docinfo = "docinfo",
    /// A bibliographic field.
    Author = "author",
    /// A bibliographic field.
    Authors = "authors",
    /// A bibliographic field.
    Organization = "organization",
    /// A bibliographic field.
    Address = "address",
    /// A bibliographic field.
    Contact = "contact",
    /// A bibliographic field.
    Version = "version",
    /// A bibliographic field.
    Revision = "revision",
    /// A bibliographic field.
    Status = "status",
    /// A bibliographic field.
    Date = "date",
    /// A bibliographic field.
    Copyright = "copyright",
    /// Page decoration: header and footer.
    Decoration = "decoration",
    /// A page header.
    Header = "header",
    /// A page footer.
    Footer = "footer",
    /// A horizontal rule.
    Transition = "transition",
    /// A paragraph.
    Paragraph = "paragraph",
    /// Several body elements forming one paragraph.
    Compound = "compound",
    /// A generic container.
    Container = "container",
    /// A bulleted list.
    BulletList = "bullet_list",
    /// A numbered list.
    EnumeratedList = "enumerated_list",
    /// An item of a bullet or enumerated list.
    ListItem = "list_item",
    /// A list of terms and definitions.
    DefinitionList = "definition_list",
    /// A term and its definition.
    DefinitionListItem = "definition_list_item",
    /// A defined term.
    Term = "term",
    /// A classifier of a term.
    Classifier = "classifier",
    /// A definition.
    Definition = "definition",
    /// A list of fields.
    FieldList = "field_list",
    /// A field: a name and a body.
    Field = "field",
    /// A field's name.
    FieldName = "field_name",
    /// A field's body.
    FieldBody = "field_body",
    /// A list of command-line options.
    OptionList = "option_list",
    /// An option list entry.
    OptionListItem = "option_list_item",
    /// Synonymous options.
    OptionGroup = "option_group",
    /// One option.
    Option = "option",
    /// An option's name.
    OptionString = "option_string",
    /// An option's argument.
    OptionArgument = "option_argument",
    /// An option's description.
    Description = "description",
    /// Preformatted text.
    LiteralBlock = "literal_block",
    /// An interactive Python session.
    DoctestBlock = "doctest_block",
    /// A display formula.
    MathBlock = "math_block",
    /// Lines whose breaks are kept.
    LineBlock = "line_block",
    /// A line of a line block.
    Line = "line",
    /// An indented block.
    BlockQuote = "block_quote",
    /// A block quote's attribution.
    Attribution = "attribution",
    /// An admonition.
    Attention = "attention",
    /// An admonition.
    Caution = "caution",
    /// An admonition.
    Danger = "danger",
    /// An admonition.
    Error = "error",
    /// An admonition.
    Important = "important",
    /// An admonition.
    Note = "note",
    /// An admonition.
    Tip = "tip",
    /// An admonition.
    Hint = "hint",
    /// An admonition.
    Warning = "warning",
    /// A generic admonition with its own title.
    Admonition = "admonition",
    /// A comment.
    Comment = "comment",
    /// A substitution definition.
    SubstitutionDefinition = "substitution_definition",
    /// A hyperlink target.
    Target = "target",
    /// A footnote.
    Footnote = "footnote",
    /// A citation.
    Citation = "citation",
    /// A footnote or citation label.
    Label = "label",
    /// A figure.
    Figure = "figure",
    /// A figure caption.
    Caption = "caption",
    /// A figure legend.
    Legend = "legend",
    /// A table.
    Table = "table",
    /// A group of table columns.
    Tgroup = "tgroup",
    /// A column specification.
    Colspec = "colspec",
    /// Table head rows.
    Thead = "thead",
    /// Table body rows.
    Tbody = "tbody",
    /// A table row.
    Row = "row",
    /// A table cell.
    Entry = "entry",
    /// A problem report.
    SystemMessage = "system_message",
    /// A placeholder for a later transform.
    Pending = "pending",
    /// Output-format-specific content.
    Raw = "raw",
    /// Emphasized text.
    Emphasis = "emphasis",
    /// Strongly emphasized text.
    Strong = "strong",
    /// Inline literal text.
    Literal = "literal",
    /// A hyperlink reference.
    Reference = "reference",
    /// A footnote reference.
    FootnoteReference = "footnote_reference",
    /// A citation reference.
    CitationReference = "citation_reference",
    /// A substitution reference.
    SubstitutionReference = "substitution_reference",
    /// The title of a work.
    TitleReference = "title_reference",
    /// An abbreviation.
    Abbreviation = "abbreviation",
    /// An acronym.
    Acronym = "acronym",
    /// Superscript text.
    Superscript = "superscript",
    /// Subscript text.
    Subscript = "subscript",
    /// An inline formula.
    Math = "math",
    /// An image.
    Image = "image",
    /// Generic inline text.
    Inline = "inline",
    /// Text markup could not be parsed.
    Problematic = "problematic",
    /// Text generated by a transform.
    Generated = "generated",
    /// A titled block outside the section flow.
    Topic = "topic",
    /// A sidebar.
    Sidebar = "sidebar",
    /// An informal heading.
    Rubric = "rubric",
    /// HTML metadata.
    Meta = "meta",
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.tag())
    }
}

/// An attribute value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttrValue {
    /// Text.
    Str(String),
    /// A list of strings, such as `ids` or `names`.
    List(Vec<String>),
    /// A number.
    Int(i64),
    /// A flag.
    Bool(bool),
    /// No value (Python's `None`); only in `pending` details.
    None,
}

impl From<&str> for AttrValue {
    fn from(s: &str) -> Self {
        AttrValue::Str(s.to_owned())
    }
}

impl From<String> for AttrValue {
    fn from(s: String) -> Self {
        AttrValue::Str(s)
    }
}

impl From<i64> for AttrValue {
    fn from(n: i64) -> Self {
        AttrValue::Int(n)
    }
}

impl From<bool> for AttrValue {
    fn from(b: bool) -> Self {
        AttrValue::Bool(b)
    }
}

impl From<Vec<String>> for AttrValue {
    fn from(list: Vec<String>) -> Self {
        AttrValue::List(list)
    }
}

/// The list attributes every element has (empty unless set), and which are
/// left out of printed output when empty.
pub const BASIC_ATTRIBUTES: [&str; 4] = ["ids", "classes", "names", "dupnames"];

/// An element's attributes, kept sorted by name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Attributes(BTreeMap<String, AttrValue>);

impl Attributes {
    /// The value of `name`, if set.
    pub fn get(&self, name: &str) -> Option<&AttrValue> {
        self.0.get(name)
    }

    /// The text value of `name`, if set and text.
    pub fn get_str(&self, name: &str) -> Option<&str> {
        match self.0.get(name) {
            Some(AttrValue::Str(s)) => Some(s),
            _ => None,
        }
    }

    /// The list value of `name`; empty if unset.
    pub fn get_list(&self, name: &str) -> &[String] {
        match self.0.get(name) {
            Some(AttrValue::List(list)) => list,
            _ => &[],
        }
    }

    /// Sets `name` to `value`.
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<AttrValue>) {
        self.0.insert(name.into(), value.into());
    }

    /// Appends `value` to the list attribute `name`.
    pub fn push(&mut self, name: &str, value: impl Into<String>) {
        match self
            .0
            .entry(name.to_owned())
            .or_insert_with(|| AttrValue::List(Vec::new()))
        {
            AttrValue::List(list) => list.push(value.into()),
            other => *other = AttrValue::List(vec![value.into()]),
        }
    }

    /// Removes `name`, returning its value.
    pub fn remove(&mut self, name: &str) -> Option<AttrValue> {
        self.0.remove(name)
    }

    /// Every attribute, sorted by name.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &AttrValue)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }
}

/// A node: an element or text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// An element.
    Element(Element),
    /// Text.
    Text(Text),
}

impl Node {
    /// Where the node came from in the parsed text, if known.
    pub fn span(&self) -> Option<Span> {
        match self {
            Node::Element(e) => e.span,
            Node::Text(t) => t.span,
        }
    }

    /// The text content: text nodes' values, concatenated in order.
    pub fn astext(&self) -> String {
        match self {
            Node::Element(e) => e.astext(),
            Node::Text(t) => t.value.clone(),
        }
    }
}

impl From<Element> for Node {
    fn from(e: Element) -> Self {
        Node::Element(e)
    }
}

impl From<Text> for Node {
    fn from(t: Text) -> Self {
        Node::Text(t)
    }
}

/// Text content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text {
    /// The text, with escapes resolved.
    pub value: String,
    /// Where it came from, if known.
    pub span: Option<Span>,
}

impl Text {
    /// Text with a span.
    pub fn new(value: impl Into<String>, span: Option<Span>) -> Self {
        Self {
            value: value.into(),
            span,
        }
    }
}

/// An element: a kind, attributes, children and a span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// The kind.
    pub kind: Kind,
    /// Attributes.
    pub attributes: Attributes,
    /// Child nodes.
    pub children: Vec<Node>,
    /// Where it came from in the parsed text, if known.
    pub span: Option<Span>,
}

impl Element {
    /// An empty element.
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            attributes: Attributes::default(),
            children: Vec::new(),
            span: None,
        }
    }

    /// The element with `span`.
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    /// The element with attribute `name` set to `value`.
    pub fn with_attr(mut self, name: impl Into<String>, value: impl Into<AttrValue>) -> Self {
        self.attributes.set(name, value);
        self
    }

    /// The element with `child` appended.
    pub fn with_child(mut self, child: impl Into<Node>) -> Self {
        self.children.push(child.into());
        self
    }

    /// The element with `children` appended.
    pub fn with_children<N: Into<Node>>(mut self, children: impl IntoIterator<Item = N>) -> Self {
        self.children.extend(children.into_iter().map(Into::into));
        self
    }

    /// The text content of all descendants, concatenated.
    pub fn astext(&self) -> String {
        self.children.iter().map(Node::astext).collect()
    }

    /// The element's child elements.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }
}
