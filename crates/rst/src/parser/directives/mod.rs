//! Directives: parsing a directive block into arguments, options and
//! content (docutils' `run_directive` and `parse_directive_block`), and
//! running the directive.

mod csv;
pub(crate) mod options;
mod standard;
mod tables;

use std::rc::Rc;

use super::document::{At, Document, ERROR, INFO, fixed};
use super::machine::{Machine, State, join, span_of};
use crate::lines::Line;
use crate::nodes::{Element, Kind, Node};
use crate::util::{split_whitespace, strip};
use options::{Opt, OptValue, convert, py_repr_opt};

/// What a directive accepts (docutils' `Directive` class attributes).
#[derive(Debug, Clone)]
pub(crate) struct Spec {
    pub(crate) required: usize,
    pub(crate) optional: usize,
    pub(crate) final_whitespace: bool,
    /// Known options; `None` for a directive without options.
    pub(crate) options: Option<&'static [(&'static str, Opt)]>,
    /// Options of a directive or role registered by the caller, or any
    /// option at all (as text).
    pub(crate) extra: Extra,
    pub(crate) has_content: bool,
}

/// Options beyond the static table.
#[derive(Debug, Clone)]
pub(crate) enum Extra {
    None,
    Public(Vec<(String, crate::extension::OptionKind)>),
    Any,
}

/// How one option is converted.
enum Converter<'a> {
    Standard(Opt),
    Public(&'a crate::extension::OptionKind),
    Text,
}

impl Converter<'_> {
    fn convert(&self, value: Option<&str>) -> Result<OptValue, String> {
        use crate::extension::OptionKind as K;
        let opt = match self {
            Converter::Standard(o) => *o,
            Converter::Text => Opt::Unchanged,
            Converter::Public(k) => match k {
                K::Flag => Opt::Flag,
                K::Text => Opt::Unchanged,
                K::RequiredText => Opt::UnchangedRequired,
                K::Path => Opt::Path,
                K::Uri => Opt::Uri,
                K::Int => Opt::Int,
                K::NonNegativeInt => Opt::NonNegativeInt,
                K::Percentage => Opt::Percentage,
                K::Length => Opt::LengthOrUnitless,
                K::LengthOrPercentage => Opt::LengthOrPercentageOrUnitless,
                K::Class => Opt::Class,
                K::Choice(values) => {
                    let values: Vec<&str> = values.iter().map(String::as_str).collect();
                    return options::choice(value, &values).map(OptValue::Str);
                }
            },
        };
        convert(opt, value)
    }
}

impl Spec {
    pub(crate) const fn new() -> Self {
        Spec {
            required: 0,
            optional: 0,
            final_whitespace: false,
            options: None,
            extra: Extra::None,
            has_content: false,
        }
    }

    fn converter(&self, name: &str) -> Option<Converter<'_>> {
        if let Some(&(_, opt)) = self.options.unwrap_or(&[]).iter().find(|(n, _)| *n == name) {
            return Some(Converter::Standard(opt));
        }
        match &self.extra {
            Extra::Public(list) => list
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, k)| Converter::Public(k)),
            Extra::Any => Some(Converter::Text),
            Extra::None => None,
        }
    }

    fn takes_arguments(&self) -> bool {
        self.required > 0 || self.optional > 0
    }

    fn has_options(&self) -> bool {
        self.options.is_some_and(|o| !o.is_empty())
            || matches!(&self.extra, Extra::Public(l) if !l.is_empty())
            || matches!(self.extra, Extra::Any)
    }
}

/// A directive being run: what the parser found in its block.
pub(crate) struct Call {
    /// The directive name as written.
    pub(crate) name: String,
    pub(crate) arguments: Vec<String>,
    /// Options in the order given.
    pub(crate) options: Vec<(String, OptValue)>,
    pub(crate) content: Vec<Line>,
    /// The absolute line offset of the first content line.
    pub(crate) content_offset: isize,
    /// The line number of the directive's first line.
    pub(crate) lineno: usize,
    /// The whole directive's text.
    pub(crate) block_text: String,
    pub(crate) span: Option<crate::nodes::Span>,
    /// Whether the directive is in a substitution definition.
    pub(crate) in_substitution: bool,
}

