//! Grid and simple tables: a port of docutils' `tableparser` and the
//! table methods of its `Body` state.
//!
//! As in docutils, East Asian wide characters are padded with a null so
//! that one character is one column; combining characters take no column.

use std::collections::BTreeMap;

use super::document::{At, Document, ERROR, WARNING, fixed};
use super::machine::{Machine, State, Step, span_of};
use crate::lines::Line;
use crate::lines::is_python_whitespace;
use crate::nodes::{Element, Kind, Span};
use crate::util::{is_combining, is_wide};

/// The padding after a wide character.
const PAD: char = '\0';

/// A table line: its characters (wide ones padded) and where they came
/// from.
#[derive(Debug, Clone)]
struct TableLine {
    chars: Vec<char>,
    line: Line,
    /// Characters removed from the start of `line.text` before `chars`.
    lead: usize,
}

impl TableLine {
    fn new(line: &Line, strip: bool) -> Self {
        let mut text: Vec<char> = line.text.chars().collect();
        let mut lead = 0;
        if strip {
            lead = text
                .iter()
                .take_while(|&&c| is_python_whitespace(c))
                .count();
            text.drain(..lead);
            while text.last().is_some_and(|&c| is_python_whitespace(c)) {
                text.pop();
            }
        }
        let mut chars = Vec::with_capacity(text.len());
        for c in text {
            chars.push(c);
            if is_wide(c) {
                chars.push(PAD);
            }
        }
        Self {
            chars,
            line: line.clone(),
            lead,
        }
    }

    /// The text without padding.
    fn text(&self) -> String {
        self.chars.iter().filter(|&&c| c != PAD).collect()
    }

    /// The characters without combining characters.
    fn stripped(&self) -> Vec<char> {
        self.chars
            .iter()
            .copied()
            .filter(|&c| !is_combining(c))
            .collect()
    }

    /// The byte offset in the source of padded position `p`.
    fn offset(&self, p: usize) -> usize {
        let index = self.lead
            + self.chars[..p.min(self.chars.len())]
                .iter()
                .filter(|&&c| c != PAD)
                .count();
        let byte = self
            .line
            .text
            .char_indices()
            .nth(index)
            .map_or(self.line.text.len(), |(b, _)| b);
        (self.line.span.start + byte).min(self.line.span.end)
    }

    /// Positions `a..b` as a line of text, padding removed.
    fn sub(&self, a: usize, b: usize) -> Line {
        let (a, b) = (a.min(self.chars.len()), b.min(self.chars.len()));
        let text: String = self.chars[a..b.max(a)]
            .iter()
            .filter(|&&c| c != PAD)
            .collect();
        let start = self.offset(a);
        Line {
            span: Span::new(
                start,
                (start + text.len()).min(self.line.span.end.max(start)),
            ),
            text,
        }
    }
}

/// A problem with a table's markup, and the table line it is on.
struct TableError {
    message: String,
    offset: usize,
}

fn table_error(message: impl Into<String>, offset: usize) -> TableError {
    TableError {
        message: message.into(),
        offset,
    }
}

/// A cell: extra rows and columns it spans, the line its text starts on
/// (relative to the table), and its text.
type Cell = (usize, usize, usize, Vec<Line>);
type Row = Vec<Option<Cell>>;

/// Column widths, head rows and body rows.
struct TableData {
    colspecs: Vec<usize>,
    head: Vec<Row>,
    body: Vec<Row>,
}

/// docutils' `StringList.get_2D_block`: lines `top..bottom`, columns
/// `left..right`, with the common indentation removed.
fn get_2d_block(
    lines: &[TableLine],
    top: usize,
    left: usize,
    bottom: usize,
    right: usize,
) -> Vec<Line> {
    let mut cut: Vec<(usize, usize, usize)> = Vec::new(); // line, start, end
    let mut indent = right;
    for (i, tl) in lines.iter().enumerate().take(bottom).skip(top) {
        let columns: Vec<usize> = tl
            .chars
            .iter()
            .enumerate()
            .filter(|&(_, &c)| !is_combining(c))
            .map(|(i, _)| i)
            .collect();
        let left_i = columns
            .get(left)
            .copied()
            .unwrap_or(left)
            .min(tl.chars.len());
        let right_i = columns
            .get(right)
            .copied()
            .unwrap_or(tl.chars.len())
            .min(tl.chars.len());
        let mut end = right_i.max(left_i);
        while end > left_i && is_python_whitespace(tl.chars[end - 1]) {
            end -= 1;
        }
        if end > left_i {
            let lead = tl.chars[left_i..end]
                .iter()
                .take_while(|&&c| is_python_whitespace(c))
                .count();
            indent = indent.min(lead);
        }
        cut.push((i, left_i, end));
    }
    let strip = if 0 < indent && indent < right {
        indent
    } else {
        0
    };
    cut.into_iter()
        .map(|(i, a, b)| {
            let a = (a + strip).min(b);
            lines[i].sub(a, b)
        })
        .collect()
}

