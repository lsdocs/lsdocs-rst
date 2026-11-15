//! Turning input text into lines the way docutils does
//! (`docutils.statemachine.string2lines`): vertical tabs and form feeds
//! become spaces, text is split at every line boundary Python's
//! `str.splitlines` knows, tabs are expanded to 8-column stops, and trailing
//! whitespace is removed. Each line remembers where it came from.

use std::rc::Rc;

use crate::nodes::Span;

/// Tab stops are every 8 columns, as in docutils' default `tab_width`.
pub const TAB_WIDTH: usize = 8;

/// A line of input, prepared for parsing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Line {
    /// The text, tabs expanded and trailing whitespace removed.
    pub text: String,
    /// Where the line is in the input, without its line ending.
    pub span: Span,
    /// The file the line comes from (`None` for the document itself) and
    /// its offset there, as docutils' `StringList` items record them.
    pub(crate) origin: LineOrigin,
}

/// Where a line comes from: an included file (or the document itself), and
/// the line's offset in it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct LineOrigin {
    pub(crate) source: Option<Rc<str>>,
    pub(crate) offset: isize,
}

impl Line {
    /// Whether the line is blank.
    pub fn is_blank(&self) -> bool {
        self.text.is_empty()
    }

    /// The number of leading spaces.
    pub fn indent(&self) -> usize {
        self.text.len() - self.text.trim_start_matches(' ').len()
    }
}

/// Whether `c` ends a line for Python's `str.splitlines`.
fn is_line_boundary(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// Splits `text` into lines as Python's `str.splitlines` does (no line
/// endings; `\r\n` counts once; no empty last line for a trailing break).
pub fn split_lines(text: &str) -> impl Iterator<Item = &str> {
    split_with_offsets(text).map(|(_, line)| line)
}

/// [`split_lines`], with the byte offset of each line.
fn split_with_offsets(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut rest = 0;
    std::iter::from_fn(move || {
        if rest >= text.len() {
            return None;
        }
        let start = rest;
        let tail = &text[start..];
        match tail.char_indices().find(|&(_, c)| is_line_boundary(c)) {
            Some((i, c)) => {
                let mut next = start + i + c.len_utf8();
                if c == '\r' && text[next..].starts_with('\n') {
                    next += 1;
                }
                rest = next;
                Some((start, &tail[..i]))
            }
            None => {
                rest = text.len();
                Some((start, tail))
            }
        }
    })
}

/// Expands tabs to `width`-column stops, like Python's `str.expandtabs`
/// (which removes them for a width of 0).
pub(crate) fn expand_tabs_to(line: &str, width: usize) -> String {
    if !line.contains('\t') {
        return line.to_owned();
    }
    let mut out = String::with_capacity(line.len() + 8);
    let mut column = 0;
    for c in line.chars() {
        if c == '\t' {
            if width == 0 {
                continue;
            }
            let spaces = width - column % width;
            out.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else {
            out.push(c);
            column += 1;
        }
    }
    out
}

/// Whether Python's `str.rstrip()` removes `c`.
pub(crate) fn is_python_whitespace(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\x1c'..='\x1f')
}

/// The input as lines, ready for parsing.
pub fn lines(text: &str) -> Vec<Line> {
    lines_with(text, TAB_WIDTH, None, None)
}

/// The lines of `text` with tab stops every `tab_width` columns (0
/// removes tabs, as Python's `expandtabs(0)` does), from `source`. With `span`, every line gets that span instead of
/// its place in `text` (lines from an included file).
pub(crate) fn lines_with(
    text: &str,
    tab_width: usize,
    source: Option<Rc<str>>,
    span: Option<Span>,
) -> Vec<Line> {
    split_with_offsets(text)
        .enumerate()
        .map(|(i, (start, raw))| {
            let normalized: String = raw
                .chars()
                .map(|c| if matches!(c, '' | '') { ' ' } else { c })
                .collect();
            let text = expand_tabs_to(&normalized, tab_width)
                .trim_end_matches(is_python_whitespace)
                .to_owned();
            Line {
                text,
                span: span.unwrap_or(Span::new(start, start + raw.len())),
                origin: LineOrigin {
                    source: source.clone(),
                    offset: i as isize,
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_like_python() {
        let parts: Vec<_> = split_lines("a\r\nb\rc\n\nd\u{2028}e\n").collect();
        assert_eq!(parts, ["a", "b", "c", "", "d", "e"]);
        assert_eq!(split_lines("").count(), 0);
        assert_eq!(split_lines("x").collect::<Vec<_>>(), ["x"]);
    }

    #[test]
    fn prepares_lines() {
        let input = "a\tb  \n\x0cx\r\n  \t";
        let lines = lines(input);
        let texts: Vec<_> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["a       b", " x", ""]);
        assert_eq!(lines[1].span, Span::new(6, 8));
        assert_eq!(&input[lines[0].span.start..lines[0].span.end], "a\tb  ");
        assert_eq!(lines[1].indent(), 1);
        assert!(lines[2].is_blank());
    }
}
