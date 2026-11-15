//! Conformance with docutils: every `cases/<name>.rst` is parsed and printed
//! as pseudo-XML, and must equal `cases/<name>.pxml`, which docutils wrote
//! (`nur gen rst`). Cases listed in `cases/PENDING` are expected to differ
//! until the parser supports them; one that starts matching fails the test
//! too, so the list stays accurate.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn cases_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cases")
}

fn pending() -> BTreeSet<String> {
    let text = std::fs::read_to_string(cases_dir().join("PENDING")).unwrap_or_default();
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

#[test]
fn matches_docutils() {
    let pending = pending();
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
        let Ok(expected) = std::fs::read_to_string(case.with_extension("pxml")) else {
            failures.push(format!("{name}: no expected output; run `nur gen rst`"));
            continue;
        };
        let actual = lsdocs_rst::Parser::new()
            .source("test data")
            .parse(&text)
            .pformat();
        let matches = actual == expected;
        match (matches, pending.contains(&name)) {
            (true, false) => passed += 1,
            (false, true) => {}
            (true, true) => {
                failures.push(format!("{name}: passes now; remove it from cases/PENDING"))
            }
            (false, false) => failures.push(format!(
                "{name}: differs from docutils (- expected, + actual):\n{}",
                diff(&expected, &actual)
            )),
        }
    }
    let unknown: Vec<_> = pending
        .iter()
        .filter(|p| !cases.iter().any(|c| c.file_stem().unwrap() == p.as_str()))
        .collect();
    for name in unknown {
        failures.push(format!("{name}: listed in cases/PENDING but has no case"));
    }
    eprintln!(
        "conformance: {passed} of {} cases match docutils ({} pending)",
        cases.len(),
        pending.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
