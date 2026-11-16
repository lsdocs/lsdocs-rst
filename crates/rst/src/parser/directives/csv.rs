//! Python's `csv.reader` (CPython's `_csv.c` state machine), for the
//! `csv-table` directive: the dialect docutils uses, `QUOTE_MINIMAL`, and
//! Python's error messages.

/// The reader's settings (a `csv.Dialect`).
#[derive(Debug, Clone)]
pub(crate) struct Dialect {
    pub(crate) delimiter: char,
    pub(crate) quotechar: char,
    pub(crate) escapechar: Option<char>,
    pub(crate) doublequote: bool,
    pub(crate) skipinitialspace: bool,
    pub(crate) strict: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum S {
    StartRecord,
    StartField,
    EscapedChar,
    InField,
    InQuotedField,
    EscapeInQuotedField,
    QuoteInQuotedField,
    EatCrnl,
    AfterEscapedCrnl,
}

/// A character, or the end of a line (CPython's `EOL`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum C {
    Char(char),
    Eol,
}

/// A parsed row: its fields and the index of the line it started on.
pub(crate) type Row = (Vec<String>, usize);

struct Reader<'a> {
    d: &'a Dialect,
    state: S,
    field: String,
    fields: Vec<String>,
}

impl Reader<'_> {
    fn save(&mut self) {
        self.fields.push(std::mem::take(&mut self.field));
    }

    fn process(&mut self, c: C) -> Result<(), String> {
        let d = self.d;
        let is_nl = |c: C| matches!(c, C::Char('\n' | '\r'));
        let is = |c: C, x: char| c == C::Char(x);
        let is_escape = |c: C| d.escapechar.is_some_and(|e| c == C::Char(e));
        let mut state = self.state;
        loop {
            match state {
                S::StartRecord => {
                    if c == C::Eol {
                        return Ok(());
                    } else if is_nl(c) {
                        self.state = S::EatCrnl;
                        return Ok(());
                    }
                    state = S::StartField;
                    continue;
                }
                S::StartField => {
                    if is_nl(c) || c == C::Eol {
                        self.save();
                        self.state = if c == C::Eol {
                            S::StartRecord
                        } else {
                            S::EatCrnl
                        };
                    } else if is(c, d.quotechar) {
                        self.state = S::InQuotedField;
                    } else if is_escape(c) {
                        self.state = S::EscapedChar;
                    } else if is(c, ' ') && d.skipinitialspace {
                        self.state = S::StartField;
                    } else if is(c, d.delimiter) {
                        self.save();
                        self.state = S::StartField;
                    } else if let C::Char(ch) = c {
                        self.field.push(ch);
                        self.state = S::InField;
                    }
                }
                S::EscapedChar => {
                    if is_nl(c) {
                        if let C::Char(ch) = c {
                            self.field.push(ch);
                        }
                        self.state = S::AfterEscapedCrnl;
                        return Ok(());
                    }
                    self.field.push(match c {
                        C::Char(ch) => ch,
                        C::Eol => '\n',
                    });
                    self.state = S::InField;
                }
                S::AfterEscapedCrnl => {
                    if c == C::Eol {
                        return Ok(());
                    }
                    state = S::InField;
                    continue;
                }
                S::InField => {
                    if is_nl(c) || c == C::Eol {
                        self.save();
                        self.state = if c == C::Eol {
                            S::StartRecord
                        } else {
                            S::EatCrnl
                        };
                    } else if is_escape(c) {
                        self.state = S::EscapedChar;
                    } else if is(c, d.delimiter) {
                        self.save();
                        self.state = S::StartField;
                    } else if let C::Char(ch) = c {
                        self.field.push(ch);
                        self.state = S::InField;
                    }
                }
                S::InQuotedField => {
                    if c == C::Eol {
                    } else if is_escape(c) {
                        self.state = S::EscapeInQuotedField;
                    } else if is(c, d.quotechar) {
                        self.state = if d.doublequote {
                            S::QuoteInQuotedField
                        } else {
                            S::InField
                        };
                    } else if let C::Char(ch) = c {
                        self.field.push(ch);
                    }
                }
                S::EscapeInQuotedField => {
                    self.field.push(match c {
                        C::Char(ch) => ch,
                        C::Eol => '\n',
                    });
                    self.state = S::InQuotedField;
                }
                S::QuoteInQuotedField => {
                    if is(c, d.quotechar) {
                        self.field.push(d.quotechar);
                        self.state = S::InQuotedField;
                    } else if is(c, d.delimiter) {
                        self.save();
                        self.state = S::StartField;
                    } else if is_nl(c) || c == C::Eol {
                        self.save();
                        self.state = if c == C::Eol {
                            S::StartRecord
                        } else {
                            S::EatCrnl
                        };
                    } else if !d.strict {
                        if let C::Char(ch) = c {
                            self.field.push(ch);
                        }
                        self.state = S::InField;
                    } else {
                        return Err(format!(
                            "'{}' expected after '{}'",
                            d.delimiter, d.quotechar
                        ));
                    }
                }
                S::EatCrnl => {
                    if is_nl(c) {
                    } else if c == C::Eol {
                        self.state = S::StartRecord;
                    } else {
                        return Err(
                            "new-line character seen in unquoted field - do you need to \
                                    open the file with newline=''?"
                                .to_owned(),
                        );
                    }
                }
            }
            return Ok(());
        }
    }
}

/// Reads `lines` (each given a trailing line break, as docutils does) into
/// rows, or Python's `csv.Error` message.
pub(crate) fn read(lines: &[String], dialect: &Dialect) -> Result<Vec<Row>, String> {
    let mut r = Reader {
        d: dialect,
        state: S::StartRecord,
        field: String::new(),
        fields: Vec::new(),
    };
    let mut rows = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let start = i;
        r.fields.clear();
        r.field.clear();
        r.state = S::StartRecord;
        loop {
            let Some(line) = lines.get(i) else {
                if !r.field.is_empty() || r.state == S::InQuotedField {
                    if dialect.strict {
                        return Err("unexpected end of data".to_owned());
                    }
                    r.save();
                    break;
                }
                return Ok(rows);
            };
            i += 1;
            for ch in line.chars().chain(['\n']) {
                r.process(C::Char(ch))?;
            }
            r.process(C::Eol)?;
            if r.state == S::StartRecord {
                break;
            }
        }
        rows.push((std::mem::take(&mut r.fields), start));
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn docutils() -> Dialect {
        Dialect {
            delimiter: ',',
            quotechar: '"',
            escapechar: None,
            doublequote: true,
            skipinitialspace: true,
            strict: true,
        }
    }

    fn rows(lines: &[&str]) -> Result<Vec<Vec<String>>, String> {
        let lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        read(&lines, &docutils()).map(|r| r.into_iter().map(|(f, _)| f).collect())
    }

    #[test]
    fn reads_like_python() {
        assert_eq!(rows(&["a, b,c"]).unwrap(), [["a", "b", "c"]]);
        assert_eq!(
            rows(&["\"x, y\", \"say \"\"hi\"\"\""]).unwrap(),
            [["x, y", "say \"hi\""]]
        );
        assert_eq!(
            rows(&["\"multi", "line\", z"]).unwrap(),
            [["multi\nline", "z"]]
        );
        assert_eq!(
            rows(&["a", "", "b"]).unwrap(),
            vec![vec!["a".to_string()], vec![], vec!["b".to_string()]]
        );
        assert_eq!(rows(&["\"a\"b"]), Err("',' expected after '\"'".to_owned()));
        assert_eq!(rows(&["\"open"]), Err("unexpected end of data".to_owned()));
    }
}