fn merge(master: &mut BTreeMap<usize, Vec<usize>>, new: BTreeMap<usize, Vec<usize>>) {
    for (k, v) in new {
        master.entry(k).or_default().extend(v);
    }
}

/// `\+-[-+]+-\+ *$`
pub(crate) fn is_grid_top(s: &[char]) -> bool {
    let n = s.iter().rposition(|&c| c != ' ').map_or(0, |i| i + 1);
    let s = &s[..n];
    s.len() >= 5
        && s[0] == '+'
        && s[1] == '-'
        && s[n - 2] == '-'
        && s[n - 1] == '+'
        && s.iter().all(|&c| c == '+' || c == '-')
}

/// `\+=[=+]+=\+ *$`
fn is_grid_head_sep(s: &[char]) -> bool {
    let n = s.iter().rposition(|&c| c != ' ').map_or(0, |i| i + 1);
    let s = &s[..n];
    s.len() >= 5
        && s[0] == '+'
        && s[1] == '='
        && s[n - 2] == '='
        && s[n - 1] == '+'
        && s.iter().all(|&c| c == '+' || c == '=')
}

/// `=+( +=+)+ *$`
pub(crate) fn is_simple_top(s: &[char]) -> bool {
    let n = s.iter().rposition(|&c| c != ' ').map_or(0, |i| i + 1);
    let s = &s[..n];
    if s.first() != Some(&'=') || !s.iter().all(|&c| c == '=' || c == ' ') {
        return false;
    }
    s.contains(&' ')
}

/// `=+[ =]*$`
fn is_simple_border(s: &[char]) -> bool {
    s.first() == Some(&'=') && s.iter().all(|&c| c == '=' || c == ' ')
}

/// `-[ -]*$`
fn is_span_line(s: &[char]) -> bool {
    s.first() == Some(&'-') && s.iter().all(|&c| c == '-' || c == ' ')
}

/// Python's `str.find(c, start)` on characters.
fn find(s: &[char], c: char, start: isize) -> isize {
    let len = s.len() as isize;
    let start = if start < 0 {
        (start + len).max(0)
    } else {
        start
    };
    (start..len).find(|&i| s[i as usize] == c).unwrap_or(-1)
}

/// The grid table parser (docutils' `GridTableParser`).
struct GridTable {
    block: Vec<TableLine>,
    stripped: Vec<Vec<char>>,
    bottom: usize,
    right: usize,
    head_body_sep: Option<usize>,
    done: Vec<isize>,
    cells: Vec<(usize, usize, usize, usize, Vec<Line>)>,
    rowseps: BTreeMap<usize, Vec<usize>>,
    colseps: BTreeMap<usize, Vec<usize>>,
}

impl GridTable {
    fn parse(block: Vec<TableLine>) -> Result<TableData, TableError> {
        let width = block[0].chars.len();
        let mut t = GridTable {
            bottom: block.len() - 1,
            right: width.saturating_sub(1),
            stripped: Vec::new(),
            head_body_sep: None,
            done: vec![-1; width],
            cells: Vec::new(),
            rowseps: BTreeMap::from([(0, vec![0])]),
            colseps: BTreeMap::from([(0, vec![0])]),
            block,
        };
        t.find_head_body_sep()?;
        t.parse_table()?;
        Ok(t.structure_from_cells())
    }

