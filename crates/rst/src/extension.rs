//! Extending the parser with directives and interpreted text roles.
//!
//! Register them on a [`Parser`](crate::Parser):
//!
//! ```
//! use lsdocs_rst::{Directive, DirectiveContext, DirectiveError, DirectiveSpec, Element, Kind, Node, Parser};
//!
//! /// `.. shout:: text` makes a paragraph of the text in capitals.
//! struct Shout;
//!
//! impl Directive for Shout {
//!     fn spec(&self) -> DirectiveSpec {
//!         DirectiveSpec { required_arguments: 1, final_argument_whitespace: true, ..Default::default() }
//!     }
//!
//!     fn run(&self, cx: &mut DirectiveContext<'_>) -> Result<Vec<Node>, DirectiveError> {
//!         let text = cx.arguments()[0].to_uppercase();
//!         let (nodes, messages) = cx.parse_inline(&text);
//!         let mut out = vec![Node::from(Element::new(Kind::Paragraph).with_children(nodes))];
//!         out.extend(messages.into_iter().map(Node::from));
//!         Ok(out)
//!     }
//! }
//!
//! let doc = Parser::new().directive("shout", Shout).parse(".. shout:: hello *world*\n");
//! assert_eq!(doc.astext(), "HELLO WORLD");
//! ```

use crate::nodes::{Element, Node, Span};
use crate::parser::{DirectiveRun, RoleRun};

/// How a directive option's text is checked and converted (docutils'
/// option conversion functions).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OptionKind {
    /// No value allowed (`directives.flag`).
    Flag,
    /// Any text, empty if none (`directives.unchanged`).
    Text,
    /// Any text, required (`directives.unchanged_required`).
    RequiredText,
    /// A path: lines joined, whitespace at their ends removed.
    Path,
    /// A URI: whitespace removed unless escaped.
    Uri,
    /// An integer.
    Int,
    /// An integer of 0 or more.
    NonNegativeInt,
    /// A percentage, the `%` optional.
    Percentage,
    /// A CSS length or a plain number.
    Length,
    /// A CSS length, a percentage or a plain number.
    LengthOrPercentage,
    /// Class names, made identifier-safe (`directives.class_option`).
    Class,
    /// One of these values (lower-case).
    Choice(Vec<String>),
}

/// A converted option value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OptionValue {
    /// A flag option (no value).
    Flag,
    /// Text.
    Text(String),
    /// A number.
    Int(i64),
    /// A list, such as class names.
    List(Vec<String>),
}

/// What a directive accepts. The defaults: no arguments, no options, no
/// content.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirectiveSpec {
    /// Arguments that must be given.
    pub required_arguments: usize,
    /// Arguments that may follow the required ones.
    pub optional_arguments: usize,
    /// Whether the last argument may contain whitespace.
    pub final_argument_whitespace: bool,
    /// The options, by name.
    pub options: Vec<(String, OptionKind)>,
    /// Whether the directive may have content.
    pub has_content: bool,
}

/// A directive's failure: reported as a system message of `level`
/// (1 info, 2 warning, 3 error, 4 severe) with the directive's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectiveError {
    /// The message level.
    pub level: i64,
    /// The message.
    pub message: String,
}

impl DirectiveError {
    /// A warning.
    pub fn warning(message: impl Into<String>) -> Self {
        Self {
            level: 2,
            message: message.into(),
        }
    }

    /// An error.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            level: 3,
            message: message.into(),
        }
    }
}

/// A directive: `.. name:: arguments`, options and content.
pub trait Directive: Send + Sync {
    /// The arguments, options and content the directive accepts.
    fn spec(&self) -> DirectiveSpec;

    /// Makes the directive's nodes. They go where the directive is.
    fn run(&self, cx: &mut DirectiveContext<'_>) -> Result<Vec<Node>, DirectiveError>;
}

/// A directive being run: what was written, and access to the parser.
pub struct DirectiveContext<'a> {
    pub(crate) run: DirectiveRun<'a>,
}

impl DirectiveContext<'_> {
    /// The directive name as written.
    pub fn name(&self) -> &str {
        self.run.name()
    }

    /// The arguments.
    pub fn arguments(&self) -> &[String] {
        self.run.arguments()
    }

    /// The options in the order given.
    pub fn options(&self) -> Vec<(String, OptionValue)> {
        self.run.options()
    }

    /// The value of option `name`, if given.
    pub fn option(&self, name: &str) -> Option<OptionValue> {
        self.options()
            .into_iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
    }

    /// The content, line by line, indentation removed.
    pub fn content(&self) -> Vec<String> {
        self.run.content()
    }

    /// The line number of the directive's first line.
    pub fn line(&self) -> usize {
        self.run.lineno()
    }

    /// The directive's whole text.
    pub fn block_text(&self) -> &str {
        self.run.block_text()
    }

    /// Where the directive is in the source.
    pub fn span(&self) -> Option<Span> {
        self.run.span()
    }

    /// Whether the directive is in a substitution definition.
    pub fn in_substitution(&self) -> bool {
        self.run.in_substitution()
    }

    /// Parses the content as body elements into `container`, and returns
    /// it.
    pub fn parse_content(&mut self, container: Element) -> Element {
        self.run.parse_content(container)
    }

    /// Parses `text` as inline markup: the nodes, and messages to place
    /// after the element holding them.
    pub fn parse_inline(&mut self, text: &str) -> (Vec<Node>, Vec<Element>) {
        self.run.parse_inline(text)
    }

    /// A system message (level 1 info to 4 severe) for the directive's
    /// line.
    pub fn message(&mut self, level: i64, text: &str) -> Element {
        self.run.message(level, text)
    }
}

/// An interpreted text role: `` :name:`text` ``.
pub trait Role: Send + Sync {
    /// Makes the role's inline nodes. An error becomes a `problematic`
    /// element and a system message, as for docutils' own roles.
    fn run(&self, cx: &mut RoleContext<'_>) -> Result<Vec<Node>, DirectiveError>;
}

/// A role being run.
pub struct RoleContext<'a> {
    pub(crate) run: RoleRun<'a>,
}

impl RoleContext<'_> {
    /// The role name as written.
    pub fn name(&self) -> &str {
        self.run.name
    }

    /// The text, escapes removed.
    pub fn text(&self) -> String {
        crate::util::unescape(self.run.text)
    }

    /// The text, backslashes kept (as written).
    pub fn raw_text(&self) -> String {
        crate::util::restore_backslashes(self.run.text)
    }

    /// The whole interpreted text as written, role included.
    pub fn rawsource(&self) -> &str {
        self.run.rawsource
    }

    /// Options given when the role was made with the `role` directive.
    pub fn options(&self) -> Vec<(String, OptionValue)> {
        self.run.options()
    }

    /// The line number the text is on.
    pub fn line(&self) -> usize {
        self.run.lineno
    }

    /// Where the text is in the source.
    pub fn span(&self) -> Option<Span> {
        self.run.span
    }
}
