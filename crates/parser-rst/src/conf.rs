//! docutils' configuration files (`docutils.conf`), for the parser.
//!
//! The INI format docutils reads: `[section]` headers, `name: value` or
//! `name = value` lines, `#` and `;` comments, continuation lines
//! indented. The parser's sections are `[general]`, `[parsers]` and
//! `[restructuredtext parser]`; others (writers, ...) are not the
//! parser's and are skipped, so a project's `docutils.conf` works as it
//! is. Names may use `-` or `_`.

use lsdocs_rst::Settings;
use serde_json::{Map, Value};

/// What a `docutils.conf` says to the parser.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Conf {
    /// The settings lsdocs-rst implements.
    pub settings: Settings,
    /// docutils' `file_insertion_enabled`: whether directives may read
    /// files.
    pub file_insertion_enabled: bool,
    /// Settings of the parser's sections that lsdocs-rst does not
    /// implement, as messages.
    pub warnings: Vec<String>,
}

impl Default for Conf {
    fn default() -> Self {
        Self {
            settings: Settings::default(),
            file_insertion_enabled: true,
            warnings: Vec::new(),
        }
    }
}

/// The sections whose settings are the parser's.
const SECTIONS: &[&str] = &["general", "parsers", "restructuredtext parser"];

/// docutils settings of those sections that are not the parser's to
/// honor (they concern output or the command line), so they are not
/// warned about.
const NOT_THE_PARSERS: &[&str] = &[
    "output_encoding",
    "output_encoding_error_handler",
    "input_encoding",
    "input_encoding_error_handler",
    "report_level",
    "halt_level",
    "exit_status_level",
    "warning_stream",
    "traceback",
    "dump_settings",
    "dump_internals",
    "dump_transforms",
    "dump_pseudo_xml",
    "expose_internals",
    "strict_visitor",
    "generator",
    "datestamp",
    "source_link",
    "source_url",
    "language_code",
    "record_dependencies",
    "config",
    "debug",
];

/// Reads a `docutils.conf`; `file` names it in messages.
pub(crate) fn read(text: &str, file: &str) -> Result<Conf, String> {
    let mut conf = Conf::default();
    let defaults = serde_json::to_value(Settings::default()).map_err(|e| e.to_string())?;
    let Value::Object(defaults) = defaults else {
        return Err("settings are not a map".into());
    };
    let mut values = Map::new();
    let mut section: Option<String> = None;
    let mut last: Option<String> = None;
    for (i, raw) in text.lines().enumerate() {
        let line_no = i + 1;
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        if let Some(name) = trimmed.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            section = Some(name.trim().to_lowercase());
            last = None;
            continue;
        }
        let ours = section.as_deref().is_some_and(|s| SECTIONS.contains(&s));
        // A continuation of the last value.
        if line.starts_with(char::is_whitespace) {
            if ours
                && let Some(name) = &last
                && let Some(Value::String(value)) = values.get_mut(name)
            {
                value.push(' ');
                value.push_str(trimmed);
            }
            continue;
        }
        let Some(at) = line.find([':', '=']) else {
            return Err(format!("{file}:{line_no}: expected `name: value`"));
        };
        if section.is_none() {
            return Err(format!("{file}:{line_no}: a setting before any [section]"));
        }
        if !ours {
            continue;
        }
        let name = line[..at].trim().to_lowercase().replace('-', "_");
        let value = line[at + 1..].trim().to_owned();
        last = Some(name.clone());
        values.insert(name, Value::String(value));
    }
    let mut typed = Map::new();
    for (name, value) in values {
        let Value::String(value) = value else {
            continue;
        };
        if name == "file_insertion_enabled" {
            conf.file_insertion_enabled = boolean(&value)
                .ok_or_else(|| format!("{file}: `{name}`: not a boolean: {value:?}"))?;
            continue;
        }
        match defaults.get(&name) {
            Some(default) => {
                let converted = match default {
                    Value::Bool(_) => Value::Bool(
                        boolean(&value)
                            .ok_or_else(|| format!("{file}: `{name}`: not a boolean: {value:?}"))?,
                    ),
                    Value::Number(_) => Value::from(
                        value
                            .parse::<u64>()
                            .map_err(|_| format!("{file}: `{name}`: not a number: {value:?}"))?,
                    ),
                    _ => Value::String(value),
                };
                typed.insert(name, converted);
            }
            None if NOT_THE_PARSERS.contains(&name.as_str()) => {}
            None => conf.warnings.push(format!(
                "{file}: docutils' `{name}` is not supported by lsdocs' reStructuredText parser; ignored"
            )),
        }
    }
    conf.settings =
        serde_json::from_value(Value::Object(typed)).map_err(|e| format!("{file}: {e}"))?;
    Ok(conf)
}

/// A boolean as docutils reads one.
fn boolean(value: &str) -> Option<bool> {
    match value.to_lowercase().as_str() {
        "1" | "yes" | "true" | "on" => Some(true),
        "0" | "no" | "false" | "off" | "" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn reads_known_settings() {
        let conf = read(
            "# A project's settings.\n[general]\ntab-width: 4\n\n[restructuredtext parser]\npep_references = yes\npep-base-url: https://pep.test/\n  more\n",
            "docutils.conf",
        )
        .unwrap();
        assert_eq!(conf.settings.tab_width, 4);
        assert!(conf.settings.pep_references);
        assert_eq!(conf.settings.pep_base_url, "https://pep.test/ more");
        assert!(conf.warnings.is_empty());
        assert!(conf.file_insertion_enabled);
    }

    #[test]
    fn skips_writer_sections() {
        let conf = read(
            "[html writers]\nstylesheet: x.css\n[general]\noutput_encoding: utf-8\n",
            "docutils.conf",
        )
        .unwrap();
        assert_eq!(conf, Conf::default());
    }

    #[test]
    fn warns_on_unimplemented() {
        let conf = read("[general]\nsmart_quotes: yes\n", "docutils.conf").unwrap();
        assert_eq!(conf.warnings.len(), 1);
        assert!(
            conf.warnings[0].contains("`smart_quotes`"),
            "{:?}",
            conf.warnings
        );
    }

    #[test]
    fn file_insertion() {
        let conf = read("[general]\nfile_insertion_enabled: no\n", "docutils.conf").unwrap();
        assert!(!conf.file_insertion_enabled);
    }

    #[test]
    fn bad_value_names_the_file() {
        let err = read("[general]\ntab_width: wide\n", "x/docutils.conf").unwrap_err();
        assert!(
            err.contains("x/docutils.conf") && err.contains("tab_width"),
            "{err}"
        );
        let err = read("tab_width: 4\n", "x/docutils.conf").unwrap_err();
        assert!(err.contains("x/docutils.conf:1"), "{err}");
    }
}
