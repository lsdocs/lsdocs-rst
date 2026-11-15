//! The line patterns of docutils' reStructuredText state machine, written
//! as matchers instead of regular expressions. Each works on a line's
//! characters and, like Python's `re.match`, matches at the start; offsets
//! are in characters. Comments give the docutils pattern.

use crate::lines::is_python_whitespace;

/// `( +|$)` at `p`: the end of the spaces, or `p` at the end of the line.
fn spaces_or_end(s: &[char], p: usize) -> Option<usize> {
    if p == s.len() {
        return Some(p);
    }
    let n = s[p..].iter().take_while(|&&c| c == ' ').count();
    (n > 0).then_some(p + n)
}

/// `(?![ ]|$)` at `p`.
fn not_space_or_end(s: &[char], p: usize) -> bool {
    p < s.len() && s[p] != ' '
}

/// `[!-/:-@[-`{-~]`: 7-bit ASCII punctuation.
pub(crate) fn is_punctuation(c: char) -> bool {
    c.is_ascii_punctuation()
}

/// `(?:(?!_)\w)`: a word character other than an underscore.
fn is_name_char(c: char) -> bool {
    c.is_alphanumeric()
}

/// The possible ends of a `simplename` starting at `p`, longest first:
/// `(?:(?!_)\w)+(?:[-._+:](?:(?!_)\w)+)*`.
pub(crate) fn simplename_ends(s: &[char], p: usize) -> Vec<usize> {
    let mut ends = Vec::new();
    let mut i = p;
    loop {
        let start = i;
        while i < s.len() && is_name_char(s[i]) {
            i += 1;
        }
        if i == start {
            break;
        }
        ends.push(i);
        if i < s.len() && "-._+:".contains(s[i]) {
            i += 1;
        } else {
            break;
        }
    }
    ends.reverse();
    ends
}

/// Whether `s` is entirely a `simplename`.
pub(crate) fn is_simplename(s: &[char]) -> bool {
    simplename_ends(s, 0).first() == Some(&s.len())
}

/// `' *$'`.
pub(crate) fn blank(s: &[char]) -> bool {
    s.iter().all(|&c| c == ' ')
}

/// `' +'`.
pub(crate) fn indent(s: &[char]) -> bool {
    s.first() == Some(&' ')
}

/// `[-+*\u{2022}\u{2023}\u{2043}]( +|$)`.
pub(crate) fn bullet(s: &[char]) -> Option<usize> {
    match s.first() {
        Some('-' | '+' | '*' | '\u{2022}' | '\u{2023}' | '\u{2043}') => spaces_or_end(s, 1),
        _ => None,
    }
}

/// An enumerated list format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    /// `(1)`
    Parens,
    /// `1)`
    Rparen,
    /// `1.`
    Period,
}

impl Format {
    pub(crate) fn prefix(self) -> &'static str {
        match self {
            Format::Parens => "(",
            _ => "",
        }
    }

    pub(crate) fn suffix(self) -> &'static str {
        match self {
            Format::Period => ".",
            _ => ")",
        }
    }
}

/// A matched enumerator.
#[derive(Debug, Clone)]
pub(crate) struct Enumerator {
    pub(crate) format: Format,
    /// The enumerator without its prefix and suffix.
    pub(crate) text: String,
    /// The end of the match, spaces included.
    pub(crate) end: usize,
}

/// The candidate enumerator texts at `p`, in the order of docutils'
/// alternation `[0-9]+|[a-z]|[A-Z]|[ivxlcdm]+|[IVXLCDM]+|#`.
fn enum_texts(s: &[char], p: usize) -> Vec<usize> {
    let run = |f: fn(char) -> bool| p + s[p..].iter().take_while(|&&c| f(c)).count();
    let one = |f: fn(char) -> bool| p + usize::from(s.get(p).is_some_and(|&c| f(c)));
    let mut ends = vec![
        run(|c| c.is_ascii_digit()),
        one(|c| c.is_ascii_lowercase()),
        one(|c| c.is_ascii_uppercase()),
        run(|c| "ivxlcdm".contains(c)),
        run(|c| "IVXLCDM".contains(c)),
        one(|c| c == '#'),
    ];
    ends.retain(|&e| e > p);
    ends
}

