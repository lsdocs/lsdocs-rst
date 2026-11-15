//! Directives and roles registered by the caller, and keeping unknown
//! directives.

use lsdocs_rst::{
    Directive, DirectiveContext, DirectiveError, DirectiveSpec, Element, Kind, Node, OptionKind,
    OptionValue, Parser, Role, RoleContext, Text,
};

/// `.. box:: title` with a `color` option and body content.
struct BoxDirective;

impl Directive for BoxDirective {
    fn spec(&self) -> DirectiveSpec {
        DirectiveSpec {
            optional_arguments: 1,
            final_argument_whitespace: true,
            options: vec![
                (
                    "color".into(),
                    OptionKind::Choice(vec!["red".into(), "blue".into()]),
                ),
                ("width".into(), OptionKind::NonNegativeInt),
            ],
            has_content: true,
            ..Default::default()
        }
    }

    fn run(&self, cx: &mut DirectiveContext<'_>) -> Result<Vec<Node>, DirectiveError> {
        if cx.content().is_empty() {
            return Err(DirectiveError::error("box needs content"));
        }
        let mut node = Element::new(Kind::Container);
        if let Some(OptionValue::Text(color)) = cx.option("color") {
            node.attributes.push("classes", color);
        }
        if let Some(title) = cx.arguments().first().cloned() {
            let (nodes, _) = cx.parse_inline(&title);
            node.children
                .push(Element::new(Kind::Title).with_children(nodes).into());
        }
        Ok(vec![cx.parse_content(node).into()])
    }
}

/// `` :shout:`text` `` in capitals, with any role options as classes.
struct Shout;

impl Role for Shout {
    fn run(&self, cx: &mut RoleContext<'_>) -> Result<Vec<Node>, DirectiveError> {
        if cx.text() == "quiet" {
            return Err(DirectiveError::warning("nothing to shout"));
        }
        let mut node =
            Element::new(Kind::Strong).with_child(Text::new(cx.text().to_uppercase(), cx.span()));
        for (name, value) in cx.options() {
            if let (OptionValue::List(classes), "class") = (value, name.as_str()) {
                for c in classes {
                    node.attributes.push("classes", c);
                }
            }
        }
        Ok(vec![node.into()])
    }
}

fn pformat(parser: &Parser, text: &str) -> String {
    parser.clone().source("test").parse(text).pformat()
}

#[test]
fn registered_directive() {
    let parser = Parser::new().directive("Box", BoxDirective);
    let out = pformat(
        &parser,
        ".. box:: A *title*\n   :color: Red\n\n   Body **text**.\n",
    );
    assert_eq!(
        out,
        "<document source=\"test\">\n    <container classes=\"red\">\n        <title>\n            A \n            <emphasis>\n                title\n        <paragraph>\n            Body \n            <strong>\n                text\n            .\n"
    );
    // Option errors are reported like docutils' own.
    let out = pformat(&parser, ".. box::\n   :color: green\n\n   x\n");
    for line in [
        "Error in \"box\" directive:",
        "invalid option value: (option: \"color\"; value: 'green')",
        "\"green\" unknown; choose from \"red\", or \"blue\".",
    ] {
        assert!(out.contains(line), "{out}");
    }
    let out = pformat(&parser, ".. box::\n");
    assert!(out.contains("box needs content"), "{out}");
}

#[test]
fn registered_directive_overrides_standard() {
    let out = pformat(
        &Parser::new().directive("note", BoxDirective),
        ".. note::\n\n   x\n",
    );
    assert!(out.contains("<container>"), "{out}");
}

#[test]
fn registered_role_and_custom_role_on_it() {
    let parser = Parser::new().role("shout", Shout);
    let out = pformat(
        &parser,
        ".. role:: loud(shout)\n   :class: x\n\n:shout:`hi` :loud:`there` :shout:`quiet`\n",
    );
    assert!(out.contains("<strong>\n            HI"), "{out}");
    assert!(
        out.contains("<strong classes=\"x\">\n            THERE"),
        "{out}"
    );
    assert!(out.contains("nothing to shout"), "{out}");
}

#[test]
fn unknown_directives_kept() {
    let parser = Parser::new().keep_unknown_directives(true);
    let out = pformat(
        &parser,
        ".. mermaid:: graph TD\n   :caption: A *chart*\n   :flag:\n\n   A --> B\n   B --> C\n\nText.\n",
    );
    assert_eq!(
        out,
        "<document source=\"test\">\n    <directive argument=\"graph TD\" name=\"mermaid\">\n        <field_list>\n            <field>\n                <field_name>\n                    caption\n                <field_body>\n                    <paragraph>\n                        A *chart*\n            <field>\n                <field_name>\n                    flag\n                <field_body>\n        <literal_block xml:space=\"preserve\">\n            A --> B\n            B --> C\n    <paragraph>\n        Text.\n"
    );
    // Off by default: reported the docutils way.
    let out = pformat(&Parser::new(), ".. mermaid:: x\n");
    assert!(out.contains("Unknown directive type \"mermaid\"."), "{out}");
}