impl Call {
    pub(crate) fn option(&self, name: &str) -> Option<&OptValue> {
        self.options.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    pub(crate) fn has(&self, name: &str) -> bool {
        self.option(name).is_some()
    }

    pub(crate) fn take(&mut self, name: &str) -> Option<OptValue> {
        let i = self.options.iter().position(|(n, _)| n == name)?;
        Some(self.options.remove(i).1)
    }

    /// The `class` option's class names.
    pub(crate) fn classes(&self) -> Vec<String> {
        self.option("class")
            .map(OptValue::as_list)
            .unwrap_or_default()
    }

    pub(crate) fn content_text(&self) -> String {
        join(&self.content)
    }
}

/// A directive's failure: a system message of `level`, to which the
/// directive's text is added (docutils' `DirectiveError`).
pub(crate) struct Failure {
    pub(crate) level: i64,
    pub(crate) message: String,
}

pub(crate) fn failure(level: i64, message: impl Into<String>) -> Failure {
    Failure {
        level,
        message: message.into(),
    }
}

pub(crate) type Run = fn(&mut Machine, &mut Document, &mut Call) -> Result<Vec<Node>, Failure>;

/// A directive implementation, standard or registered.
type RunDyn<'a> = &'a dyn Fn(&mut Machine, &mut Document, &mut Call) -> Result<Vec<Node>, Failure>;

/// The canonical directive for an English directive name.
fn canonical(name: &str) -> Option<&'static str> {
    Some(match name {
        "attention"
        | "caution"
        | "danger"
        | "error"
        | "hint"
        | "important"
        | "note"
        | "tip"
        | "warning"
        | "admonition"
        | "sidebar"
        | "topic"
        | "line-block"
        | "parsed-literal"
        | "math"
        | "rubric"
        | "epigraph"
        | "highlights"
        | "pull-quote"
        | "compound"
        | "container"
        | "table"
        | "csv-table"
        | "list-table"
        | "meta"
        | "image"
        | "figure"
        | "include"
        | "raw"
        | "replace"
        | "unicode"
        | "date"
        | "role"
        | "default-role"
        | "title"
        | "contents"
        | "header"
        | "footer"
        | "target-notes"
        | "restructuredtext-test-directive" => {
            return standard::NAMES.iter().copied().find(|&n| n == name);
        }
        "code" | "code-block" | "sourcecode" => "code",
        "class" | "rst-class" => "class",
        "sectnum" | "section-numbering" => "sectnum",
        _ => return None,
    })
}

/// A directive's spec and implementation by canonical name.
fn registered(name: &str) -> Option<(Spec, Run)> {
    standard::lookup(name)
}

/// Why a directive block could not be parsed (docutils' `MarkupError`).
struct MarkupError(String);