    fn find_head_body_sep(&mut self) -> Result<(), TableError> {
        let mut last = 0;
        for i in 0..self.block.len() {
            last = i;
            if is_grid_head_sep(&self.block[i].chars) {
                if let Some(sep) = self.head_body_sep.filter(|&s| s != 0) {
                    return Err(table_error(
                        format!(
                            "Multiple head/body row separators (table lines {} and {}); \
                             only one allowed.",
                            sep + 1,
                            i + 1
                        ),
                        i,
                    ));
                }
                self.head_body_sep = Some(i);
                for c in &mut self.block[i].chars {
                    if *c == '=' {
                        *c = '-';
                    }
                }
            }
        }
        if self.head_body_sep == Some(0) || self.head_body_sep == Some(self.block.len() - 1) {
            return Err(table_error(
                "The head/body row separator may not be the first or last line of the table.",
                last,
            ));
        }
        Ok(())
    }

    fn at(&self, row: usize, col: usize) -> char {
        self.stripped[row].get(col).copied().unwrap_or(' ')
    }

    fn parse_table(&mut self) -> Result<(), TableError> {
        self.stripped = self.block.iter().map(TableLine::stripped).collect();
        let mut corners = vec![(0usize, 0usize)];
        while !corners.is_empty() {
            let (top, left) = corners.remove(0);
            if top == self.bottom || left == self.right || top as isize <= self.done[left] {
                continue;
            }
            let Some((bottom, right, rowseps, colseps)) = self.scan_right(top, left) else {
                continue;
            };
            merge(&mut self.rowseps, rowseps);
            merge(&mut self.colseps, colseps);
            for done in &mut self.done[left..right] {
                *done = bottom as isize - 1;
            }
            let cell = get_2d_block(&self.block, top + 1, left + 1, bottom, right);
            self.cells.push((top, left, bottom, right, cell));
            corners.extend([(top, right), (bottom, left)]);
            corners.sort();
        }
        let last = self.bottom as isize - 1;
        if (0..self.right).any(|col| self.done[col] != last) {
            return Err(table_error("Malformed table; parse incomplete.", 0));
        }
        Ok(())
    }

    #[allow(clippy::type_complexity)]
    fn scan_right(
        &self,
        top: usize,
        left: usize,
    ) -> Option<(
        usize,
        usize,
        BTreeMap<usize, Vec<usize>>,
        BTreeMap<usize, Vec<usize>>,
    )> {
        let mut colseps = BTreeMap::new();
        for i in left + 1..=self.right {
            match self.at(top, i) {
                '+' => {
                    colseps.insert(i, vec![top]);
                    if let Some((bottom, rowseps, newcolseps)) = self.scan_down(top, left, i) {
                        merge(&mut colseps, newcolseps);
                        return Some((bottom, i, rowseps, colseps));
                    }
                }
                '-' => {}
                _ => return None,
            }
        }
        None
    }

    #[allow(clippy::type_complexity)]
    fn scan_down(
        &self,
        top: usize,
        left: usize,
        right: usize,
    ) -> Option<(
        usize,
        BTreeMap<usize, Vec<usize>>,
        BTreeMap<usize, Vec<usize>>,
    )> {
        let mut rowseps = BTreeMap::new();
        for i in top + 1..=self.bottom {
            match self.at(i, right) {
                '+' => {
                    rowseps.insert(i, vec![right]);
                    if let Some((newrowseps, colseps)) = self.scan_left(top, left, i, right) {
                        merge(&mut rowseps, newrowseps);
                        return Some((i, rowseps, colseps));
                    }
                }
                '|' => {}
                _ => return None,
            }
        }
        None
    }

    #[allow(clippy::type_complexity)]
    fn scan_left(
        &self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
    ) -> Option<(BTreeMap<usize, Vec<usize>>, BTreeMap<usize, Vec<usize>>)> {
        let mut colseps = BTreeMap::new();
        for i in (left + 1..right).rev() {
            match self.at(bottom, i) {
                '+' => {
                    colseps.insert(i, vec![bottom]);
                }
                '-' => {}
                _ => return None,
            }
        }
        if self.at(bottom, left) != '+' {
            return None;
        }
        let rowseps = self.scan_up(top, left, bottom)?;
        Some((rowseps, colseps))
    }

    fn scan_up(
        &self,
        top: usize,
        left: usize,
        bottom: usize,
    ) -> Option<BTreeMap<usize, Vec<usize>>> {
        let mut rowseps = BTreeMap::new();
        for i in (top + 1..bottom).rev() {
            match self.at(i, left) {
                '+' => {
                    rowseps.insert(i, vec![left]);
                }
                '|' => {}
                _ => return None,
            }
        }
        Some(rowseps)
    }