/// `(parens|rparen|period)( +|$)`.
pub(crate) fn enumerator(s: &[char]) -> Option<Enumerator> {
    for format in [Format::Parens, Format::Rparen, Format::Period] {
        let start = format.prefix().len();
        if format == Format::Parens && s.first() != Some(&'(') {
            continue;
        }
        let suffix = format.suffix().chars().next().unwrap();
        for e in enum_texts(s, start) {
            if s.get(e) == Some(&suffix)
                && let Some(end) = spaces_or_end(s, e + 1)
            {
                return Some(Enumerator {
                    format,
                    text: s[start..e].iter().collect(),
                    end,
                });
            }
        }
    }
    None
}

/// `:(?![: ])([^:\\]|\\.|:(?!([ `]|$)))*(?<! ):( +|$)`.
pub(crate) fn field_marker(s: &[char]) -> Option<usize> {
    if s.first() != Some(&':') || matches!(s.get(1), Some(':' | ' ')) {
        return None;
    }
    // Every position the body can stop at, in order.
    let mut stops = vec![1];
    let mut i = 1;
    while i < s.len() {
        match s[i] {
            '\\' if i + 1 < s.len() => i += 2,
            '\\' => break,
            ':' if !matches!(s.get(i + 1), None | Some(' ' | '`')) => i += 1,
            ':' => break,
            _ => i += 1,
        }
        stops.push(i);
    }
    stops.into_iter().rev().find_map(|p| {
        if s.get(p) == Some(&':') && s[p - 1] != ' ' {
            spaces_or_end(s, p + 1)
        } else {
            None
        }
    })
}

