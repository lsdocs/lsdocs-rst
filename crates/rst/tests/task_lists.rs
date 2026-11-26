//! Task lists (`- [ ] item`, `- [x] item`), the `task_lists` setting.

#![allow(clippy::unwrap_used)]

use lsdocs_rst::{AttrValue, Parser, Settings};

fn parse(text: &str, task_lists: bool) -> lsdocs_rst::Element {
    Parser::new()
        .settings(Settings {
            task_lists,
            ..Settings::default()
        })
        .parse(text)
}

#[test]
fn off_by_default_like_docutils() {
    assert!(!Settings::default().task_lists);
    let doc = Parser::new().parse("- [x] done\n");
    assert!(!doc.pformat().contains("checked"), "{}", doc.pformat());
    assert!(doc.astext().contains("[x] done"));
}

#[test]
fn markers_become_the_checked_attribute() {
    let doc = parse("- [ ] todo\n- [x] done\n- [X]   spaced\n", true);
    let pformat = doc.pformat();
    assert_eq!(
        pformat,
        "<document>\n    <bullet_list bullet=\"-\">\n        <list_item checked=\"0\">\n            <paragraph>\n                todo\n        <list_item checked=\"1\">\n            <paragraph>\n                done\n        <list_item checked=\"1\">\n            <paragraph>\n                spaced\n",
        "{pformat}"
    );
}

#[test]
fn not_markers() {
    for text in [
        "- [x]\n",
        "- [x]done\n",
        "- [y] no\n",
        "- \\[x] escaped\n",
        "- x [x] later\n",
    ] {
        let doc = parse(text, true);
        assert!(
            !doc.pformat().contains("checked"),
            "{text:?}: {}",
            doc.pformat()
        );
    }
}

#[test]
fn enumerated_and_nested_items_and_spans() {
    let text = "1. [x] first\n\n   - [ ] inner\n     more\n";
    let doc = parse(text, true);
    let pformat = doc.pformat();
    assert_eq!(pformat.matches("checked=").count(), 2, "{pformat}");
    let list = doc.children[0].as_element().unwrap();
    let item = list.children[0].as_element().unwrap();
    assert_eq!(item.attributes.get("checked"), Some(&AttrValue::Bool(true)));
    let paragraph = item.children[0].as_element().unwrap();
    let span = paragraph.span.unwrap();
    assert_eq!(&text[span.start..span.end], "first");
}