    fn structure_from_cells(self) -> TableData {
        let rowseps: Vec<usize> = self.rowseps.keys().copied().collect();
        let colseps: Vec<usize> = self.colseps.keys().copied().collect();
        let row_index = |r: usize| rowseps.iter().position(|&x| x == r).unwrap_or(0);
        let col_index = |c: usize| colseps.iter().position(|&x| x == c).unwrap_or(0);
        let colspecs: Vec<usize> = colseps.windows(2).map(|w| w[1] - w[0] - 1).collect();
        let mut rows: Vec<Row> =
            vec![vec![None; colseps.len().saturating_sub(1)]; rowseps.len().saturating_sub(1)];
        for (top, left, bottom, right, block) in self.cells {
            let (rownum, colnum) = (row_index(top), col_index(left));
            let morerows = row_index(bottom) - rownum - 1;
            let morecols = col_index(right) - colnum - 1;
            if let Some(slot) = rows.get_mut(rownum).and_then(|r| r.get_mut(colnum)) {
                *slot = Some((morerows, morecols, top + 1, block));
            }
        }
        let (head, body) = match self.head_body_sep {
            Some(sep) if sep != 0 => {
                let n = row_index(sep);
                let body = rows.split_off(n.min(rows.len()));
                (rows, body)
            }
            _ => (Vec::new(), rows),
        };
        TableData {
            colspecs,
            head,
            body,
        }
    }
}

/// The simple table parser (docutils' `SimpleTableParser`).
struct SimpleTable {
    block: Vec<TableLine>,
    head_body_sep: Option<usize>,
    columns: Vec<(usize, usize)>,
    border_end: usize,
    table: Vec<Vec<Cell>>,
}

impl SimpleTable {
    fn parse(mut block: Vec<TableLine>) -> Result<TableData, TableError> {
        let last = block.len() - 1;
        for i in [0, last] {
            for c in &mut block[i].chars {
                if *c == '=' {
                    *c = '-';
                }
            }
        }
        let mut t = SimpleTable {
            block,
            head_body_sep: None,
            columns: Vec::new(),
            border_end: 0,
            table: Vec::new(),
        };
        t.find_head_body_sep()?;
        t.parse_table()?;
        Ok(t.structure_from_cells())
    }

    fn find_head_body_sep(&mut self) -> Result<(), TableError> {
        let mut last = 0;
        for i in 0..self.block.len() {
            last = i;
            if is_simple_border(&self.block[i].chars) {
                if let Some(sep) = self.head_body_sep.filter(|&s| s != 0) {
                    return Err(table_error(
                        format!(
                            "Multiple head/body row separators (table lines {} and {}); \
                             only one allowed.",
                            sep + 1,
                            i + 1
                        ),
                        i,
                    ));
                }
                self.head_body_sep = Some(i);
                for c in &mut self.block[i].chars {
                    if *c == '=' {
                        *c = '-';
                    }
                }
            }
        }
        if self.head_body_sep == Some(0) || self.head_body_sep == Some(self.block.len() - 1) {
            return Err(table_error(
                "The head/body row separator may not be the first or last line of the table.",
                last,
            ));
        }
        Ok(())
    }

    fn parse_table(&mut self) -> Result<(), TableError> {
        let top = self.block[0].chars.clone();
        self.columns = self.parse_columns(&top, 0)?;
        self.border_end = self.columns.last().map_or(0, |c| c.1);
        let (first_start, first_end) = self.columns[0];
        let mut offset = 1;
        let mut start = 1;
        let mut text_found = false;
        while offset < self.block.len() {
            let line = self.block[offset].chars.clone();
            if is_span_line(&line) {
                let mut span = line.clone();
                while span.last().is_some_and(|&c| is_python_whitespace(c)) {
                    span.pop();
                }
                self.parse_row(start, offset, Some((span, offset)))?;
                start = offset + 1;
                text_found = false;
            } else if !line
                .get(first_start.min(line.len())..first_end.min(line.len()))
                .unwrap_or(&[])
                .iter()
                .all(|&c| is_python_whitespace(c))
            {
                if text_found && offset != start {
                    self.parse_row(start, offset, None)?;
                }
                start = offset;
                text_found = true;
            } else if !text_found {
                start = offset + 1;
            }
            offset += 1;
        }
        Ok(())
    }

