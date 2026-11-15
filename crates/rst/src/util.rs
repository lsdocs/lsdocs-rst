//! Text helpers shared by the parser, ported from docutils: name
//! normalization, identifiers, backslash escapes, column widths and Roman
//! numerals.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::canonical_combining_class;

use crate::lines::is_python_whitespace;

/// Python's `str.split()` (split on runs of whitespace, no empty items).
pub(crate) fn split_whitespace(s: &str) -> impl Iterator<Item = &str> {
    s.split(is_python_whitespace).filter(|p| !p.is_empty())
}

/// Python's `str.strip()`.
pub(crate) fn strip(s: &str) -> &str {
    s.trim_matches(is_python_whitespace)
}

/// Python's `str.rstrip()`.
pub(crate) fn rstrip(s: &str) -> &str {
    s.trim_end_matches(is_python_whitespace)
}

/// Python's `str.lstrip()`.
pub(crate) fn lstrip(s: &str) -> &str {
    s.trim_start_matches(is_python_whitespace)
}

/// The number of characters in `s`.
pub(crate) fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// `s` without its first `n` characters (Python's `s[n:]`).
pub(crate) fn skip_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[i..],
        None => "",
    }
}

/// The first `n` characters of `s` (Python's `s[:n]`).
pub(crate) fn take_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// A case- and whitespace-normalized reference name.
pub(crate) fn normalize_name(name: &str) -> String {
    split_whitespace(&name.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A whitespace-normalized name.
pub(crate) fn whitespace_normalize_name(name: &str) -> String {
    split_whitespace(name).collect::<Vec<_>>().join(" ")
}

/// Backslash escapes as nulls: `\x` becomes `\0x` (docutils'
/// `escape2null`).
pub(crate) fn escape2null(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            out.push('\0');
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Removes escape nulls, and escaped whitespace with them (docutils'
/// `unescape`).
pub(crate) fn unescape(text: &str) -> String {
    text.replace("\0 ", "")
        .replace("\0\n", "")
        .replace('\0', "")
}

/// Turns escape nulls back into backslashes (docutils'
/// `unescape(text, restore_backslashes=True)`).
pub(crate) fn restore_backslashes(text: &str) -> String {
    text.replace('\0', "\\")
}

/// Whether `c` is in a table of inclusive code point ranges.
pub(crate) fn in_table(c: char, table: &[(u32, u32)]) -> bool {
    let c = c as u32;
    table
        .binary_search_by(|&(a, b)| {
            if c < a {
                std::cmp::Ordering::Greater
            } else if c > b {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// Python's `\w`.
pub(crate) fn is_word(c: char) -> bool {
    in_table(c, crate::tables::WORD)
}

/// Python's `\s`.
pub(crate) fn is_space(c: char) -> bool {
    in_table(c, crate::tables::WHITESPACE)
}

/// Splits on escaped whitespace (null and space, or null and line break).
pub(crate) fn split_escaped_whitespace(text: &str) -> Vec<String> {
    text.split("\0 ")
        .flat_map(|s| s.split("\0\n"))
        .map(str::to_owned)
        .collect()
}

/// An identifier made from `name`, docutils' `make_id`: lowercase ASCII
/// letters and digits in hyphen-separated runs, starting with a letter.
pub(crate) fn make_id(name: &str) -> String {
    let mut id = String::new();
    for c in name.to_lowercase().chars() {
        match c {
            '\u{df}' => id.push_str("sz"),
            '\u{e6}' => id.push_str("ae"),
            '\u{153}' => id.push_str("oe"),
            '\u{238}' => id.push_str("db"),
            '\u{239}' => id.push_str("qp"),
            c => id.push(translate_id_char(c)),
        }
    }
    let ascii: String = id.nfkd().filter(char::is_ascii).collect();
    let words = split_whitespace(&ascii).collect::<Vec<_>>().join(" ");
    // Runs of anything but [a-z0-9] become one hyphen.
    let mut out = String::new();
    let mut in_run = false;
    for c in words.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
            in_run = false;
        } else if !in_run {
            out.push('-');
            in_run = true;
        }
    }
    // No leading digits or hyphens, no trailing hyphens.
    let out = out.trim_start_matches(|c: char| c == '-' || c.is_ascii_digit());
    out.trim_end_matches('-').to_owned()
}

/// Letters with strokes and hooks that NFKD does not decompose.
fn translate_id_char(c: char) -> char {
    match c {
        '\u{f8}' => 'o',
        '\u{111}' => 'd',
        '\u{127}' => 'h',
        '\u{131}' => 'i',
        '\u{142}' => 'l',
        '\u{167}' => 't',
        '\u{180}' | '\u{183}' => 'b',
        '\u{188}' => 'c',
        '\u{18c}' => 'd',
        '\u{192}' => 'f',
        '\u{199}' => 'k',
        '\u{19a}' => 'l',
        '\u{19e}' => 'n',
        '\u{1a5}' => 'p',
        '\u{1ab}' | '\u{1ad}' => 't',
        '\u{1b4}' => 'y',
        '\u{1b6}' => 'z',
        '\u{1e5}' => 'g',
        '\u{225}' => 'z',
        '\u{234}' => 'l',
        '\u{235}' => 'n',
        '\u{236}' => 't',
        '\u{237}' => 'j',
        '\u{23c}' => 'c',
        '\u{23f}' => 's',
        '\u{240}' => 'z',
        '\u{247}' => 'e',
        '\u{249}' => 'j',
        '\u{24b}' => 'q',
        '\u{24d}' => 'r',
        '\u{24f}' => 'y',
        c => c,
    }
}

/// The display width of `text` in columns: East Asian wide and full-width
/// characters count two, combining characters none.
pub(crate) fn column_width(text: &str) -> usize {
    text.chars()
        .map(|c| {
            if canonical_combining_class(c) != 0 {
                0
            } else if is_wide(c) {
                2
            } else {
                1
            }
        })
        .sum()
}

/// Whether `c` is East Asian Wide or Fullwidth (the main ranges of
/// Unicode's EastAsianWidth.txt).
fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F
        | 0x231A..=0x231B
        | 0x2329..=0x232A
        | 0x23E9..=0x23EC
        | 0x23F0 | 0x23F3
        | 0x25FD..=0x25FE
        | 0x2614..=0x2615
        | 0x2648..=0x2653
        | 0x267F | 0x2693 | 0x26A1
        | 0x26AA..=0x26AB
        | 0x26BD..=0x26BE
        | 0x26C4..=0x26C5
        | 0x26CE | 0x26D4 | 0x26EA
        | 0x26F2..=0x26F3
        | 0x26F5 | 0x26FA | 0x26FD | 0x2705
        | 0x270A..=0x270B
        | 0x2728 | 0x274C | 0x274E
        | 0x2753..=0x2755
        | 0x2757
        | 0x2795..=0x2797
        | 0x27B0 | 0x27BF
        | 0x2B1B..=0x2B1C
        | 0x2B50 | 0x2B55
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xA960..=0xA97F
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE10..=0xFE19
        | 0xFE30..=0xFE6F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x16FE0..=0x16FE4
        | 0x17000..=0x18CFF
        | 0x1B000..=0x1B2FF
        | 0x1F004 | 0x1F0CF | 0x1F18E
        | 0x1F191..=0x1F19A
        | 0x1F200..=0x1F251
        | 0x1F260..=0x1F265
        | 0x1F300..=0x1F64F
        | 0x1F680..=0x1F6FF
        | 0x1F7E0..=0x1F7EB
        | 0x1F90C..=0x1F9FF
        | 0x1FA70..=0x1FAFF
        | 0x20000..=0x2FFFD
        | 0x30000..=0x3FFFD)
}

const ROMAN: [(u32, &str); 13] = [
    (1000, "M"),
    (900, "CM"),
    (500, "D"),
    (400, "CD"),
    (100, "C"),
    (90, "XC"),
    (50, "L"),
    (40, "XL"),
    (10, "X"),
    (9, "IX"),
    (5, "V"),
    (4, "IV"),
    (1, "I"),
];

/// `n` as an uppercase Roman numeral; `None` outside 1 to 4999.
pub(crate) fn to_roman(mut n: i64) -> Option<String> {
    if !(1..=4999).contains(&n) {
        return None;
    }
    let mut out = String::new();
    for (value, numeral) in ROMAN {
        while n >= i64::from(value) {
            out.push_str(numeral);
            n -= i64::from(value);
        }
    }
    Some(out)
}

/// The value of a well-formed Roman numeral, all uppercase or all
/// lowercase.
pub(crate) fn from_roman(s: &str) -> Option<i64> {
    if s.is_empty() || !s.is_ascii() {
        return None;
    }
    let upper = s.to_ascii_uppercase();
    if s != upper && s != s.to_ascii_lowercase() {
        return None;
    }
    if !upper.chars().all(|c| "IVXLCDM".contains(c)) {
        return None;
    }
    let mut total = 0i64;
    let mut rest = upper.as_str();
    for (value, numeral) in ROMAN {
        while let Some(r) = rest.strip_prefix(numeral) {
            total += i64::from(value);
            rest = r;
        }
    }
    (rest.is_empty() && to_roman(total).as_deref() == Some(upper.as_str())).then_some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_like_docutils() {
        assert_eq!(make_id("Too Short Title"), "too-short-title");
        assert_eq!(make_id("1"), "");
        assert_eq!(make_id("Caf\u{e9} na\u{ef}ve"), "cafe-naive");
        assert_eq!(make_id("  -2 Stra\u{df}e!  "), "strasze");
    }

    #[test]
    fn roman_numerals() {
        assert_eq!(from_roman("iv"), Some(4));
        assert_eq!(from_roman("MMXXIV"), Some(2024));
        assert_eq!(from_roman("IIII"), None);
        assert_eq!(from_roman("Iv"), None);
        assert_eq!(to_roman(4999).as_deref(), Some("MMMMCMXCIX"));
        assert_eq!(to_roman(5000), None);
    }

    #[test]
    fn widths_and_escapes() {
        assert_eq!(column_width("abc"), 3);
        assert_eq!(column_width("\u{65e5}\u{672c}"), 4);
        assert_eq!(column_width("e\u{301}"), 1);
        assert_eq!(unescape(&escape2null(r"a\*b\ c")), "a*bc");
    }
}
