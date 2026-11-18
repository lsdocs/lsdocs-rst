#![allow(clippy::unwrap_used, clippy::panic)]
use lsdocs_plugin_sdk::doctree::{Document, Node, plain_text};
use lsdocs_plugin_sdk::protocol::{Diagnostic, Severity, Syntax};
use serde_json::json;

use super::parse;

fn doc(text: &str) -> (Document, Vec<Diagnostic>) {
    parse("guide/page.rst", text, None, &Syntax::default())
}

fn with_syntax(text: &str, syntax: serde_json::Value) -> (Document, Vec<Diagnostic>) {
    let syntax: Syntax = serde_json::from_value(syntax).unwrap();
    parse("guide/page.rst", text, None, &syntax)
}

fn kinds(doc: &Document) -> Vec<String> {
    doc.descendants().map(|n| n.kind().into_owned()).collect()
}

#[test]
fn sections_nest_and_titles_stay_sections() {
    let (d, diagnostics) = doc("Title\n=====\n\nIntro.\n\nPart\n----\n\nText.\n");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let Node::Section(top) = &d.children[0] else {
        panic!("{d:?}")
    };
    assert_eq!(top.id, None);
    let Node::Heading(h) = &top.children[0] else {
        panic!()
    };
    assert_eq!((h.level, plain_text(&h.children)), (1, "Title".into()));
    let Some(Node::Section(part)) = top.children.last() else {
        panic!()
    };
    let Node::Heading(h) = &part.children[0] else {
        panic!()
    };
    assert_eq!(h.level, 2);
    assert_eq!(d.title().as_deref(), Some("Title"));
}

