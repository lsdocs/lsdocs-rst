//! Text helpers shared by the parser, ported from docutils: name
//! normalization, identifiers, backslash escapes, column widths and Roman
//! numerals.

use unicode_normalization::UnicodeNormalization;

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

/// Python's `repr()` of a string.
pub(crate) fn py_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::new();
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// The current time formatted like C's `strftime` (UTC; docutils uses
/// local time). Supports the common codes; others are kept as written.
pub(crate) fn strftime(format: &str) -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    format_time(format, secs)
}

/// [`strftime`] for `secs` seconds after the Unix epoch.
pub(crate) fn format_time(format: &str, secs: i64) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    const DAYS: [&str; 7] = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ];
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (hour, minute, second) = (rem / 3600, rem % 3600 / 60, rem % 60);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let before: [i64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let yday = before[(month - 1) as usize] + day + i64::from(leap && month > 2);
    let weekday = (days + 3).rem_euclid(7) as usize; // 1970-01-01 was a Thursday
    let month_name = MONTHS[(month - 1) as usize];
    let mut out = String::new();
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('Y') => out.push_str(&year.to_string()),
            Some('y') => out.push_str(&format!("{:02}", year.rem_euclid(100))),
            Some('m') => out.push_str(&format!("{month:02}")),
            Some('d') => out.push_str(&format!("{day:02}")),
            Some('H') => out.push_str(&format!("{hour:02}")),
            Some('I') => out.push_str(&format!("{:02}", (hour + 11) % 12 + 1)),
            Some('p') => out.push_str(if hour < 12 { "AM" } else { "PM" }),
            Some('M') => out.push_str(&format!("{minute:02}")),
            Some('S') => out.push_str(&format!("{second:02}")),
            Some('j') => out.push_str(&format!("{yday:03}")),
            Some('B') => out.push_str(month_name),
            Some('b') => out.push_str(&month_name[..3]),
            Some('A') => out.push_str(DAYS[weekday]),
            Some('a') => out.push_str(&DAYS[weekday][..3]),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
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

/// The value of `c` as a Unicode decimal digit (Python's `\d`).
pub(crate) fn digit_value(c: char) -> Option<u32> {
    let c = u32::from(c);
    let table = crate::tables::DIGIT;
    let k = table.partition_point(|&(_, last)| last < c);
    let &(first, _) = table.get(k).filter(|&&(first, _)| first <= c)?;
    Some((c - first) % 10)
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
/// characters count two, combining characters none (docutils'
/// `column_width`).
pub(crate) fn column_width(text: &str) -> usize {
    text.chars()
        .map(|c| {
            if is_combining(c) {
                0
            } else if is_wide(c) {
                2
            } else {
                1
            }
        })
        .sum()
}

/// Whether `c` is East Asian Wide or Fullwidth.
pub(crate) fn is_wide(c: char) -> bool {
    in_table(c, crate::tables::WIDE)
}

/// Whether `c` is a combining character.
pub(crate) fn is_combining(c: char) -> bool {
    in_table(c, crate::tables::COMBINING)
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
    fn formats_times() {
        // 2024-02-29 13:05:09 UTC, a Thursday.
        let t = 1_709_211_909;
        assert_eq!(format_time("%Y-%m-%d %H:%M:%S", t), "2024-02-29 13:05:09");
        assert_eq!(
            format_time("%a %A %b %B %j %y %I%p %% %q", t),
            "Thu Thursday Feb February 060 24 01PM % %q"
        );
    }

    #[test]
    fn widths_and_escapes() {
        assert_eq!(column_width("abc"), 3);
        assert_eq!(column_width("\u{65e5}\u{672c}"), 4);
        assert_eq!(column_width("e\u{301}"), 1);
        assert_eq!(unescape(&escape2null(r"a\*b\ c")), "a*bc");
    }
}
