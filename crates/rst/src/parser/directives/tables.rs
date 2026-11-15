//! The table directives (docutils' `directives.tables`): `table`,
//! `csv-table` and `list-table`.

use super::csv::{self, Dialect};
use super::options::{Opt, OptValue};
use super::{Call, Failure, failure};
use crate::lines::Line;
use crate::nodes::{Element, Kind, Node};
use crate::parser::document::{Document, ERROR, MsgNode, WARNING};
use crate::parser::inline::{Origin, inline_text};
use crate::parser::machine::Machine;
use crate::parser::table::{TableData, Widths};
use crate::util::normalize_name;

pub(super) const TABLE_OPTIONS: &[(&str, Opt)] = &[
    ("class", Opt::Class),
    ("name", Opt::Unchanged),
    ("align", Opt::Choice(&["left", "center", "right"])),
    ("width", Opt::LengthOrPercentageOrUnitless),
    ("widths", Opt::Widths { grid: true }),
];

pub(super) const CSV_OPTIONS: &[(&str, Opt)] = &[
    ("header-rows", Opt::NonNegativeInt),
    ("stub-columns", Opt::NonNegativeInt),
    ("header", Opt::Unchanged),
    ("width", Opt::LengthOrPercentageOrUnitless),
    ("widths", Opt::Widths { grid: false }),
    ("file", Opt::Path),
    ("url", Opt::Uri),
    ("encoding", Opt::Encoding),
    ("class", Opt::Class),
    ("name", Opt::Unchanged),
    ("align", Opt::Choice(&["left", "center", "right"])),
    ("delim", Opt::SingleCharOrWhitespace),
    ("keepspace", Opt::Flag),
    ("quote", Opt::SingleChar),
    ("escape", Opt::SingleChar),
];

pub(super) const LIST_OPTIONS: &[(&str, Opt)] = &[
    ("header-rows", Opt::NonNegativeInt),
    ("stub-columns", Opt::NonNegativeInt),
    ("width", Opt::LengthOrPercentageOrUnitless),
    ("widths", Opt::Widths { grid: false }),
    ("class", Opt::Class),
    ("name", Opt::Unchanged),
    ("align", Opt::Choice(&["left", "center", "right"])),
];

/// The table title from the argument, and its messages.
fn make_title(doc: &mut Document, call: &Call) -> (Option<Element>, Vec<Element>) {
    match call.arguments.first() {
        Some(text) => {
            let (nodes, messages) = inline_text(doc, text, call.lineno, &Origin::default());
            (
                Some(Element::new(Kind::Title).with_children(nodes)),
                messages,
            )
        }
        None => (None, Vec::new()),
    }
}

fn widths(call: &Call) -> Widths {
    match call.option("widths") {
        Some(OptValue::Str(s)) if s == "auto" => Widths::Auto,
        Some(OptValue::Str(s)) if s == "grid" => Widths::Grid,
        Some(OptValue::IntList(l)) => Widths::Given(l.clone()),
        _ => Widths::Default,
    }
}

fn int_option(call: &Call, name: &str) -> usize {
    match call.option(name) {
        Some(OptValue::Int(n)) => *n as usize,
        _ => 0,
    }
}

/// Checks header rows and stub columns against the rows' lengths.
fn check_dimensions(
    call: &Call,
    rows: &[usize],
    header_rows: usize,
    stub_columns: usize,
) -> Result<(), Failure> {
    let name = &call.name;
    if rows.len() < header_rows {
        return Err(failure(
            ERROR,
            format!(
                "{header_rows} header row(s) specified but only {} row(s) of data supplied (\"{name}\" directive).",
                rows.len()
            ),
        ));
    }
    if rows.len() == header_rows && header_rows > 0 {
        return Err(failure(
            ERROR,
            format!(
                "Insufficient data supplied ({} row(s)); no data remaining for table body, \
                 required by \"{name}\" directive.",
                rows.len()
            ),
        ));
    }
    for &len in rows {
        if len < stub_columns {
            return Err(failure(
                ERROR,
                format!(
                    "{stub_columns} stub column(s) specified but only {len} columns(s) of data \
                     supplied (\"{name}\" directive)."
                ),
            ));
        }
        if len == stub_columns && stub_columns > 0 {
            return Err(failure(
                ERROR,
                format!(
                    "Insufficient data supplied ({len} columns(s)); no data remaining for table \
                     body, required by \"{name}\" directive."
                ),
            ));
        }
    }
    Ok(())
}