impl Machine {
    /// A directive named `type_name`, its marker ending at character `end`
    /// of the current line. Returns the nodes and whether a blank line
    /// followed.
    pub(crate) fn run_directive_named(
        &mut self,
        doc: &mut Document,
        type_name: &str,
        end: usize,
        presets: Vec<(String, OptValue)>,
        in_substitution: bool,
    ) -> (Vec<Node>, bool) {
        let normname = type_name.to_lowercase();
        // Directives the caller registered come first, without lookup
        // messages (like docutils' `register_directive`).
        if let Some(directive) = doc.extensions.directives.get(&normname).cloned() {
            let public = directive.spec();
            let spec = Spec {
                required: public.required_arguments,
                optional: public.optional_arguments,
                final_whitespace: public.final_argument_whitespace,
                extra: Extra::Public(public.options),
                has_content: public.has_content,
                ..Spec::new()
            };
            let run = |m: &mut Machine, doc: &mut Document, call: &mut Call| {
                let mut cx = crate::extension::DirectiveContext {
                    run: DirectiveRun { m, doc, call },
                };
                directive
                    .run(&mut cx)
                    .map_err(|e| failure(e.level, e.message))
            };
            return self.run_directive(doc, type_name, end, spec, &run, presets, in_substitution);
        }
        let found = match canonical(&normname) {
            Some(c) => registered(c),
            // Kept unknown directives need no lookup message.
            None if doc.extensions.keep_unknown && registered(&normname).is_none() => None,
            None => {
                let info = doc.message(
                    INFO,
                    &format!(
                        "No directive entry for \"{type_name}\" in module \
                         \"docutils.parsers.rst.languages.en\".\n\
                         Trying \"{type_name}\" as canonical directive name."
                    ),
                    Vec::new(),
                    At::Top(doc.current_line),
                );
                doc.append(info);
                registered(&normname)
            }
        };
        match found {
            Some((spec, run)) => {
                self.run_directive(doc, type_name, end, spec, &run, presets, in_substitution)
            }
            None if doc.extensions.keep_unknown => {
                let spec = Spec {
                    optional: 1,
                    final_whitespace: true,
                    extra: Extra::Any,
                    has_content: true,
                    ..Spec::new()
                };
                self.run_directive(
                    doc,
                    type_name,
                    end,
                    spec,
                    &generic_directive,
                    presets,
                    in_substitution,
                )
            }
            None => self.unknown_directive(doc, type_name),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn run_directive(
        &mut self,
        doc: &mut Document,
        type_name: &str,
        end: usize,
        spec: Spec,
        run: RunDyn<'_>,
        presets: Vec<(String, OptValue)>,
        in_substitution: bool,
    ) -> (Vec<Node>, bool) {
        let lineno = self.abs_line_number();
        let initial = self.line_offset;
        let (indented, _, offset, blank_finish) =
            self.get_first_known_indented(doc, end, false, true, false);
        let block_lines = self.lines(initial, self.line_offset);
        let block_text = join(&block_lines);
        let span = span_of(&block_lines);
        let literal = |text: &str| -> Node { fixed(Kind::LiteralBlock, text, span).into() };
        let (arguments, options, content, content_offset) =
            match self.parse_directive_block(doc, indented, offset, &spec, presets) {
                Ok(parts) => parts,
                Err(MarkupError(detail)) => {
                    let error = doc.message(
                        ERROR,
                        &format!("Error in \"{type_name}\" directive:\n{detail}."),
                        vec![literal(&block_text)],
                        At::Top(Some(lineno)),
                    );
                    return (vec![error.into()], blank_finish);
                }
            };
        let mut call = Call {
            name: type_name.to_owned(),
            arguments,
            options,
            content,
            content_offset,
            lineno,
            block_text: block_text.clone(),
            span,
            in_substitution,
        };
        let result = match run(self, doc, &mut call) {
            Ok(nodes) => nodes,
            Err(f) => {
                let msg = doc.message(
                    f.level,
                    &f.message,
                    vec![literal(&block_text)],
                    At::Top(Some(lineno)),
                );
                vec![msg.into()]
            }
        };
        (result, blank_finish || self.is_next_line_blank())
    }

    /// Splits a directive block into arguments, options and content.
    #[allow(clippy::type_complexity)]
    fn parse_directive_block(
        &mut self,
        doc: &mut Document,
        mut indented: Vec<Line>,
        mut line_offset: isize,
        spec: &Spec,
        presets: Vec<(String, OptValue)>,
    ) -> Result<(Vec<String>, Vec<(String, OptValue)>, Vec<Line>, isize), MarkupError> {
        if indented.first().is_some_and(|l| strip(&l.text).is_empty()) {
            indented.remove(0);
            line_offset += 1;
        }
        while indented.last().is_some_and(|l| strip(&l.text).is_empty()) {
            indented.pop();
        }
        let mut i = 0;
        let (mut arg_block, mut content, mut content_offset);
        if !indented.is_empty() && (spec.takes_arguments() || spec.has_options()) {
            i = indented
                .iter()
                .position(|l| strip(&l.text).is_empty())
                .unwrap_or(indented.len());
            arg_block = indented[..i].to_vec();
            content = indented
                .get(i + 1..)
                .map(<[Line]>::to_vec)
                .unwrap_or_default();
            content_offset = line_offset + i as isize + 1;
        } else {
            content = indented.clone();
            content_offset = line_offset;
            arg_block = Vec::new();
        }
        let mut options = Vec::new();
        if spec.has_options() {
            let (o, rest) = self.parse_directive_options(doc, presets, spec, arg_block)?;
            options = o;
            arg_block = rest;
        }
        if !arg_block.is_empty() && !spec.takes_arguments() {
            content = arg_block;
            content.extend_from_slice(&indented[i.min(indented.len())..]);
            content_offset = line_offset;
            arg_block = Vec::new();
        }
        while content.first().is_some_and(|l| strip(&l.text).is_empty()) {
            content.remove(0);
            content_offset += 1;
        }
        let arguments = if spec.takes_arguments() {
            parse_directive_arguments(spec, &arg_block)?
        } else {
            Vec::new()
        };
        if !content.is_empty() && !spec.has_content {
            return Err(MarkupError("no content permitted".to_owned()));
        }
        Ok((arguments, options, content, content_offset))
    }

    /// Splits off the field list of options at the end of the arguments.
    #[allow(clippy::type_complexity)]
    fn parse_directive_options(
        &mut self,
        doc: &mut Document,
        presets: Vec<(String, OptValue)>,
        spec: &Spec,
        arg_block: Vec<Line>,
    ) -> Result<(Vec<(String, OptValue)>, Vec<Line>), MarkupError> {
        let mut options = presets;
        let split = arg_block.iter().position(|l| {
            crate::patterns::field_marker(&l.text.chars().collect::<Vec<_>>()).is_some()
        });
        let (args, opt_block) = match split {
            Some(i) => (arg_block[..i].to_vec(), arg_block[i..].to_vec()),
            None => (arg_block, Vec::new()),
        };
        if !opt_block.is_empty() {
            let parsed = self.parse_extension_options(doc, spec, opt_block)?;
            for (name, value) in parsed {
                match options.iter_mut().find(|(n, _)| *n == name) {
                    Some(slot) => slot.1 = value,
                    None => options.push((name, value)),
                }
            }
        }
        Ok((options, args))
    }

    /// Parses option fields and converts their values.
    fn parse_extension_options(
        &mut self,
        doc: &mut Document,
        spec: &Spec,
        lines: Vec<Line>,
    ) -> Result<Vec<(String, OptValue)>, MarkupError> {
        let count = lines.len() as isize;
        doc.open(Element::new(Kind::FieldList), false);
        let mut m = Machine::new(Rc::from(lines), 0, 0);
        m.blank_finish = true;
        m.run(doc, State::ExtensionOptions);
        let field_list = doc.close().unwrap();
        if m.abs_line_offset() != count {
            return Err(MarkupError("invalid option block".to_owned()));
        }
        let mut raw = Vec::new();
        for field in field_list.elements() {
            let mut parts = field.elements();
            let (Some(name), Some(body)) = (parts.next(), parts.next()) else {
                continue;
            };
            let name_text = super::inline::astext(name);
            if split_whitespace(&name_text).count() != 1 {
                return Err(MarkupError(
                    "invalid option data: extension option field name may not contain multiple words"
                        .to_owned(),
                ));
            }
            let name = name_text.to_lowercase();
            let data = if body.children.is_empty() {
                None
            } else {
                match body.children.as_slice() {
                    [Node::Element(p)] if p.kind == Kind::Paragraph && p.children.len() == 1 => {
                        match &p.children[0] {
                            Node::Text(t) => Some(crate::util::unescape(&t.value)),
                            Node::Element(_) => None,
                        }
                        .ok_or_else(|| single_paragraph(&name))?
                        .into()
                    }
                    _ => return Err(single_paragraph(&name)),
                }
            };
            raw.push((name, data));
        }
        let mut options: Vec<(String, OptValue)> = Vec::new();
        for (name, value) in raw {
            let Some(converter) = spec.converter(&name) else {
                return Err(MarkupError(format!("unknown option: \"{name}\"")));
            };
            if options.iter().any(|(n, _)| *n == name) {
                return Err(MarkupError(format!(
                    "invalid option data: duplicate option \"{name}\""
                )));
            }
            match converter.convert(value.as_deref()) {
                Ok(v) => options.push((name, v)),
                Err(detail) => {
                    return Err(MarkupError(format!(
                        "invalid option value: (option: \"{name}\"; value: {})\n{detail}",
                        py_repr_opt(value.as_deref())
                    )));
                }
            }
        }
        if !m.blank_finish {
            return Err(MarkupError("option data incompletely parsed".to_owned()));
        }
        Ok(options)
    }

    /// An error for an unknown directive, with the directive's text.
    pub(crate) fn unknown_directive(
        &mut self,
        doc: &mut Document,
        name: &str,
    ) -> (Vec<Node>, bool) {
        let start = self.line_offset;
        let lineno = self.abs_line_number();
        let (indented, _, _, blank_finish) =
            self.get_first_known_indented(doc, 0, false, false, true);
        let text = join(&indented);
        let span = self.span(start, self.line_offset);
        let error = doc.message(
            ERROR,
            &format!("Unknown directive type \"{name}\"."),
            vec![fixed(Kind::LiteralBlock, &text, span).into()],
            At::Top(Some(lineno)),
        );
        (vec![error.into()], blank_finish)
    }
}

/// An unknown directive kept as a generic `directive` element: its name
/// and argument as attributes, its options as a field list, its content
/// as a literal block.
fn generic_directive(
    _: &mut Machine,
    _: &mut Document,
    call: &mut Call,
) -> Result<Vec<Node>, Failure> {
    let mut node =
        Element::new(Kind::Other("directive".into())).with_attr("name", call.name.clone());
    node.span = call.span;
    if let Some(argument) = call.arguments.first() {
        node.attributes.set("argument", argument.clone());
    }
    if !call.options.is_empty() {
        let mut list = Element::new(Kind::FieldList);
        for (name, value) in &call.options {
            let text = value.as_str().unwrap_or("").to_owned();
            let mut body = Element::new(Kind::FieldBody);
            if !text.is_empty() {
                body.children.push(super::document::paragraph(&text).into());
            }
            list.children.push(
                Element::new(Kind::Field)
                    .with_child(
                        Element::new(Kind::FieldName)
                            .with_child(crate::nodes::Text::new(name.clone(), None)),
                    )
                    .with_child(body)
                    .into(),
            );
        }
        node.children.push(list.into());
    }
    if !call.content.is_empty() {
        node.children.push(
            fixed(
                Kind::LiteralBlock,
                &call.content_text(),
                span_of(&call.content),
            )
            .into(),
        );
    }
    Ok(vec![node.into()])
}

/// What a registered directive's context reads and does.
pub(crate) struct DirectiveRun<'a> {
    m: &'a mut Machine,
    doc: &'a mut Document,
    call: &'a mut Call,
}

impl DirectiveRun<'_> {
    pub(crate) fn name(&self) -> &str {
        &self.call.name
    }

