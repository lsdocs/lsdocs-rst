//! The conformance cases of the Rust crate (crates/rst/tests/cases, written
//! by docutils with `nur gen rst`), through the Lua module: each case is
//! parsed from Lua, and its tree goes back through the writers as the
//! plain tables Lua got, so the tables must carry the whole tree.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use mlua::{Function, Lua, LuaSerdeExt, Table, Value};

fn cases() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/rst/tests/cases")
}

fn pending(name: &str) -> HashSet<String> {
    std::fs::read_to_string(cases().join(name))
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

fn read(path: &Path) -> String {
    String::from_utf8(std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn conformance() {
    let lua = Lua::new();
    let rst = lsdocs_rst_lua::open(&lua).unwrap();
    // pformat and to_xml of what parse returned, all in Lua.
    let run: Function = lua
        .load(
            r#"
            local rst, text, opts = ...
            local doc = rst.parse(text, opts)
            return rst.pformat(doc), rst.to_xml(doc, { indents = true })
            "#,
        )
        .into_function()
        .unwrap();
    let pending_parser = pending("PENDING");
    let pending_transforms = pending("PENDING-TRANSFORMS");
    let mut files: Vec<_> = std::fs::read_dir(cases())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "rst"))
        .collect();
    files.sort();
    assert!(files.len() > 50, "too few cases");

    let mut failures = Vec::new();
    for case in &files {
        let stem = case.file_stem().unwrap().to_string_lossy().into_owned();
        let text = read(case);
        let settings_file = case.with_extension("settings.json");
        let settings: Value = if settings_file.exists() {
            let json: serde_json::Value = serde_json::from_str(&read(&settings_file)).unwrap();
            lua.to_value(&json).unwrap()
        } else {
            Value::Nil
        };
        for transforms in [false, true] {
            let skip = if transforms {
                &pending_transforms
            } else {
                &pending_parser
            };
            if skip.contains(&stem) {
                continue;
            }
            let opts: Table = lua.create_table().unwrap();
            opts.set("source", "test data").unwrap();
            opts.set("transforms", transforms).unwrap();
            opts.set("settings", settings.clone()).unwrap();
            if stem.starts_with("file_") {
                opts.set("root", cases().to_string_lossy().into_owned())
                    .unwrap();
            }
            let (pformat, xml): (String, String) =
                match run.call((rst.clone(), text.as_str(), opts)) {
                    Ok(out) => out,
                    Err(e) => {
                        failures.push(format!("{stem} (transforms={transforms}): {e}"));
                        continue;
                    }
                };
            let suffix = if transforms { "tpxml" } else { "pxml" };
            if pformat != read(&case.with_extension(suffix)) {
                failures.push(format!("{stem}.{suffix}"));
            }
            if transforms && xml != read(&case.with_extension("xml")) {
                failures.push(format!("{stem}.xml"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
