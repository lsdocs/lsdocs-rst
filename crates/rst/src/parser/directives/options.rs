//! Directive option conversion (docutils' `directives` conversion
//! functions), with docutils' (and Python's) error messages.

pub(crate) use crate::util::py_repr;
use crate::util::{
    escape2null, make_id, split_escaped_whitespace, split_whitespace, strip, unescape,
};

/// How an option's text is converted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Opt {
    Flag,
    Unchanged,
    UnchangedRequired,
    Path,
    Uri,
    Int,
    NonNegativeInt,
    Percentage,
    LengthOrUnitless,
    LengthOrPercentageOrUnitless,
    Class,
    Encoding,
    ParserName,
    /// `value_or((None,), int)`: a number or nothing.
    IntOrNone,
    Choice(&'static [&'static str]),
    /// The `contents` directive's `backlinks`.
    Backlinks,
    /// The `figure` directive's `figwidth`.
    FigWidth,
    /// One character or a character code.
    SingleChar,
    /// As `SingleChar`, or `tab` or `space`.
    SingleCharOrWhitespace,
    /// A table's `widths`: `auto`, or with `grid` also `grid`, or a list of
    /// positive numbers.
    Widths {
        grid: bool,
    },
}

/// A converted option value.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum OptValue {
    None,
    Str(String),
    Int(i64),
    List(Vec<String>),
    IntList(Vec<i64>),
}

impl OptValue {
    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            OptValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn as_list(&self) -> Vec<String> {
        match self {
            OptValue::List(l) => l.clone(),
            _ => Vec::new(),
        }
    }
}

/// Python's `repr()` of an optional string.
pub(crate) fn py_repr_opt(s: Option<&str>) -> String {
    s.map_or_else(|| "None".to_owned(), py_repr)
}

/// docutils' `unicode_code`.
pub(crate) fn unicode_code(code: &str) -> Result<String, String> {
    let range = "ValueError: chr() arg not in range(0x110000)";
    let to_char = |n: u64| -> Result<String, String> {
        u32::try_from(n)
            .ok()
            .and_then(char::from_u32)
            .map(String::from)
            .ok_or_else(|| range.to_owned())
    };
    if !code.is_empty() && code.chars().all(|c| c.is_ascii_digit()) {
        return code
            .parse::<u64>()
            .map_err(|_| range.to_owned())
            .and_then(to_char);
    }
    let lower = code.to_lowercase();
    let hex = ["0x", "x", "\\x", "u+", "u", "\\u"]
        .iter()
        .find_map(|p| lower.strip_prefix(p))
        .filter(|h| !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit()))
        .or_else(|| {
            lower
                .strip_prefix("&#x")
                .and_then(|h| h.strip_suffix(';'))
                .filter(|h| !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit()))
        });
    match hex {
        Some(h) => u64::from_str_radix(h, 16)
            .map_err(|_| range.to_owned())
            .and_then(to_char),
        None => Ok(code.to_owned()),
    }
}

const NONE_TYPE: &str =
    "int() argument must be a string, a bytes-like object or a real number, not 'NoneType'";

/// Python's `int()` of text.
pub(crate) fn python_int(text: Option<&str>) -> Result<i64, String> {
    let Some(text) = text else {
        return Err(NONE_TYPE.to_owned());
    };
    let invalid = || format!("invalid literal for int() with base 10: {}", py_repr(text));
    let t = strip(text);
    let digits = t.strip_prefix(['+', '-']).unwrap_or(t);
    if digits.is_empty()
        || !digits.chars().all(|c| c.is_ascii_digit() || c == '_')
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
    {
        return Err(invalid());
    }
    t.replace('_', "").parse().map_err(|_| invalid())
}

const CSS3_UNITS: &[&str] = &[
    "em", "ex", "ch", "rem", "vw", "vh", "vmin", "vmax", "cm", "mm", "Q", "in", "pt", "pc", "px",
];

/// docutils' `parse_measure`: a number and a unit.
fn parse_measure(measure: &str) -> Result<(String, f64, String), String> {
    let invalid = || format!("\"{measure}\" is no valid measure.");
    let chars: Vec<char> = measure.chars().collect();
    let mut i = 0;
    if chars.first() == Some(&'-') {
        i = 1;
    }
    let num_end = i + chars[i..]
        .iter()
        .take_while(|c| c.is_ascii_digit() || **c == '.')
        .count();
    if num_end == i {
        return Err(invalid());
    }
    let number: String = chars[..num_end].iter().collect();
    let mut j = num_end;
    while chars.get(j) == Some(&' ') {
        j += 1;
    }
    let unit: String = chars[j..].iter().collect();
    let unit_ok = unit
        .chars()
        .all(|c| c.is_ascii_alphabetic() || c == '\u{b5}')
        || unit == "%";
    if !unit_ok {
        return Err(invalid());
    }
    // Python: int(), else float().
    let text = if let Ok(n) = number.parse::<i64>() {
        (n.to_string(), n as f64)
    } else {
        let f: f64 = python_float(&number).ok_or_else(invalid)?;
        (python_float_str(f), f)
    };
    Ok((text.0, text.1, unit))
}