/// Column widths: as given, or equal shares of 100.
fn column_widths(call: &Call, n_cols: usize) -> Result<Vec<i64>, Failure> {
    match widths(call) {
        Widths::Given(w) => {
            if w.len() != n_cols {
                return Err(failure(
                    ERROR,
                    format!(
                        "\"{}\" widths do not match the number of columns in table ({n_cols}).",
                        call.name
                    ),
                ));
            }
            Ok(w)
        }
        _ if n_cols > 0 => Ok(vec![100 / n_cols as i64; n_cols]),
        _ => Err(failure(ERROR, "No table data detected in CSV file.")),
    }
}

/// The options all table directives set on the table, then the name and
/// title.
fn finish(
    doc: &mut Document,
    call: &mut Call,
    table: &mut Element,
    title: Option<Element>,
    align_first: bool,
) {
    let align = call
        .option("align")
        .and_then(OptValue::as_str)
        .map(str::to_owned);
    if align_first && let Some(a) = &align {
        table.attributes.set("align", a.clone());
    }
    for c in call.classes() {
        table.attributes.push("classes", c);
    }
    if let Some(OptValue::Str(w)) = call.option("width") {
        table.attributes.set("width", w.clone());
    }
    if !align_first && let Some(a) = align {
        table.attributes.set("align", a);
    }
    if let Some(name) = call.take("name") {
        table
            .attributes
            .push("names", normalize_name(name.as_str().unwrap_or("")));
        doc.note_explicit_target(table, Some(MsgNode::Itself), Some(call.lineno));
    }
    if let Some(t) = title {
        table.children.insert(0, t.into());
    }
}

/// `table`: a title and options for a grid or simple table.
pub(super) fn table(
    m: &mut Machine,
    doc: &mut Document,
    call: &mut Call,
) -> Result<Vec<Node>, Failure> {
    if call.content.is_empty() {
        return Err(failure(
            WARNING,
            format!(
                "Content block expected for the \"{}\" directive; none found.",
                call.name
            ),
        ));
    }
    let (title, messages) = make_title(doc, call);
    let children = super::standard::parse_into_container(m, doc, call);
    let mut table = match children.as_slice() {
        [Node::Element(t)] if t.kind == Kind::Table => t.clone(),
        _ => {
            return Err(failure(
                ERROR,
                format!(
                    "Error parsing content block for the \"{}\" directive: exactly one table expected.",
                    call.name
                ),
            ));
        }
    };
    for c in call.classes() {
        table.attributes.push("classes", c);
    }
    if let Some(OptValue::Str(w)) = call.option("width") {
        table.attributes.set("width", w.clone());
    }
    if let Some(a) = call.option("align").and_then(OptValue::as_str) {
        table.attributes.set("align", a.to_owned());
    }
    let w = widths(call);
    if let Widths::Given(_) = w
        && let Some(Node::Element(tgroup)) = table.children.first_mut()
    {
        let cols = match tgroup.attributes.get("cols") {
            Some(crate::nodes::AttrValue::Int(n)) => *n as usize,
            _ => 0,
        };
        let col_widths = column_widths(call, cols)?;
        let colspecs = tgroup.children.iter_mut().filter_map(|c| match c {
            Node::Element(e) if e.kind == Kind::Colspec => Some(e),
            _ => None,
        });
        for (colspec, width) in colspecs.zip(col_widths) {
            colspec.attributes.set("colwidth", width);
        }
    }
    w.classes(&mut table);
    if let Some(name) = call.take("name") {
        table
            .attributes
            .push("names", normalize_name(name.as_str().unwrap_or("")));
        doc.note_explicit_target(&mut table, Some(MsgNode::Itself), Some(call.lineno));
    }
    if let Some(t) = title {
        table.children.insert(0, t.into());
    }
    let mut out = vec![Node::from(table)];
    out.extend(messages.into_iter().map(Node::from));
    Ok(out)
}