/// The ends of `optarg` at `p`, in preference order:
/// `([a-zA-Z][a-zA-Z0-9_-]*|<[^<>]+>)`.
fn optarg_ends(s: &[char], p: usize) -> Vec<usize> {
    let mut ends = Vec::new();
    if s.get(p).is_some_and(char::is_ascii_alphabetic) {
        let n = s[p + 1..]
            .iter()
            .take_while(|&&c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            .count();
        ends.extend((p + 1..=p + 1 + n).rev());
    }
    if s.get(p) == Some(&'<') {
        let n = s[p + 1..]
            .iter()
            .take_while(|&&c| c != '<' && c != '>')
            .count();
        if n > 0 && s.get(p + 1 + n) == Some(&'>') {
            ends.push(p + 2 + n);
        }
    }
    ends
}

/// The ends of one `option` at `p`, in preference order.
fn option_ends(s: &[char], p: usize) -> Vec<usize> {
    let mut ends = Vec::new();
    // shortopt: (-|\+)[a-zA-Z0-9]( ?optarg)?
    if matches!(s.get(p), Some('-' | '+')) && s.get(p + 1).is_some_and(char::is_ascii_alphanumeric)
    {
        let q = p + 2;
        if s.get(q) == Some(&' ') {
            ends.extend(optarg_ends(s, q + 1));
        }
        ends.extend(optarg_ends(s, q));
        ends.push(q);
    }
    // longopt: (--|/)optname([ =]optarg)?
    let name_start = if s[p..].starts_with(&['-', '-']) {
        Some(p + 2)
    } else if s.get(p) == Some(&'/') {
        Some(p + 1)
    } else {
        None
    };
    if let Some(n) = name_start
        && s.get(n).is_some_and(char::is_ascii_alphanumeric)
    {
        let len = s[n + 1..]
            .iter()
            .take_while(|&&c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            .count();
        for q in (n + 1..=n + 1 + len).rev() {
            if matches!(s.get(q), Some(' ' | '=')) {
                ends.extend(optarg_ends(s, q + 1));
            }
            ends.push(q);
        }
    }
    ends
}

/// `option(, option)*(  +| ?$)` from the end `e` of an option.
fn option_rest(s: &[char], e: usize) -> Option<usize> {
    if s[e..].starts_with(&[',', ' ']) {
        for e2 in option_ends(s, e + 2) {
            if let Some(end) = option_rest(s, e2) {
                return Some(end);
            }
        }
    }
    let spaces = s[e..].iter().take_while(|&&c| c == ' ').count();
    if spaces >= 2 {
        Some(e + spaces)
    } else if e + spaces == s.len() {
        Some(s.len())
    } else {
        None
    }
}

/// `%(option)s(, %(option)s)*(  +| ?$)`.
pub(crate) fn option_marker(s: &[char]) -> Option<usize> {
    if s.is_empty() {
        return None;
    }
    option_ends(s, 0)
        .into_iter()
        .find_map(|e| option_rest(s, e))
}

/// `>>>( +|$)`.
pub(crate) fn doctest(s: &[char]) -> Option<usize> {
    if s.starts_with(&['>', '>', '>']) {
        spaces_or_end(s, 3)
    } else {
        None
    }
}

/// `\|( +|$)`: the end, and the number of spaces after the bar.
pub(crate) fn line_block(s: &[char]) -> Option<(usize, usize)> {
    if s.first() == Some(&'|') {
        spaces_or_end(s, 1).map(|end| (end, end - 1))
    } else {
        None
    }
}

/// `\.\.( +|$)`.
pub(crate) fn explicit_markup(s: &[char]) -> Option<usize> {
    if s.starts_with(&['.', '.']) {
        spaces_or_end(s, 2)
    } else {
        None
    }
}

/// `__( +|$)`.
pub(crate) fn anonymous(s: &[char]) -> Option<usize> {
    if s.starts_with(&['_', '_']) {
        spaces_or_end(s, 2)
    } else {
        None
    }
}

/// `(%(nonalphanum7bit)s)\1* *$`: a line of one punctuation character.
pub(crate) fn line(s: &[char]) -> bool {
    match s.first() {
        Some(&c) if is_punctuation(c) => {
            let n = s.iter().take_while(|&&x| x == c).count();
            s[n..].iter().all(|&x| x == ' ')
        }
        _ => false,
    }
}

/// `\.\.[ ]+`: the end of the explicit markup start.
fn explicit_start(s: &[char]) -> Option<usize> {
    if !s.starts_with(&['.', '.']) {
        return None;
    }
    let n = s[2..].iter().take_while(|&&c| c == ' ').count();
    (n > 0).then_some(2 + n)
}

/// A footnote label: `\.\.[ ]+\[([0-9]+|\#|\#simplename|\*)\]([ ]+|$)`.
pub(crate) fn footnote(s: &[char]) -> Option<(String, usize)> {
    let p = explicit_start(s)?;
    if s.get(p) != Some(&'[') {
        return None;
    }
    let q = p + 1;
    let mut candidates = Vec::new();
    let digits = s[q..].iter().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 {
        candidates.push(q + digits);
    }
    if s.get(q) == Some(&'#') {
        candidates.push(q + 1);
        candidates.extend(simplename_ends(s, q + 1));
    }
    if s.get(q) == Some(&'*') {
        candidates.push(q + 1);
    }
    candidates.into_iter().find_map(|e| {
        if s.get(e) == Some(&']') {
            let end = spaces_or_end(s, e + 1)?;
            Some((s[q..e].iter().collect(), end))
        } else {
            None
        }
    })
}

/// A citation label: `\.\.[ ]+\[(simplename)\]([ ]+|$)`.
pub(crate) fn citation(s: &[char]) -> Option<(String, usize)> {
    let p = explicit_start(s)?;
    if s.get(p) != Some(&'[') {
        return None;
    }
    simplename_ends(s, p + 1).into_iter().find_map(|e| {
        if s.get(e) == Some(&']') {
            let end = spaces_or_end(s, e + 1)?;
            Some((s[p + 1..e].iter().collect(), end))
        } else {
            None
        }
    })
}

/// `\.\.[ ]+_(?![ ]|$)`.
pub(crate) fn hyperlink_target(s: &[char]) -> Option<usize> {
    let p = explicit_start(s)?;
    (s.get(p) == Some(&'_') && not_space_or_end(s, p + 1)).then_some(p + 1)
}

/// `\.\.[ ]+\|(?![ ]|$)`.
pub(crate) fn substitution_def(s: &[char]) -> Option<usize> {
    let p = explicit_start(s)?;
    (s.get(p) == Some(&'|') && not_space_or_end(s, p + 1)).then_some(p + 1)
}

/// `(simplename)[ ]?::([ ]+|$)` at `p`: the directive name and the end.
fn directive_at(s: &[char], p: usize) -> Option<(String, usize)> {
    simplename_ends(s, p).into_iter().find_map(|e| {
        let mut options = Vec::new();
        if s.get(e) == Some(&' ') {
            options.push(e + 1);
        }
        options.push(e);
        options.into_iter().find_map(|q| {
            if s[q..].starts_with(&[':', ':']) {
                let end = spaces_or_end(s, q + 2)?;
                Some((s[p..e].iter().collect(), end))
            } else {
                None
            }
        })
    })
}

/// `\.\.[ ]+(simplename)[ ]?::([ ]+|$)`.
pub(crate) fn directive(s: &[char]) -> Option<(String, usize)> {
    directive_at(s, explicit_start(s)?)
}

/// `(simplename)::( +|$)`: a directive inside a substitution definition.
pub(crate) fn embedded_directive(s: &[char]) -> Option<(String, usize)> {
    simplename_ends(s, 0).into_iter().find_map(|e| {
        if s[e..].starts_with(&[':', ':']) {
            let end = spaces_or_end(s, e + 2)?;
            Some((s[..e].iter().collect(), end))
        } else {
            None
        }
    })
}

/// `(?<![\s\x00])` before `p`.
fn not_ws_or_null_before(s: &[char], p: usize) -> bool {
    p > 0 && !is_python_whitespace(s[p - 1]) && s[p - 1] != '\0'
}

/// The name of a hyperlink target, from text with escapes as nulls:
/// `(_|(?!_)(`?)(?![ `])(.+?)(?<![\s\x00])\2)(?<!(?<!\x00):)(?<![\s\x00])[ ]?:([ ]+|$)`.
/// Returns the end and the name (`None` for an anonymous target).
pub(crate) fn target_name(s: &[char]) -> Option<(usize, Option<String>)> {
    let finish = |pos: usize| -> Option<usize> {
        // (?<!(?<!\x00):)
        if pos > 0 && s[pos - 1] == ':' && !(pos > 1 && s[pos - 2] == '\0') {
            return None;
        }
        if !not_ws_or_null_before(s, pos) {
            return None;
        }
        let mut options = Vec::new();
        if s.get(pos) == Some(&' ') {
            options.push(pos + 1);
        }
        options.push(pos);
        options.into_iter().find_map(|q| {
            if s.get(q) == Some(&':') {
                spaces_or_end(s, q + 1)
            } else {
                None
            }
        })
    };
    if s.first() == Some(&'_') {
        return finish(1).map(|end| (end, None));
    }
    let quoted = s.first() == Some(&'`');
    let start = usize::from(quoted);
    if matches!(s.get(start), None | Some(' ' | '`')) {
        return None;
    }
    for e in start + 1..=s.len() {
        if !not_ws_or_null_before(s, e) {
            continue;
        }
        let pos = if quoted {
            if s.get(e) != Some(&'`') {
                continue;
            }
            e + 1
        } else {
            e
        };
        if let Some(end) = finish(pos) {
            return Some((end, Some(s[start..e].iter().collect())));
        }
    }
    None
}

/// A substitution name, from text with escapes as nulls:
/// `((?![ ])(.+?)(?<![\s\x00])\|)([ ]+|$)`. Returns the end and the name.
pub(crate) fn substitution_name(s: &[char]) -> Option<(usize, String)> {
    if matches!(s.first(), None | Some(' ')) {
        return None;
    }
    (1..s.len()).find_map(|e| {
        if s[e] == '|' && not_ws_or_null_before(s, e) {
            let end = spaces_or_end(s, e + 1)?;
            Some((end, s[..e].iter().collect()))
        } else {
            None
        }
    })
}

/// A reference to a target: `simplename_` or `` `phrase`_ ``. Returns the
/// name, escapes still as nulls.
pub(crate) fn reference(s: &[char]) -> Option<String> {
    if s.last() == Some(&'_') && is_simplename(&s[..s.len() - 1]) {
        return Some(s[..s.len() - 1].iter().collect());
    }
    let n = s.len();
    if n >= 4
        && s[0] == '`'
        && s[1] != ' '
        && s.ends_with(&['`', '_'])
        && not_ws_or_null_before(s, n - 2)
    {
        return Some(s[1..n - 2].iter().collect());
    }
    None
}

/// `[-_!~*'{|}/#?^`&=+$%a-zA-Z0-9\x00]`
fn is_emailc(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-_!~*'{|}/#?^`&=+$%\0".contains(c)
}

/// Whether `s` is an email address (docutils' `email_pattern`, anchored).
pub(crate) fn is_email(s: &str) -> bool {
    let Some((name, host)) = s.split_once('@') else {
        return false;
    };
    let name_ok = !name.is_empty()
        && name
            .split('.')
            .all(|p| !p.is_empty() && p.chars().all(is_emailc))
        && !name.ends_with('\0');
    let host: Vec<char> = host.chars().collect();
    let host_ok = host.len() >= 2
        && is_emailc(host[0])
        && host[1..host.len() - 1]
            .iter()
            .all(|&c| c == '.' || is_emailc(c))
        && (host[host.len() - 1].is_ascii_alphanumeric()
            || "_~*/=+".contains(host[host.len() - 1]));
    name_ok && host_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn markers() {
        assert_eq!(bullet(&c("- x")), Some(2));
        assert_eq!(bullet(&c("-x")), None);
        let e = enumerator(&c("(ii) x")).unwrap();
        assert_eq!(
            (e.format, e.text.as_str(), e.end),
            (Format::Parens, "ii", 5)
        );
        assert_eq!(enumerator(&c("3. x")).unwrap().text, "3");
        assert!(enumerator(&c("ab. x")).is_none());
        assert_eq!(field_marker(&c(":a:b: c")), Some(6));
        assert_eq!(field_marker(&c(":a: ")), Some(4));
        assert_eq!(field_marker(&c(":a :")), None);
        assert_eq!(field_marker(&c("::")), None);
    }

    #[test]
    fn options() {
        assert_eq!(option_marker(&c("-a  x")), Some(4));
        assert_eq!(option_marker(&c("-b file  x")), Some(9));
        assert_eq!(option_marker(&c("--input=file  x")), Some(14));
        assert_eq!(option_marker(&c("-c, --count  x")), Some(13));
        assert_eq!(option_marker(&c("-a x")), Some(4));
        assert_eq!(option_marker(&c("-a b c")), None);
        assert_eq!(option_marker(&c("--very-long-option")), Some(18));
        assert_eq!(option_marker(&c("-f <[path]file>  x")), Some(17));
    }

    #[test]
    fn explicit() {
        assert_eq!(footnote(&c(".. [1] x")), Some(("1".into(), 7)));
        assert_eq!(footnote(&c(".. [#a.b] x")).unwrap().0, "#a.b");
        assert_eq!(citation(&c(".. [CIT2002] x")).unwrap().0, "CIT2002");
        assert_eq!(directive(&c(".. note:: x")), Some(("note".into(), 10)));
        assert_eq!(directive(&c(".. a:b ::")).unwrap().0, "a:b");
        assert_eq!(target_name(&c("name: uri")), Some((6, Some("name".into()))));
        assert_eq!(target_name(&c("_: uri")), Some((3, None)));
        assert_eq!(target_name(&c("`a: b`: uri")).unwrap().1.unwrap(), "a: b");
        assert_eq!(substitution_name(&c("sub| x")), Some((5, "sub".into())));
        assert_eq!(reference(&c("example_")).as_deref(), Some("example"));
        assert_eq!(reference(&c("`a b`_")).as_deref(), Some("a b"));
        assert!(is_email("me@example.com"));
        assert!(!is_email("http://example.com"));
    }
}
