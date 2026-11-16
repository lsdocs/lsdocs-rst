//! The serde form of trees and settings: every conformance case's tree
//! survives a JSON round trip, and the JSON has the documented shape.

#![allow(clippy::unwrap_used)]

use std::path::Path;

use lsdocs_rst::{Element, Parser, Settings, TocBacklinks};
use serde_json::json;

#[test]
fn trees_round_trip_through_json() {
    let cases = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cases");
    let parser = Parser::new().source("test data").transforms(true);
    let mut count = 0;
    for entry in std::fs::read_dir(cases).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "rst") {
            continue;
        }
        let doc = parser.parse(&std::fs::read_to_string(&path).unwrap());
        let json = serde_json::to_string(&doc).unwrap();
        let back: Element = serde_json::from_str(&json).unwrap();
        assert_eq!(back, doc, "{}", path.display());
        count += 1;
    }
    assert!(count > 50, "only {count} cases");
}

#[test]
fn json_shape() {
    let doc = Parser::new().parse("Hi *there*.\n\n.. _t:\n\nText.\n");
    assert_eq!(
        serde_json::to_value(&doc).unwrap(),
        json!({
            "kind": "document",
            "children": [
                {
                    "kind": "paragraph",
                    "children": [
                        {"text": "Hi ", "span": {"start": 0, "end": 3}},
                        {
                            "kind": "emphasis",
                            "children": [{"text": "there", "span": {"start": 3, "end": 10}}],
                            "span": {"start": 3, "end": 10},
                        },
                        {"text": ".", "span": {"start": 10, "end": 11}},
                    ],
                    "span": {"start": 0, "end": 11},
                },
                {
                    "kind": "target",
                    "attributes": {"ids": ["t"], "names": ["t"]},
                    "span": {"start": 13, "end": 19},
                },
                {
                    "kind": "paragraph",
                    "children": [{"text": "Text.", "span": {"start": 21, "end": 26}}],
                    "span": {"start": 21, "end": 26},
                },
            ],
            "span": {"start": 0, "end": 27},
        })
    );
}

#[test]
fn settings_use_docutils_names() {
    let settings: Settings = serde_json::from_value(json!({
        "tab_width": 4,
        "toc_backlinks": "top",
        "title": "T",
    }))
    .unwrap();
    assert_eq!(
        settings,
        Settings {
            tab_width: 4,
            toc_backlinks: TocBacklinks::Top,
            title: Some("T".into()),
            ..Settings::default()
        }
    );
    let error = serde_json::from_value::<Settings>(json!({"tabwidth": 4})).unwrap_err();
    assert!(
        error.to_string().contains("unknown field `tabwidth`"),
        "{error}"
    );
}
