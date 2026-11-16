//! Conformance with docutils: every `cases/<name>.rst` is parsed and printed
//! as pseudo-XML, and must equal what docutils wrote (`nur gen rst`):
//! `cases/<name>.pxml` for the parser alone, `cases/<name>.tpxml` with
//! transforms too, and `cases/<name>.xml`, the transformed tree through the
//! XML writer. Cases listed in `cases/PENDING` (or
//! `cases/PENDING-TRANSFORMS`) are expected to differ until the parser
//! supports them; one that starts matching fails the test too, so the
//! lists stay accurate. A case with a `cases/<name>.settings.json` (docutils
//! setting names and values) is parsed with those settings, by docutils
//! too.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn cases_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cases")
}

fn pending(file: &str) -> BTreeSet<String> {
    let text = std::fs::read_to_string(cases_dir().join(file)).unwrap_or_default();
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

/// A minimal line diff of `expected` and `actual`, for failure messages.
fn diff(expected: &str, actual: &str) -> String {
    let mut out = String::new();
    let (e, a): (Vec<_>, Vec<_>) = (expected.lines().collect(), actual.lines().collect());
    for i in 0..e.len().max(a.len()) {
        match (e.get(i), a.get(i)) {
            (Some(x), Some(y)) if x == y => out.push_str(&format!("  {x}\n")),
            (x, y) => {
                if let Some(x) = x {
                    out.push_str(&format!("- {x}\n"));
                }
                if let Some(y) = y {
                    out.push_str(&format!("+ {y}\n"));
                }
            }
        }
    }
    out
}

/// Compares every case's output with the `extension` files.
fn check(
    extension: &str,
    pending_file: &str,
    parser: lsdocs_rst::Parser,
    write: fn(&lsdocs_rst::Element) -> String,
) {
    let pending = pending(pending_file);
    let mut cases: Vec<PathBuf> = std::fs::read_dir(cases_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "rst"))
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no conformance cases found");

    let mut failures = Vec::new();
    let mut passed = 0;
    for case in &cases {
        let name = case.file_stem().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(case).unwrap();
        let Ok(expected) = std::fs::read_to_string(case.with_extension(extension)) else {
            failures.push(format!("{name}: no expected output; run `nur gen rst`"));
            continue;
        };
        let mut parser = parser.clone();
        if let Ok(json) = std::fs::read_to_string(case.with_extension("settings.json")) {
            let settings: lsdocs_rst::Settings = serde_json::from_str(&json).unwrap();
            parser = parser.settings(settings);
        }
        // `file_*` cases may read files, from the cases directory.
        if name.starts_with("file_") {
            parser = parser.file_loader(lsdocs_rst::DirectoryLoader::new(cases_dir()));
        }
        let actual = write(&parser.parse(&text));
        let matches = actual == expected;
        match (matches, pending.contains(&name)) {
            (true, false) => passed += 1,
            (false, true) => {}
            (true, true) => failures.push(format!(
                "{name}: passes now; remove it from cases/{pending_file}"
            )),
            (false, false) => failures.push(format!(
                "{name}: differs from docutils (- expected, + actual):\n{}",
                diff(&expected, &actual)
            )),
        }
    }
    for name in pending
        .iter()
        .filter(|p| !cases.iter().any(|c| c.file_stem().unwrap() == p.as_str()))
    {
        failures.push(format!(
            "{name}: listed in cases/{pending_file} but has no case"
        ));
    }
    eprintln!(
        "conformance (.{extension}): {passed} of {} cases match docutils ({} pending)",
        cases.len(),
        pending.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn parser_matches_docutils() {
    check(
        "pxml",
        "PENDING",
        lsdocs_rst::Parser::new().source("test data"),
        lsdocs_rst::Element::pformat,
    );
}

#[test]
fn transforms_match_docutils() {
    check(
        "tpxml",
        "PENDING-TRANSFORMS",
        lsdocs_rst::Parser::new()
            .source("test data")
            .transforms(true),
        lsdocs_rst::Element::pformat,
    );
}

#[test]
fn xml_matches_docutils() {
    check(
        "xml",
        "PENDING-TRANSFORMS",
        lsdocs_rst::Parser::new()
            .source("test data")
            .transforms(true),
        |doc| {
            doc.to_xml(&lsdocs_rst::XmlOptions {
                indents: true,
                ..lsdocs_rst::XmlOptions::default()
            })
        },
    );
}
