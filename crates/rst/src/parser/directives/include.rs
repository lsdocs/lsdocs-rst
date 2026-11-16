//! The directives that read files: `include`, and the file parts of
//! `raw` and `csv-table` (docutils' `directives.misc.Include`, `Raw` and
//! `tables.CSVTable`), through the caller's [`FileLoader`].
//!
//! [`FileLoader`]: crate::files::FileLoader

use std::rc::Rc;

use super::options::OptValue;
use super::standard::{add_name, code_block, set_classes};
use super::{Call, Failure, failure};
use crate::files::{adapt_path, decode, os_error};
use crate::lines::{Line, LineOrigin, lines_with, split_lines};
use crate::nodes::{Kind, Node, Span, Text};
use crate::parser::document::{Document, ERROR, IncludeClip, SEVERE, WARNING, fixed};
use crate::parser::machine::Machine;

/// The path of a `:file:` option or `include` argument, resolved against
/// the file being parsed.
pub(super) fn resolve(doc: &Document, path: &str) -> String {
    let source = doc.current_source.as_deref().or(doc.source());
    adapt_path(path, source)
}

/// Reads and decodes the file at `path` for directive `name`.
pub(super) fn read_file(
    doc: &Document,
    name: &str,
    path: &str,
    call: &Call,
) -> Result<String, Failure> {
    let Some(loader) = doc.extensions.loader.clone() else {
        return Err(failure(WARNING, format!("\"{name}\" directive disabled.")));
    };
    let bytes = loader.read_file(path).map_err(|e| {
        failure(
            ERROR,
            format!(
                "Problems with \"{name}\" directive path:\nInputError: {}.",
                os_error(&e, path)
            ),
        )
    })?;
    decode(&bytes, encoding(call))
        .map_err(|e| failure(ERROR, format!("Problem with \"{name}\" directive:\n{e}")))
}

/// Reads `url` through the loader: `Err` with the error as Python prints
/// it (`URLError: ...`) if it cannot be read, else the decoded text or
/// the decoding error.
pub(super) fn read_url(
    doc: &Document,
    url: &str,
    call: &Call,
) -> Result<Result<String, String>, String> {
    let loader = doc.extensions.loader.clone().ok_or_else(String::new)?;
    let bytes = loader
        .read_url(url)
        .map_err(|e| format!("URLError: <urlopen error {e}>"))?;
    Ok(decode(&bytes, encoding(call)))
}

/// Reads the file at `path` for `csv-table`: its lines, or the failure
/// docutils reports.
pub(super) fn read_csv_file(doc: &Document, path: &str, call: &Call) -> Result<String, Failure> {
    let name = &call.name;
    let Some(loader) = doc.extensions.loader.clone() else {
        return Err(failure(WARNING, format!("\"{name}\" directive disabled.")));
    };
    let bytes = loader.read_file(path).map_err(|e| {
        failure(
            SEVERE,
            format!(
                "Problems with \"{name}\" directive path:\n{}.",
                os_error(&e, path)
            ),
        )
    })?;
    decode(&bytes, encoding(call))
        .map_err(|e| failure(ERROR, format!("Problem with \"{name}\" directive:\n{e}")))
}

fn encoding(call: &Call) -> Option<&str> {
    match call.option("encoding") {
        Some(OptValue::Str(s)) => Some(s),
        _ => None,
    }
}

fn int(call: &Call, name: &str) -> Option<i64> {
    match call.option(name) {
        Some(OptValue::Int(n)) => Some(*n),
        _ => None,
    }
}

fn text_option(call: &Call, name: &str) -> Option<String> {
    match call.option(name) {
        Some(OptValue::Str(s)) => Some(s.clone()),
        Some(OptValue::None) => Some(String::new()),
        _ => None,
    }
}

/// Python's `seq[start:end]` bounds for a sequence of `len` items.
fn py_slice(len: usize, start: Option<i64>, end: Option<i64>) -> (usize, usize) {
    let len = len as i64;
    let bound = |i: i64| if i < 0 { (i + len).max(0) } else { i.min(len) };
    let a = start.map_or(0, bound);
    let b = end.map_or(len, bound);
    (a as usize, b.max(a) as usize)
}

/// `include`: inserts a file's lines into the input, or makes a literal
/// or code block of it.
pub(super) fn include(
    m: &mut Machine,
    doc: &mut Document,
    call: &mut Call,
) -> Result<Vec<Node>, Failure> {
    let name = call.name.clone();
    if doc.extensions.loader.is_none() {
        return Err(failure(WARNING, format!("\"{name}\" directive disabled.")));
    }
    if call.has("parser") {
        return Err(failure(
            ERROR,
            format!("The \"parser\" option of the \"{name}\" directive is not supported."),
        ));
    }
    let tab_width = int(call, "tab-width").unwrap_or(doc.extensions.settings.tab_width as i64);
    let clip: IncludeClip = (
        int(call, "start-line"),
        int(call, "end-line"),
        text_option(call, "start-after"),
        text_option(call, "end-before"),
    );
    let argument: String = split_lines(&call.arguments[0])
        .map(crate::util::strip)
        .collect();
    let path = resolve(doc, &argument);
    let text = read_file(doc, &name, &path, call)?;
    let text = clip_text(text, &clip, &name)?;
    if call.has("literal") {
        return Ok(vec![literal_block(doc, call, &text, &path, tab_width)]);
    }
    if call.has("code") {
        let language = text_option(call, "code").unwrap_or_default();
        let text = expand(&text, tab_width);
        let text = text.strip_suffix('\n').unwrap_or(&text).to_owned();
        return Ok(vec![
            code_block(doc, call, &language, &text, 1, Some(&path)).into(),
        ]);
    }
    insert(m, doc, call, &text, &path, tab_width, clip)?;
    Ok(Vec::new())
}