/// The CSV dialect the options ask for.
fn dialect(call: &Call) -> Dialect {
    let ch = |name: &str| {
        call.option(name)
            .and_then(OptValue::as_str)
            .and_then(|s| s.chars().next())
    };
    let escape = ch("escape");
    Dialect {
        delimiter: ch("delim").unwrap_or(','),
        quotechar: ch("quote").unwrap_or('"'),
        escapechar: escape,
        doublequote: escape.is_none(),
        skipinitialspace: !call.has("keepspace"),
        strict: true,
    }
}

/// A CSV row: its cells' lines.
type CsvRow = Vec<Vec<Line>>;

/// `csv-table`: a table from comma-separated values.
pub(super) fn csv_table(
    m: &mut Machine,
    doc: &mut Document,
    call: &mut Call,
) -> Result<Vec<Node>, Failure> {
    if call.has("file") || call.has("url") {
        return Err(failure(
            WARNING,
            format!(
                "File and URL access deactivated; ignoring \"{}\" directive.",
                call.name
            ),
        ));
    }
    let (title, messages) = make_title(doc, call);
    if call.content.is_empty() {
        return Err(failure(
            WARNING,
            format!(
                "The \"{}\" directive requires content; none supplied.",
                call.name
            ),
        ));
    }
    let d = dialect(call);
    let csv_error = |e: String| {
        failure(
            ERROR,
            format!("Error with CSV data in \"{}\" directive:\n{e}", call.name),
        )
    };
    let read = |texts: &[String], lines: &[Line]| -> Result<(Vec<CsvRow>, usize), String> {
        let rows = csv::read(texts, &d)?;
        let mut out = Vec::new();
        let mut max_cols = 0;
        for (fields, start) in rows {
            let span = lines.get(start).map(|l| l.span).unwrap_or_default();
            max_cols = max_cols.max(fields.len());
            out.push(
                fields
                    .iter()
                    .map(|f| {
                        crate::lines::split_lines(f)
                            .map(|t| Line {
                                text: t.to_owned(),
                                span,
                            })
                            .collect()
                    })
                    .collect(),
            );
        }
        Ok((out, max_cols))
    };
    let mut head = Vec::new();
    let mut max_header_cols = 0;
    if let Some(OptValue::Str(header)) = call.option("header") {
        let texts: Vec<String> = header.split('\n').map(str::to_owned).collect();
        let (rows, cols) = read(&texts, &[]).map_err(csv_error)?;
        head.extend(rows);
        max_header_cols = cols;
    }
    let texts: Vec<String> = call.content.iter().map(|l| l.text.clone()).collect();
    let (rows, max_cols) = read(&texts, &call.content).map_err(csv_error)?;
    let max_cols = max_cols.max(max_header_cols);
    let header_rows = int_option(call, "header-rows");
    let stub_columns = int_option(call, "stub-columns");
    let lens: Vec<usize> = rows.iter().map(Vec::len).collect();
    check_dimensions(call, &lens, header_rows, stub_columns)?;
    let mut rows = rows;
    let body = rows.split_off(header_rows.min(rows.len()));
    head.extend(rows);
    let col_widths = column_widths(call, max_cols)?;
    let to_rows = |rows: Vec<Vec<Vec<Line>>>| -> Vec<crate::parser::table::Row> {
        rows.into_iter()
            .map(|cells| {
                let mut row: Vec<Option<crate::parser::table::Cell>> =
                    cells.into_iter().map(|c| Some((0, 0, 0, c))).collect();
                while row.len() < max_cols {
                    row.push(Some((0, 0, 0, Vec::new())));
                }
                row
            })
            .collect()
    };
    let data = TableData {
        colspecs: col_widths.iter().map(|&w| w as usize).collect(),
        head: to_rows(head),
        body: to_rows(body),
    };
    let mut table = m.build_table(doc, data, call.content_offset, stub_columns, widths(call));
    table.span = call.span;
    finish(doc, call, &mut table, title, false);
    let mut out = vec![Node::from(table)];
    out.extend(messages.into_iter().map(Node::from));
    Ok(out)
}