    fn parse_columns(
        &self,
        line: &[char],
        offset: usize,
    ) -> Result<Vec<(usize, usize)>, TableError> {
        let mut cols = Vec::new();
        let mut end: isize = 0;
        loop {
            let begin = find(line, '-', end);
            end = find(line, ' ', begin);
            if begin < 0 {
                break;
            }
            if end < 0 {
                end = line.len() as isize;
            }
            cols.push((begin as usize, end as usize));
        }
        if !self.columns.is_empty() {
            if cols.last().map(|c| c.1) != Some(self.border_end) {
                return Err(table_error(
                    format!("Column span incomplete in table line {}.", offset + 1),
                    offset,
                ));
            }
            let last = cols.len() - 1;
            cols[last].1 = self.columns[self.columns.len() - 1].1;
        }
        Ok(cols)
    }

    fn init_row(&self, colspec: &[(usize, usize)], offset: usize) -> Result<Vec<Cell>, TableError> {
        let mut i = 0;
        let mut cells = Vec::new();
        let problem = || {
            table_error(
                format!(
                    "Column span alignment problem in table line {}.",
                    offset + 2
                ),
                offset + 1,
            )
        };
        for &(start, end) in colspec {
            let mut morecols = 0;
            if self.columns.get(i).map(|c| c.0) != Some(start) {
                return Err(problem());
            }
            while self.columns.get(i).ok_or_else(problem)?.1 != end {
                i += 1;
                morecols += 1;
            }
            cells.push((0, morecols, offset, Vec::new()));
            i += 1;
        }
        Ok(cells)
    }

    /// Lines `start..stop` of the block form a row.
    fn parse_row(
        &mut self,
        start: usize,
        stop: usize,
        spanline: Option<(Vec<char>, usize)>,
    ) -> Result<(), TableError> {
        if start >= stop && spanline.is_none() {
            return Ok(());
        }
        let mut columns = match &spanline {
            Some((line, offset)) => self.parse_columns(line, *offset)?,
            None => self.columns.clone(),
        };
        let lines: Vec<TableLine> = self.block[start..stop].to_vec();
        self.check_columns(&lines, start, &mut columns)?;
        let mut row = self.init_row(&columns, start)?;
        for (i, &(a, b)) in columns.iter().enumerate() {
            row[i].3 = get_2d_block(&lines, 0, a, lines.len(), b);
        }
        self.table.push(row);
        Ok(())
    }

    fn check_columns(
        &mut self,
        lines: &[TableLine],
        first_line: usize,
        columns: &mut Vec<(usize, usize)>,
    ) -> Result<(), TableError> {
        columns.push((usize::MAX, 0));
        let last_col = columns.len() - 2;
        let lines: Vec<Vec<char>> = lines.iter().map(TableLine::stripped).collect();
        for i in 0..columns.len() - 1 {
            let (start, end) = columns[i];
            let next_start = columns[i + 1].0;
            for (offset, line) in lines.iter().enumerate() {
                let tail = |a: usize, b: usize| -> bool {
                    let (a, b) = (a.min(line.len()), b.min(line.len()));
                    line[a..b.max(a)].iter().any(|&c| !is_python_whitespace(c))
                };
                if i == last_col && tail(end, usize::MAX) {
                    let mut text_end = line.len();
                    while text_end > start && is_python_whitespace(line[text_end - 1]) {
                        text_end -= 1;
                    }
                    let new_end = start + text_end.saturating_sub(start.min(text_end));
                    let (main_start, main_end) = self.columns[self.columns.len() - 1];
                    columns[i] = (start, main_end.max(new_end));
                    if new_end > main_end {
                        let n = self.columns.len() - 1;
                        self.columns[n] = (main_start, new_end);
                    }
                } else if tail(end, next_start) {
                    return Err(table_error(
                        format!(
                            "Text in column margin in table line {}.",
                            first_line + offset + 1
                        ),
                        first_line + offset,
                    ));
                }
            }
        }
        columns.pop();
        Ok(())
    }

