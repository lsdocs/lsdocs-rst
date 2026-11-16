# lsdocs-rst

A reStructuredText parser in Rust: a port of [docutils]' parser and
transforms that builds the same document tree docutils builds, with the
byte span of the text each node came from.

```rust
let doc = lsdocs_rst::Parser::new()
    .source("example")
    .transforms(true)
    .parse("Title\n=====\n\nHello, *world*.\n");
print!("{}", doc.pformat());
```

```text
<document ids="title" names="title" source="example" title="Title">
    <title>
        Title
    <paragraph>
        Hello, 
        <emphasis>
            world
        .
```

## What it does

- **The docutils tree.** Element kinds and attributes are docutils' node
  classes and attributes, and `pformat()` prints the same pseudo-XML.
  Every conformance case (`tests/cases/`) is checked against docutils'
  own output: the parser's tree (`.pxml`), the tree after docutils'
  transforms (`.tpxml`) and the XML writer's output (`.xml`). A fuzzer
  (`nur fuzz rst`) compares random documents with docutils.
- **Spans.** Each element and text node records the byte range of the
  source it came from.
- **Transforms** (`Parser::transforms`): references resolved, footnotes
  numbered, substitutions replaced, the document title and docinfo,
  `contents`, `sectnum` and `target-notes`, as `publish_doctree` does.
- **Settings** (`Settings`): the docutils settings that change the tree,
  under docutils' names and with docutils' defaults, such as `tab_width`,
  `pep_references`, `id_prefix` and `doctitle_xform`.
- **Extensions:** add directives and roles (`Directive`, `Role`), keep
  unknown directives as elements, and let directives read files through a
  `FileLoader` (nothing is read without one).
- **Output:** pseudo-XML (`pformat`) and Docutils XML (`to_xml`). With the
  `serde` feature, trees and settings serialize to and from JSON,
  MessagePack and the rest.

HTML and other output formats are left to the caller.

## Not yet

docutils' `smart_quotes`, `character_level_inline_markup` and languages
other than English are not supported, nor are syntax-highlighted `code`
blocks (the tree is the one docutils builds with `syntax_highlight`
set to `none`). See the TODO list in the repository's `ROADMAP.md`.

## Bindings

The same parser is available from Python (`lsdocs-rst`, module
`lsdocs_rst`) and Node.js (`@lsdocs/rst`); see `bindings/rst/` in the
repository.

## License

Apache-2.0 OR MIT.

[docutils]: https://docutils.sourceforge.io/