/// The rows of a two-level bullet list, checked.
fn list_rows(call: &Call, children: Vec<Node>) -> Result<Vec<Vec<Vec<Node>>>, Failure> {
    let problem = |detail: String| {
        failure(
            ERROR,
            format!(
                "Error parsing content block for the \"{}\" directive: {detail}",
                call.name
            ),
        )
    };
    let list = match children.as_slice() {
        [Node::Element(l)] if l.kind == Kind::BulletList => l.clone(),
        _ => return Err(problem("exactly one bullet list expected.".to_owned())),
    };
    let mut rows = Vec::new();
    let mut num_cols = 0;
    for (index, item) in list.elements().enumerate() {
        let inner = match item.children.as_slice() {
            [Node::Element(l)] if l.kind == Kind::BulletList => l,
            _ => {
                return Err(problem(format!(
                    "two-level bullet list expected, but row {} does not contain a second-level bullet list.",
                    index + 1
                )));
            }
        };
        let cells: Vec<Vec<Node>> = inner.elements().map(|c| c.children.clone()).collect();
        if index == 0 {
            num_cols = cells.len();
        } else if cells.len() != num_cols {
            return Err(problem(format!(
                "uniform two-level bullet list expected, but row {} does not contain the same number \
                 of items as row 1 ({} vs {num_cols}).",
                index + 1,
                cells.len()
            )));
        }
        rows.push(cells);
    }
    Ok(rows)
}

/// `list-table`: a table from a two-level bullet list.
pub(super) fn list_table(
    m: &mut Machine,
    doc: &mut Document,
    call: &mut Call,
) -> Result<Vec<Node>, Failure> {
    if call.content.is_empty() {
        return Err(failure(
            ERROR,
            format!(
                "The \"{}\" directive is empty; content required.",
                call.name
            ),
        ));
    }
    let (title, messages) = make_title(doc, call);
    let children = super::standard::parse_into_container(m, doc, call);
    let rows = list_rows(call, children)?;
    let num_cols = rows.first().map_or(0, Vec::len);
    let col_widths = column_widths(call, num_cols)?;
    let header_rows = int_option(call, "header-rows");
    let mut stub_columns = int_option(call, "stub-columns");
    let lens: Vec<usize> = rows.iter().map(Vec::len).collect();
    check_dimensions(call, &lens, header_rows, stub_columns)?;
    let mut table = Element::new(Kind::Table);
    table.span = call.span;
    widths(call).classes(&mut table);
    let mut tgroup = Element::new(Kind::Tgroup).with_attr("cols", col_widths.len() as i64);
    for w in &col_widths {
        let mut colspec = Element::new(Kind::Colspec).with_attr("colwidth", *w);
        if stub_columns > 0 {
            colspec.attributes.set("stub", true);
            stub_columns -= 1;
        }
        tgroup.children.push(colspec.into());
    }
    let mut row_nodes: Vec<Node> = rows
        .into_iter()
        .map(|cells| {
            let mut row = Element::new(Kind::Row);
            for cell in cells {
                row.children
                    .push(Element::new(Kind::Entry).with_children(cell).into());
            }
            row.into()
        })
        .collect();
    let body = row_nodes.split_off(header_rows.min(row_nodes.len()));
    if header_rows > 0 {
        tgroup
            .children
            .push(Element::new(Kind::Thead).with_children(row_nodes).into());
    }
    tgroup
        .children
        .push(Element::new(Kind::Tbody).with_children(body).into());
    table.children.push(tgroup.into());
    finish(doc, call, &mut table, title, true);
    let mut out = vec![Node::from(table)];
    out.extend(messages.into_iter().map(Node::from));
    Ok(out)
}