#[test]
fn labels_become_ids_and_references_links() {
    let (d, diagnostics) = doc(
        ".. _My Label:\n\nTitle\n=====\n\nSee `Other`_, `Title`_ and `My Label`_.\n\n\
         Other\n-----\n\nUnreferenced\n------------\n",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let ids: Vec<_> = d
        .descendants()
        .filter_map(|n| match n {
            Node::Section(s) => Some(s.id.clone()),
            _ => None,
        })
        .collect();
    // The label; the referenced title's own id; none for the rest.
    assert_eq!(ids, [Some("my-label".into()), Some("other".into()), None]);
    let links: Vec<_> = d
        .descendants()
        .filter_map(|n| match n {
            Node::Link(l) => Some(json!([l.url, l.target, plain_text(&l.children)])),
            _ => None,
        })
        .collect();
    assert_eq!(
        links,
        [
            json!(["#other", {"anchor": "other"}, "Other"]),
            json!(["#title", {"anchor": "title"}, "Title"]),
            json!(["#my-label", {"anchor": "my-label"}, "My Label"]),
        ]
    );
}

#[test]
fn declared_directives_and_roles_become_custom_nodes() {
    let syntax = json!({
        "directives": [
            {"name": "versionadded", "required_arguments": 1, "final_argument_whitespace": true,
             "options": {"class": "value", "hidden": "flag"}, "content": "blocks"},
            {"name": "centered", "required_arguments": 1, "final_argument_whitespace": true,
             "argument": "inline"},
            {"name": "code-block", "optional_arguments": 1, "content": "text"},
        ],
        "roles": [{"name": "kbd"}],
    });
    let (d, diagnostics) = with_syntax(
        ".. versionadded:: 1.2 beta\n   :class: x\n   :hidden:\n\n   *New* here.\n\n\
         .. centered:: Hello *world*\n\n\
         .. code-block:: rust\n\n   fn main() {}\n   // done\n\n\
         Press :kbd:`Ctrl\\+C`.\n",
        syntax.clone(),
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let value = serde_json::to_value(&d.children).unwrap();
    assert_eq!(value[0]["type"], "custom");
    assert_eq!(value[0]["kind"], "versionadded");
    assert_eq!(
        value[0]["attrs"],
        json!({"argument": "1.2 beta", "options": {"class": "x", "hidden": true}})
    );
    assert_eq!(value[0]["children"][0]["type"], "paragraph");
    assert_eq!(value[0]["children"][0]["children"][0]["type"], "emphasis");
    assert_eq!(value[1]["kind"], "centered");
    assert_eq!(value[1]["children"][0]["kind"], "directive_argument");
    assert_eq!(value[1]["children"][0]["children"][1]["type"], "emphasis");
    assert_eq!(
        value[2]["attrs"],
        json!({"argument": "rust", "content": "fn main() {}\n// done"})
    );
    let role = &value[3]["children"][1];
    assert_eq!(role["kind"], "role");
    assert_eq!(role["attrs"], json!({"name": "kbd", "text": "Ctrl+C"}));
    assert_eq!(role["children"][0]["value"], "Ctrl+C");
    // Declarations are checked as docutils checks directives.
    let (_, diagnostics) = with_syntax(".. versionadded::\n   :nope: 1\n", syntax);
    assert!(
        diagnostics[0]
            .message
            .contains("Error in \"versionadded\" directive"),
        "{diagnostics:?}"
    );
    // Undeclared, the role is unknown.
    let (_, diagnostics) = doc(":kbd:`x`\n");
    assert!(
        diagnostics[0]
            .message
            .contains("Unknown interpreted text role \"kbd\""),
        "{diagnostics:?}"
    );
}

#[test]
fn bibliographic_fields_are_metadata() {
    let (d, _) =
        doc(":Author: Me\n:Authors: A; B\n:Version: 1.0\n:Title: Custom title\n\nTitle\n=====\n");
    assert_eq!(d.metadata["author"], json!("Me"));
    assert_eq!(d.metadata["authors"], json!(["A", "B"]));
    assert_eq!(d.metadata["version"], json!("1.0"));
    assert_eq!(d.title().as_deref(), Some("Custom title"));
    assert_eq!(kinds(&d)[0], "section");
    // Other fields: booleans and numbers as such.
    let (d, _) = doc(":nav_order: 3\n:nav_exclude: true\n:weight: 0.5\n:tag: v2\n\nText.\n");
    assert_eq!(
        json!(d.metadata),
        json!({"nav_order": 3, "nav_exclude": true, "weight": 0.5, "tag": "v2"})
    );
}

#[test]
fn common_constructs_have_core_kinds() {
    let (d, diagnostics) = doc(".. note:: Careful.\n\n\
         .. admonition:: Mine\n\n   Text.\n\n\
         Term\n   Definition.\n\n\
         Footnote [#]_ and :math:`x^2`.\n\n\
         .. [#] The note.\n\n\
         .. math:: E = mc^2\n\n\
         .. figure:: pic.png\n   :alt: A picture\n   :name: fig\n\n   The caption.\n\n\
         .. code:: rust\n\n   fn main() {}\n\n\
         ----\n\n\
         | line one\n| line two\n");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let k = kinds(&d);
    for kind in [
        "admonition",
        "definition_list",
        "definition_term",
        "footnote_reference",
        "footnote_definition",
        "math",
        "figure",
        "image",
        "caption",
        "code_block",
        "thematic_break",
        "line_break",
    ] {
        assert!(k.iter().any(|x| x == kind), "no {kind} in {k:?}");
    }
    let admonitions: Vec<_> = d
        .descendants()
        .filter_map(|n| match n {
            Node::Admonition(a) => Some((a.kind.clone(), a.title.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        admonitions,
        [
            ("note".into(), None),
            ("admonition".into(), Some("Mine".into()))
        ]
    );
    let figure = d
        .descendants()
        .find_map(|n| match n {
            Node::Figure(f) => Some(f),
            _ => None,
        })
        .unwrap();
    assert_eq!(figure.id.as_deref(), Some("fig"));
    let code = d
        .descendants()
        .find_map(|n| match n {
            Node::CodeBlock(c) => Some(c),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        (code.language.as_deref(), code.value.as_str()),
        (Some("rust"), "fn main() {}")
    );
}

#[test]
fn unknown_directives_become_custom_nodes() {
    let (d, diagnostics) = doc(".. mermaid:: arg\n   :theme: dark\n\n   graph TD; A-->B\n");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let Node::Custom(c) = &d.children[0] else {
        panic!("{d:?}")
    };
    assert_eq!(c.kind, "mermaid");
    assert_eq!(
        json!(c.attrs),
        json!({"argument": "arg", "options": {"theme": "dark"}, "content": "graph TD; A-->B"})
    );
}

#[test]
fn problems_are_diagnostics_with_spans() {
    let text = "Para with `broken link.\n\nAnd unknown_.\n";
    let (_, diagnostics) = doc(text);
    let found: Vec<_> = diagnostics
        .iter()
        .map(|d| {
            let s = d.span.unwrap();
            (
                d.severity,
                d.message.clone(),
                &text[s.start as usize..s.end as usize],
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            (
                Severity::Warning,
                "Inline interpreted text or phrase reference start-string without end-string."
                    .to_owned(),
                "`"
            ),
            (
                Severity::Warning,
                "Unknown target name: \"unknown\".".to_owned(),
                "unknown_"
            ),
        ]
    );
    assert!(
        diagnostics
            .iter()
            .all(|d| d.source.as_deref() == Some("guide/page.rst"))
    );
}

#[test]
fn includes_read_from_the_source_directory() {
    let dir = std::env::temp_dir().join(format!("lsdocs-parser-rst-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("guide")).unwrap();
    std::fs::write(dir.join("guide/part.txt"), "Included *text*.\n").unwrap();
    let (d, diagnostics) = parse(
        "guide/page.rst",
        ".. include:: part.txt\n",
        Some(dir.to_str().unwrap()),
        &Syntax::default(),
    );
    std::fs::remove_dir_all(&dir).ok();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(plain_text(&d.children), "Included text.");
    // Without file access the directive is reported.
    let (_, diagnostics) = doc(".. include:: part.txt\n");
    assert_eq!(diagnostics[0].message, "\"include\" directive disabled.");
}

#[tokio::test]
async fn docutils_conf_settings_apply() {
    use lsdocs_plugin_sdk::Parser as _;
    let dir = std::env::temp_dir().join(format!("lsdocs-rst-conf-{}", std::process::id()));
    let conf = dir.join("lsdocs/rst/docutils.conf");
    std::fs::create_dir_all(conf.parent().unwrap()).unwrap();
    std::fs::write(
        &conf,
        "[general]\nsmart_quotes: yes\n[restructuredtext parser]\npep_references: yes\n",
    )
    .unwrap();
    let params = lsdocs_plugin_sdk::protocol::InitializeParams {
        config: lsdocs_plugin_sdk::protocol::ConfigInfo {
            dir: dir.to_string_lossy().into_owned(),
            name: "rst".into(),
        },
        ..Default::default()
    };
    let plugin = crate::plugin();
    let setup = plugin.setup(&params).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(setup.config_files, ["lsdocs/rst/docutils.conf"]);
    let ctx = lsdocs_plugin_sdk::testing::context(&setup);
    let result = crate::Rst::default()
        .parse(
            lsdocs_plugin_sdk::protocol::ParseParams {
                source: "a.rst".into(),
                text: "See PEP 8.\n".into(),
                syntax: Default::default(),
            },
            &ctx,
        )
        .await
        .unwrap();
    let json = serde_json::to_string(&result.document).unwrap();
    assert!(json.contains("pep-0008"), "{json}");
}