    pub(crate) fn arguments(&self) -> &[String] {
        &self.call.arguments
    }

    pub(crate) fn options(&self) -> Vec<(String, crate::extension::OptionValue)> {
        self.call
            .options
            .iter()
            .map(|(k, v)| (k.clone(), public_value(v)))
            .collect()
    }

    pub(crate) fn content(&self) -> Vec<String> {
        self.call.content.iter().map(|l| l.text.clone()).collect()
    }

    pub(crate) fn lineno(&self) -> usize {
        self.call.lineno
    }

    pub(crate) fn block_text(&self) -> &str {
        &self.call.block_text
    }

    pub(crate) fn span(&self) -> Option<crate::nodes::Span> {
        self.call.span
    }

    pub(crate) fn in_substitution(&self) -> bool {
        self.call.in_substitution
    }

    pub(crate) fn parse_content(&mut self, container: Element) -> Element {
        standard::parse_content(self.m, self.doc, self.call, container)
    }

    pub(crate) fn parse_inline(&mut self, text: &str) -> (Vec<Node>, Vec<Element>) {
        super::inline::inline_text(
            self.doc,
            text,
            self.call.lineno,
            &super::inline::Origin::default(),
        )
    }

    pub(crate) fn message(&mut self, level: i64, text: &str) -> Element {
        self.doc
            .message(level, text, Vec::new(), At::Top(Some(self.call.lineno)))
    }
}