    fn structure_from_cells(self) -> TableData {
        let colspecs = self.columns.iter().map(|(s, e)| e - s).collect();
        let mut first_body_row = 0;
        if let Some(sep) = self.head_body_sep.filter(|&s| s != 0)
            && let Some(i) = self.table.iter().position(|row| row[0].2 > sep)
        {
            first_body_row = i;
        }
        let rows: Vec<Row> = self
            .table
            .into_iter()
            .map(|row| row.into_iter().map(Some).collect())
            .collect();
        let mut head = rows;
        let body = head.split_off(first_body_row.min(head.len()));
        TableData {
            colspecs,
            head,
            body,
        }
    }
}

impl Machine {
    /// A table, grid or simple, starting at the current line.
    pub(crate) fn table_top(&mut self, doc: &mut Document, state: State, grid: bool) -> Step {
        let (block, mut messages, blank_finish) = if grid {
            self.isolate_grid_table(doc)
        } else {
            self.isolate_simple_table(doc)
        };
        let mut nodes = Vec::new();
        if !block.is_empty() {
            let parsed = if grid {
                GridTable::parse(block.clone())
            } else {
                SimpleTable::parse(block.clone())
            };
            match parsed {
                Ok(data) => {
                    let tableline = self.abs_line_number() as isize + 1 - block.len() as isize;
                    nodes.push(self.build_table(doc, data, tableline));
                }
                Err(e) => {
                    nodes.extend(self.malformed_table(doc, &block, &e.message, e.offset));
                }
            }
        }
        nodes.append(&mut messages);
        doc.extend(nodes);
        if !blank_finish {
            let w = doc.message(
                WARNING,
                "Blank line required after table.",
                Vec::new(),
                At::Top(Some(self.abs_line_number() + 1)),
            );
            doc.append(w);
        }
        Ok(state)
    }

    /// An error for a malformed table, with its text.
    fn malformed_table(
        &self,
        doc: &Document,
        block: &[TableLine],
        detail: &str,
        offset: usize,
    ) -> Vec<Element> {
        let lines: Vec<Line> = block
            .iter()
            .map(|tl| Line {
                text: tl.text(),
                span: tl.line.span,
            })
            .collect();
        let data = super::machine::join(&lines);
        let start = self.abs_line_number() as isize + 1 - block.len() as isize;
        let mut message = "Malformed table.".to_owned();
        if !detail.is_empty() {
            message.push('\n');
            message.push_str(detail);
        }
        vec![doc.message(
            ERROR,
            &message,
            vec![fixed(Kind::LiteralBlock, &data, span_of(&lines)).into()],
            At::TopSigned(start + offset as isize),
        )]
    }

    fn isolate_grid_table(&mut self, doc: &mut Document) -> (Vec<TableLine>, Vec<Element>, bool) {
        let mut messages = Vec::new();
        let mut blank_finish = true;
        let lines = match self.get_text_block(doc, true) {
            Ok(b) => b,
            Err((b, line)) => {
                messages.push(doc.message(
                    ERROR,
                    "Unexpected indentation.",
                    Vec::new(),
                    At::Exact(Some(line)),
                ));
                blank_finish = false;
                b
            }
        };
        let mut block: Vec<TableLine> = lines.iter().map(|l| TableLine::new(l, false)).collect();
        let width = TableLine::new(&lines[0], true).chars.len();
        let mut i = 0;
        let mut cut = None;
        for (k, line) in lines.iter().enumerate() {
            i = k;
            block[k] = TableLine::new(line, true);
            if !matches!(block[k].chars.first(), Some('+' | '|')) {
                cut = Some(k);
                break;
            }
        }
        if let Some(k) = cut {
            blank_finish = false;
            self.previous_line(doc, (block.len() - k) as isize);
            block.truncate(k);
        }
        if !block.last().is_some_and(|l| is_grid_top(&l.chars)) {
            let found = (2..block.len().saturating_sub(1))
                .rev()
                .find(|&k| is_grid_top(&block[k].chars));
            match found {
                Some(k) => {
                    self.previous_line(doc, (block.len() - k + 1) as isize);
                    block.truncate(k + 1);
                    blank_finish = false;
                }
                None => {
                    if block.len() > 3 {
                        i = 2;
                    }
                    messages.extend(self.malformed_table(
                        doc,
                        &block,
                        "Bottom border missing or corrupt.",
                        i,
                    ));
                    return (Vec::new(), messages, blank_finish);
                }
            }
        }
        for (k, line) in block.iter().enumerate() {
            let stripped = line.stripped();
            if stripped.len() != width || !matches!(line.chars.last(), Some('+' | '|')) {
                messages.extend(self.malformed_table(
                    doc,
                    &block,
                    "Right border not aligned or missing.",
                    k,
                ));
                return (Vec::new(), messages, blank_finish);
            }
        }
        (block, messages, blank_finish)
    }