/// The part of `text` the clipping options select.
fn clip_text(text: String, clip: &IncludeClip, name: &str) -> Result<String, Failure> {
    let (start_line, end_line, start_after, end_before) = clip;
    let mut text = text;
    if start_line.is_some_and(|n| n != 0) || end_line.is_some() {
        let lines: Vec<&str> = split_lines(&text).collect();
        let (a, b) = py_slice(lines.len(), *start_line, *end_line);
        text = lines[a..b].join("\n");
    }
    let start_after = match start_after.as_deref() {
        Some("") => Some("\n\n"),
        other => other,
    };
    if let Some(after) = start_after {
        match text.find(after) {
            Some(i) => text = text[i + after.len()..].to_owned(),
            None => {
                return Err(failure(
                    ERROR,
                    format!(
                        "Problem with \"start-after\" option of \"{name}\" directive:\nText not found."
                    ),
                ));
            }
        }
    }
    match end_before.as_deref() {
        Some("") => {
            if let Some(i) = text.find("\n\n")
                && i > 0
            {
                text.truncate(i + 1);
            }
        }
        Some(before) => match text.find(before) {
            Some(i) => text.truncate(i),
            None => {
                return Err(failure(
                    ERROR,
                    format!(
                        "Problem with \"end-before\" option of \"{name}\" directive:\nText not found."
                    ),
                ));
            }
        },
        None => {}
    }
    Ok(text)
}

/// `text` with tabs expanded (Python's `expandtabs`; a negative width
/// leaves them).
fn expand(text: &str, tab_width: i64) -> String {
    if tab_width < 0 {
        return text.to_owned();
    }
    text.split('\n')
        .map(|l| crate::lines::expand_tabs_to(l, tab_width as usize))
        .collect::<Vec<_>>()
        .join("\n")
}

fn literal_block(
    doc: &mut Document,
    call: &mut Call,
    text: &str,
    path: &str,
    tab_width: i64,
) -> Node {
    let text = expand(text, tab_width);
    let mut node = fixed(Kind::LiteralBlock, "", call.span);
    node.attributes.set("source", path);
    set_classes(&mut node, call);
    add_name(doc, call, &mut node);
    match call.option("number-lines") {
        Some(start) => {
            let start = match start {
                OptValue::Int(n) => *n,
                _ => 1,
            };
            let text = text.strip_suffix('\n').unwrap_or(&text);
            let end = start + split_lines(text).count() as i64;
            super::standard::number_lines(&mut node, text, start, end);
        }
        None => node.children.push(Text::new(text, call.span).into()),
    }
    node.into()
}

/// Inserts `text` into the input after the directive.
#[allow(clippy::too_many_arguments)]
fn insert(
    m: &mut Machine,
    doc: &mut Document,
    call: &Call,
    text: &str,
    path: &str,
    tab_width: i64,
    clip: IncludeClip,
) -> Result<(), Failure> {
    let source: Rc<str> = Rc::from(path);
    let at = call.span.map_or(0, |s| s.start);
    let span = Span::new(at, at);
    let mut lines = lines_with(
        text,
        tab_width.max(0) as usize,
        Some(source.clone()),
        Some(span),
    );
    for (i, line) in lines.iter().enumerate() {
        if line.text.chars().count() > doc.extensions.settings.line_length_limit {
            let line_no = i as i64 + 1 + clip.0.unwrap_or(0);
            return Err(failure(
                WARNING,
                format!("\"{path}\": line {line_no} exceeds the line-length-limit."),
            ));
        }
    }
    if doc.include_log.is_empty() {
        let current = doc
            .current_source
            .as_deref()
            .or(doc.source())
            .unwrap_or_default()
            .to_owned();
        doc.include_log.push((current, (None, None, None, None)));
    }
    let entry = (path.to_owned(), clip);
    if doc.include_log.contains(&entry) {
        let mut chain = vec![path.to_owned()];
        chain.extend(doc.include_log.iter().rev().map(|(p, _)| p.clone()));
        return Err(failure(
            WARNING,
            format!(
                "circular inclusion in \"{}\" directive:\n{}",
                call.name,
                chain.join("\n> ")
            ),
        ));
    }
    doc.include_log.push(entry);
    let n = lines.len() as isize;
    for (i, text) in [
        String::new(),
        format!(".. end of inclusion from \"{path}\""),
    ]
    .into_iter()
    .enumerate()
    {
        lines.push(Line {
            text,
            span,
            origin: LineOrigin {
                source: Some(source.clone()),
                offset: n + i as isize,
            },
        });
    }
    m.insert_input(lines, path);
    Ok(())
}
