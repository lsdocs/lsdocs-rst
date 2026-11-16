//! The docutils settings the parser honors.
//!
//! Each field is named after the docutils setting it stands for, and its
//! default is docutils' default, so a parser with [`Settings::default`]
//! builds the tree docutils' `publish_doctree` builds with its defaults.
//! With the `serde` feature, settings deserialize from a map of docutils
//! setting names (unknown names are an error), so they can come from a
//! docutils configuration translated to JSON, or from Python keyword
//! arguments.
//!
//! ```
//! use lsdocs_rst::{Parser, Settings};
//!
//! let settings = Settings {
//!     pep_references: true,
//!     ..Settings::default()
//! };
//! let doc = Parser::new().settings(settings).parse("See PEP 8.\n");
//! assert!(doc.pformat().contains("refuri=\"https://peps.python.org/pep-0008\""));
//! ```

/// The docutils settings the parser and its transforms honor. See the
/// [module documentation](self).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default, deny_unknown_fields)
)]
pub struct Settings {
    /// Make a lone top-level section the document title, and a lone
    /// subsection then its subtitle (transforms only).
    pub doctitle_xform: bool,
    /// Turn a field list first in the document into the docinfo
    /// (transforms only).
    pub docinfo_xform: bool,
    /// Make a lone subsection of a section that section's subtitle
    /// (transforms only).
    pub sectsubtitle_xform: bool,
    /// Number sections for the `sectnum` directive. Without, the directive
    /// is removed and sections are left as they are (transforms only).
    pub sectnum_xform: bool,
    /// The document's `title` attribute, instead of the text of its title
    /// (transforms only).
    pub title: Option<String>,
    /// Tab stops, in columns.
    pub tab_width: usize,
    /// The longest line parsed, in characters; a longer line stops parsing
    /// with an error.
    pub line_length_limit: usize,
    /// Allow the `raw` directive and role.
    pub raw_enabled: bool,
    /// Turn `PEP 8` and `pep-0008` in plain text into references.
    pub pep_references: bool,
    /// Where PEP references point: this, then `pep_file_url_template`.
    pub pep_base_url: String,
    /// The PEP's part of its URL, a printf-style template of its number
    /// (`%d`, `%04d`, ...).
    pub pep_file_url_template: String,
    /// Turn `RFC 2822` in plain text into references.
    pub rfc_references: bool,
    /// Where RFC references point: this, then `rfc<number>.html`.
    pub rfc_base_url: String,
    /// Remove the whitespace before a footnote reference.
    pub trim_footnote_reference_space: bool,
    /// Where the section titles of a `contents` table link back to, unless
    /// the directive says (transforms only).
    pub toc_backlinks: TocBacklinks,
    /// A prefix for every id.
    pub id_prefix: String,
    /// The prefix of generated ids; a trailing `%` stands for the element's
    /// kind (`footnote-1`, `section-2`).
    pub auto_id_prefix: String,
    /// Remove comments (transforms only).
    pub strip_comments: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            doctitle_xform: true,
            docinfo_xform: true,
            sectsubtitle_xform: false,
            sectnum_xform: true,
            title: None,
            tab_width: 8,
            line_length_limit: 10_000,
            raw_enabled: true,
            pep_references: false,
            pep_base_url: "https://peps.python.org/".to_owned(),
            pep_file_url_template: "pep-%04d".to_owned(),
            rfc_references: false,
            rfc_base_url: "https://tools.ietf.org/html/".to_owned(),
            trim_footnote_reference_space: false,
            toc_backlinks: TocBacklinks::Entry,
            id_prefix: String::new(),
            auto_id_prefix: "%".to_owned(),
            strip_comments: false,
        }
    }
}

impl Settings {
    /// The URL of PEP `number`.
    pub fn pep_url(&self, number: i64) -> String {
        format!(
            "{}{}",
            self.pep_base_url,
            printf_int(&self.pep_file_url_template, number)
        )
    }

    /// The URL of RFC `number`.
    pub fn rfc_url(&self, number: i64) -> String {
        format!("{}rfc{number}.html", self.rfc_base_url)
    }
}

/// Where the section titles of a table of contents link back to (docutils'
/// `toc_backlinks`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "lowercase")
)]
pub enum TocBacklinks {
    /// To the title's entry in the table.
    #[default]
    Entry,
    /// To the table itself.
    Top,
    /// Nowhere.
    None,
}

impl TocBacklinks {
    /// The setting's value as docutils writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            TocBacklinks::Entry => "entry",
            TocBacklinks::Top => "top",
            TocBacklinks::None => "none",
        }
    }
}

/// Python's `template % number` for templates whose conversions are all
/// `%d` (with optional `0`/`-` flags and a width) or `%%`; anything else
/// is kept as it is.
fn printf_int(template: &str, number: i64) -> String {
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'%') {
            chars.next();
            out.push('%');
            continue;
        }
        let mut spec = String::new();
        while let Some(&c) = chars.peek() {
            if c == '0' || c == '-' || c.is_ascii_digit() {
                spec.push(c);
                chars.next();
            } else {
                break;
            }
        }
        if chars.peek() == Some(&'d') {
            chars.next();
            let left = spec.contains('-');
            let zero = spec.starts_with('0') && !left;
            let width: usize = spec.trim_start_matches(['0', '-']).parse().unwrap_or(0);
            let digits = number.unsigned_abs().to_string();
            let sign = if number < 0 { "-" } else { "" };
            let len = sign.len() + digits.len();
            let pad = width.saturating_sub(len);
            if left {
                out.push_str(&format!("{sign}{digits}{}", " ".repeat(pad)));
            } else if zero {
                out.push_str(&format!("{sign}{}{digits}", "0".repeat(pad)));
            } else {
                out.push_str(&format!("{}{sign}{digits}", " ".repeat(pad)));
            }
        } else {
            out.push('%');
            out.push_str(&spec);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printf_matches_python() {
        assert_eq!(printf_int("pep-%04d", 8), "pep-0008");
        assert_eq!(printf_int("pep-%04d", 12345), "pep-12345");
        assert_eq!(printf_int("%d.html", 42), "42.html");
        assert_eq!(printf_int("%5d|", 42), "   42|");
        assert_eq!(printf_int("%-5d|", 42), "42   |");
        assert_eq!(printf_int("100%% %04d", -7), "100% -007");
        assert_eq!(printf_int("x%sy", 1), "x%sy");
    }

    #[test]
    fn urls() {
        let s = Settings::default();
        assert_eq!(s.pep_url(8), "https://peps.python.org/pep-0008");
        assert_eq!(s.rfc_url(2822), "https://tools.ietf.org/html/rfc2822.html");
    }
}