/// Python's `float()` of the digits-and-dots text `parse_measure` allows.
fn python_float(s: &str) -> Option<f64> {
    if s.matches('.').count() > 1 || s == "." || s == "-." {
        return None;
    }
    s.parse().ok()
}

/// Python's `str()` of a float.
fn python_float_str(f: f64) -> String {
    let s = format!("{f:?}");
    if s.contains('e') || s.contains('.') || s.contains("inf") || s.contains("NaN") {
        s
    } else {
        format!("{s}.0")
    }
}

/// docutils' `get_measure`.
fn get_measure(argument: &str, units: &[&str]) -> Result<String, String> {
    let (text, value, unit) = parse_measure(argument)?;
    if value < 0.0 || !units.contains(&unit.as_str()) {
        let listed: Vec<&str> = units.iter().copied().filter(|u| !u.is_empty()).collect();
        return Err(format!(
            "not a positive number or measure of one of the following units:\n{}",
            listed.join(", ")
        ));
    }
    Ok(format!("{text}{unit}"))
}

fn units_with(extra: &str) -> Vec<&'static str> {
    let mut units: Vec<&'static str> = CSS3_UNITS.to_vec();
    units.push(if extra == "%" { "%" } else { "" });
    units
}

/// docutils' `length_or_percentage_or_unitless`.
pub(crate) fn length_or_percentage_or_unitless(
    argument: Option<&str>,
    default: &str,
) -> Result<String, String> {
    let Some(arg) = argument else {
        return Err("expected string or bytes-like object, got 'NoneType'".to_owned());
    };
    match get_measure(arg, &units_with("%")) {
        Ok(v) => Ok(v),
        Err(error) => get_measure(arg, &[""])
            .map(|v| v + default)
            .map_err(|_| error),
    }
}

/// docutils' `class_option`.
pub(crate) fn class_option(argument: Option<&str>) -> Result<Vec<String>, String> {
    let Some(arg) = argument else {
        return Err("argument required but none supplied".to_owned());
    };
    split_whitespace(arg)
        .map(|name| {
            let id = make_id(name);
            if id.is_empty() {
                Err(format!("cannot make \"{name}\" into a class name"))
            } else {
                Ok(id)
            }
        })
        .collect()
}

fn format_values(values: &[&str]) -> String {
    let Some((last, rest)) = values.split_last() else {
        return String::new();
    };
    let rest: Vec<String> = rest.iter().map(|v| format!("\"{v}\"")).collect();
    format!("{}, or \"{last}\"", rest.join(", "))
}

/// docutils' `choice`.
pub(crate) fn choice(argument: Option<&str>, values: &[&str]) -> Result<String, String> {
    let Some(arg) = argument else {
        return Err(format!(
            "must supply an argument; choose from {}",
            format_values(values)
        ));
    };
    let value = strip(&arg.to_lowercase()).to_owned();
    if values.contains(&value.as_str()) {
        Ok(value)
    } else {
        Err(format!(
            "\"{arg}\" unknown; choose from {}",
            format_values(values)
        ))
    }
}

/// Encodings Python knows under these names (a common subset).
const ENCODINGS: &[&str] = &[
    "utf-8",
    "utf8",
    "utf_8",
    "u8",
    "utf-8-sig",
    "utf_8_sig",
    "utf-16",
    "utf16",
    "utf-16-le",
    "utf-16-be",
    "utf-32",
    "ascii",
    "us-ascii",
    "latin-1",
    "latin1",
    "latin_1",
    "iso-8859-1",
    "iso8859-1",
    "iso-8859-15",
    "cp1252",
    "windows-1252",
    "cp437",
    "shift_jis",
    "euc-jp",
    "gb2312",
    "gbk",
    "gb18030",
    "big5",
    "koi8-r",
    "mac-roman",
];