/// An option value as the public type.
pub(crate) fn public_value(v: &OptValue) -> crate::extension::OptionValue {
    use crate::extension::OptionValue as V;
    match v {
        OptValue::None => V::Flag,
        OptValue::Str(s) => V::Text(s.clone()),
        OptValue::Int(n) => V::Int(*n),
        OptValue::List(l) => V::List(l.clone()),
        OptValue::IntList(l) => V::List(l.iter().map(i64::to_string).collect()),
    }
}

fn single_paragraph(name: &str) -> MarkupError {
    MarkupError(format!(
        "invalid option data: extension option field body may contain\n\
         a single paragraph only (option \"{name}\")"
    ))
}

/// The directive's arguments from the argument block.
fn parse_directive_arguments(spec: &Spec, arg_block: &[Line]) -> Result<Vec<String>, MarkupError> {
    let text = join(arg_block);
    let arguments: Vec<String> = split_whitespace(&text).map(str::to_owned).collect();
    let (required, total) = (spec.required, spec.required + spec.optional);
    if arguments.len() < required {
        return Err(MarkupError(format!(
            "{required} argument(s) required, {} supplied",
            arguments.len()
        )));
    }
    if arguments.len() > total {
        if spec.final_whitespace {
            return Ok(python_split(&text, total - 1));
        }
        return Err(MarkupError(format!(
            "maximum {total} argument(s) allowed, {} supplied",
            arguments.len()
        )));
    }
    Ok(arguments)
}

/// Python's `str.split(None, maxsplit)`.
pub(crate) fn python_split(text: &str, maxsplit: usize) -> Vec<String> {
    let ws = crate::lines::is_python_whitespace;
    let mut out = Vec::new();
    let mut rest = text.trim_start_matches(ws);
    while !rest.is_empty() {
        if out.len() == maxsplit {
            out.push(rest.to_owned());
            break;
        }
        let end = rest.find(ws).unwrap_or(rest.len());
        out.push(rest[..end].to_owned());
        rest = rest[end..].trim_start_matches(ws);
    }
    out
}
