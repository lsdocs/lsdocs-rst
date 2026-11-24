//! Runs the Lua specs (`spec/*_spec.lua`) in an embedded Lua, with
//! `require("lsdocs_rst")` resolving to this crate's module table. Built
//! with the default features (Lua 5.4) or `--no-default-features
//! --features luajit,vendored`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use mlua::{Function, Lua, Table};

#[test]
fn lua_specs() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("spec");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with("_spec.lua"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no specs in {}", dir.display());

    let mut failures = Vec::new();
    let mut ran = 0;
    for file in files {
        let lua = Lua::new();
        let preload: Table = lua
            .globals()
            .get::<Table>("package")
            .unwrap()
            .get("preload")
            .unwrap();
        preload
            .set(
                "lsdocs_rst",
                lua.create_function(|lua, ()| lsdocs_rst_lua::open(lua))
                    .unwrap(),
            )
            .unwrap();
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        let source = std::fs::read_to_string(&file).unwrap();
        let tests: Table = lua.load(&source).set_name(&name).eval().unwrap();
        let mut cases: Vec<(String, Function)> =
            tests.pairs().collect::<mlua::Result<_>>().unwrap();
        cases.sort_by(|a, b| a.0.cmp(&b.0));
        for (case, run) in cases {
            ran += 1;
            if let Err(e) = run.call::<()>(()) {
                failures.push(format!("{name}: {case}: {e}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {ran} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
    eprintln!("{ran} Lua specs passed");
}