    fn isolate_simple_table(&mut self, doc: &mut Document) -> (Vec<TableLine>, Vec<Element>, bool) {
        let start = self.line_offset;
        let line_at = |m: &Machine, i: isize| m.get(i).cloned().unwrap_or_default();
        let limit = self.lines_len() - 1;
        let toplen = TableLine::new(&line_at(self, start), true).chars.len();
        let mut found = 0;
        let mut found_at = None;
        let mut i = start + 1;
        let next_blank = |m: &Machine, i: isize| {
            i == limit || m.get(i + 1).is_none_or(|l| l.text.trim().is_empty())
        };
        let mut end = None;
        while i <= limit {
            let line = line_at(self, i);
            let chars: Vec<char> = line.text.chars().collect();
            if is_simple_border(&chars) {
                if TableLine::new(&line, true).chars.len() != toplen {
                    self.next_line(doc, i - start);
                    let block: Vec<TableLine> = (start..=i)
                        .map(|k| TableLine::new(&line_at(self, k), false))
                        .collect();
                    let messages = self.malformed_table(
                        doc,
                        &block,
                        "Bottom border or header rule does not match top border.",
                        (i - start) as usize,
                    );
                    return (Vec::new(), messages, next_blank(self, i));
                }
                found += 1;
                found_at = Some(i);
                if found == 2 || next_blank(self, i) {
                    end = Some(i);
                    break;
                }
            }
            i += 1;
        }
        let Some(end) = end else {
            let mut details = "No bottom table border found".to_owned();
            let block: Vec<TableLine> = if let Some(at) = found_at {
                details.push_str(" or no blank line after table bottom");
                self.next_line(doc, at - start);
                (start..=at)
                    .map(|k| TableLine::new(&line_at(self, k), false))
                    .collect()
            } else {
                self.next_line(doc, i - start - 1);
                (start..=limit)
                    .map(|k| TableLine::new(&line_at(self, k), false))
                    .collect()
            };
            details.push('.');
            let messages = self.malformed_table(doc, &block, &details, 0);
            return (Vec::new(), messages, found == 0);
        };
        self.next_line(doc, end - start);
        let block = (start..=end)
            .map(|k| TableLine::new(&line_at(self, k), false))
            .collect();
        (block, Vec::new(), next_blank(self, end))
    }

    /// The table element from parsed table data.
    fn build_table(&mut self, doc: &mut Document, data: TableData, tableline: isize) -> Element {
        let mut tgroup = Element::new(Kind::Tgroup).with_attr("cols", data.colspecs.len() as i64);
        for width in &data.colspecs {
            tgroup.children.push(
                Element::new(Kind::Colspec)
                    .with_attr("colwidth", *width as i64)
                    .into(),
            );
        }
        doc.open(Element::new(Kind::Table), false);
        doc.open(tgroup, true);
        if !data.head.is_empty() {
            doc.open(Element::new(Kind::Thead), true);
            for row in data.head {
                self.build_table_row(doc, row, tableline);
            }
            doc.close();
        }
        doc.open(Element::new(Kind::Tbody), true);
        for row in data.body {
            self.build_table_row(doc, row, tableline);
        }
        doc.close();
        doc.close();
        doc.close().unwrap()
    }

    fn build_table_row(&mut self, doc: &mut Document, row: Row, tableline: isize) {
        doc.open(Element::new(Kind::Row), true);
        for (morerows, morecols, offset, block) in row.into_iter().flatten() {
            let mut entry = Element::new(Kind::Entry);
            if morerows > 0 {
                entry.attributes.set("morerows", morerows as i64);
            }
            if morecols > 0 {
                entry.attributes.set("morecols", morecols as i64);
            }
            entry.span = span_of(&block);
            doc.open(entry, true);
            if block.iter().any(|l| !l.text.is_empty()) {
                self.nested_parse(doc, block, tableline + offset as isize - 1);
            }
            doc.close();
        }
        doc.close();
    }
}