/// Converts an option's text (`None` if it has no text).
pub(crate) fn convert(opt: Opt, argument: Option<&str>) -> Result<OptValue, String> {
    let required = || "argument required but none supplied".to_owned();
    Ok(match opt {
        Opt::Flag => match argument {
            Some(a) if !strip(a).is_empty() => {
                return Err(format!("no argument is allowed; \"{a}\" supplied"));
            }
            _ => OptValue::None,
        },
        Opt::Unchanged => OptValue::Str(argument.unwrap_or("").to_owned()),
        Opt::UnchangedRequired => OptValue::Str(argument.ok_or_else(required)?.to_owned()),
        Opt::Path => {
            let a = argument.ok_or_else(required)?;
            OptValue::Str(crate::lines::split_lines(a).map(strip).collect())
        }
        Opt::Uri => {
            let a = argument.ok_or_else(required)?;
            let parts = split_escaped_whitespace(&escape2null(a));
            let joined: Vec<String> = parts
                .iter()
                .map(|p| split_whitespace(&unescape(p)).collect::<String>())
                .collect();
            OptValue::Str(joined.join(" "))
        }
        Opt::Int => OptValue::Int(python_int(argument)?),
        Opt::NonNegativeInt => {
            let n = python_int(argument)?;
            if n < 0 {
                return Err("negative value; must be positive or zero".to_owned());
            }
            OptValue::Int(n)
        }
        Opt::Percentage => {
            let trimmed = argument.map(|a| a.trim_end_matches([' ', '%']));
            return convert(Opt::NonNegativeInt, trimmed);
        }
        Opt::LengthOrUnitless => {
            let Some(arg) = argument else {
                return Err("expected string or bytes-like object, got 'NoneType'".to_owned());
            };
            OptValue::Str(get_measure(arg, &units_with(""))?)
        }
        Opt::LengthOrPercentageOrUnitless => {
            OptValue::Str(length_or_percentage_or_unitless(argument, "")?)
        }
        Opt::Class => OptValue::List(class_option(argument)?),
        Opt::Encoding => {
            let a = argument.unwrap_or("");
            if !ENCODINGS.contains(&a.to_lowercase().as_str()) {
                return Err(format!("unknown encoding: \"{a}\""));
            }
            OptValue::Str(a.to_owned())
        }
        Opt::ParserName => match argument {
            None | Some("") => OptValue::None,
            Some(a) => OptValue::Str(a.to_owned()),
        },
        Opt::IntOrNone => match argument {
            None => OptValue::None,
            a => OptValue::Int(python_int(a)?),
        },
        Opt::Choice(values) => OptValue::Str(choice(argument, values)?),
        Opt::Backlinks => {
            let v = choice(argument, &["top", "entry", "none"])?;
            if v == "none" {
                OptValue::None
            } else {
                OptValue::Str(v)
            }
        }
        Opt::SingleChar | Opt::SingleCharOrWhitespace => {
            let a = argument.unwrap_or("");
            let c = match a {
                "tab" if opt == Opt::SingleCharOrWhitespace => "	".to_owned(),
                "space" if opt == Opt::SingleCharOrWhitespace => " ".to_owned(),
                _ => unicode_code(a)?,
            };
            if c.chars().count() > 1 {
                return Err(format!(
                    "{} invalid; must be a single character or a Unicode code",
                    py_repr(&c)
                ));
            }
            OptValue::Str(c)
        }
        Opt::Widths { grid } => match argument {
            Some("auto") => OptValue::Str("auto".to_owned()),
            Some("grid") if grid => OptValue::Str("grid".to_owned()),
            None => return Err("argument of type 'NoneType' is not iterable".to_owned()),
            Some(a) => {
                let entries: Vec<&str> = if a.contains(',') {
                    a.split(',').collect()
                } else {
                    split_whitespace(a).collect()
                };
                let mut out = Vec::new();
                for e in entries {
                    let n = python_int(Some(e))?;
                    if n < 1 {
                        return Err("negative or zero value; must be positive".to_owned());
                    }
                    out.push(n);
                }
                OptValue::IntList(out)
            }
        },
        Opt::FigWidth => match argument {
            Some(a) if a.to_lowercase() == "image" => OptValue::Str("image".to_owned()),
            a => OptValue::Str(length_or_percentage_or_unitless(a, "px")?),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_like_docutils() {
        assert_eq!(
            convert(Opt::Percentage, Some("50 %")),
            Ok(OptValue::Int(50))
        );
        assert_eq!(
            convert(Opt::Int, Some("x")),
            Err("invalid literal for int() with base 10: 'x'".to_owned())
        );
        assert_eq!(
            convert(Opt::LengthOrUnitless, Some("3 pt")),
            Ok(OptValue::Str("3pt".into()))
        );
        assert_eq!(
            convert(Opt::LengthOrPercentageOrUnitless, Some("1.50%")),
            Ok(OptValue::Str("1.5%".into()))
        );
        assert_eq!(
            convert(Opt::FigWidth, Some("300")),
            Ok(OptValue::Str("300px".into()))
        );
        assert_eq!(
            convert(Opt::Choice(&["left", "right"]), Some("up")),
            Err("\"up\" unknown; choose from \"left\", or \"right\"".to_owned())
        );
        assert_eq!(py_repr("it's"), "\"it's\"");
        assert_eq!(py_repr("a\\b"), "'a\\\\b'");
    }
}
