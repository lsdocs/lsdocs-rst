# lsdocs-rst

Part of [lsdocs](https://github.com/lsdocs), a static-site generator whose
parsers, transformers and renderers are plugins.

reStructuredText for lsdocs and beyond:

| Path | What |
|---|---|
| `crates/rst` | `lsdocs-rst`: a standalone reStructuredText parser, a port of docutils' state machine and transforms (docutils-style tree with spans), with docutils' settings, an XML writer and an optional `serde` feature. Depends on nothing of lsdocs. |
| `crates/parser-rst` | `lsdocs-parser-rst`: the lsdocs plugin parsing reStructuredText with it, settings from `docutils.conf`. |
| `bindings/python` | `lsdocs-rst` for Python (module `lsdocs_rst`). |
| `bindings/node` | `@lsdocs/rst` for Node.js. |
| `bindings/lua` | `lsdocs-rst` for Lua 5.4 and LuaJIT (module `lsdocs_rst`), with mlua. |

The conformance cases (`crates/rst/tests/cases`) are checked against
docutils (the version `DOCUTILS` in the nurfile pins); the bindings' tests run them
too. `nur gen rst` regenerates their expected trees with docutils, and
`nur fuzz rst` compares the parser with docutils on random documents.

## Known limitations

Items marked `[ ]` under TODO are also limitations until done. The
`docutils.conf` reader's differences from docutils are listed in
`crates/parser-rst/README.md`; the bindings' own in their READMEs.

## TODO

### lsdocs-rst

- [ ] `include :parser:` is reported as unsupported; docutils' standard
      include files (`<isonum.txt>`, `<s5defs.txt>`, ...) are not bundled
      (the path goes to the loader as written).
- [ ] `:encoding:` accepts every name Python knows, but only UTF-8
      (`utf-8-sig` too), Latin-1 and ASCII decode; others are an error.
- [ ] docutils prints `line="True"` for messages it reports with
      `line=None`; reproduced for the `class` directive's error only.
- [ ] `csv-table` cells: docutils numbers each cell's lines from 0, with
      the table's source; lsdocs-rst keeps each line's real origin. Only
      visible in messages about cell content.
- [ ] Absolute `include` paths: docutils makes them relative to the
      working directory; lsdocs-rst keeps them absolute. Expected trees for
      includes from subdirectories differ between Windows and other
      systems (path separators), so the conformance cases avoid them.
- [ ] A `.. end of inclusion from "x"` comment without an `include` makes
      docutils crash; lsdocs-rst ignores it.
- [ ] RCS keyword cleaning (docinfo) matches case-insensitively only for
      ASCII; Python's `re.IGNORECASE` also folds a few non-ASCII letters.
- [ ] `figure :figwidth: image`: docutils turns the URI into a path with
      `url2pathname` (percent-decoding); lsdocs-rst uses the URI as is.
- [ ] Messages for `:url:` failures are close to Python's, not identical.
- [ ] `nur fuzz rst` runs without file access, so includes are not fuzzed;
      add a mode with generated include files.
- [ ] Settings not supported yet: `smart_quotes` (and
      `smartquotes_locales`), `character_level_inline_markup`,
      `language_code` (labels and bibliographic field names are English),
      `root_prefix` (absolute include paths), `syntax_highlight` (`code`
      blocks are always as with `none`), `expose_internals`, and the
      `Decorations` settings (`generator`, `datestamp`, `source_link`,
      `source_url`).
- [ ] PEP and RFC references in plain text match ASCII digits only;
      Python's `\d` also matches other Unicode decimal digits.
- [ ] The XML writer doesn't check raw XML for well-formedness (docutils
      warns about invalid raw XML).
- [ ] Report docutils' `TargetNotes` infinite loop upstream: a paragraph
      with an embedded-URI reference and its own target, plus
      `target-notes`.

### parser-rst

- [ ] Its `docutils.conf` reader's known differences from docutils are
      listed in `crates/parser-rst/README.md`; keep that list current.
- [ ] Inline semantics are lost: `subscript`, `superscript`,
      `abbreviation`, `acronym` and `inline` with classes become their
      text; `title_reference` becomes emphasis. The inline `custom`
      convention `sphinx-addons` uses (`inline: true`, `html_tag`) would
      do.
- [ ] Tables lose cell spans (`morecols`, `morerows`) and stub columns; the
      doctree's tables have neither.
- [ ] Images lose `width`, `height`, `scale` and `align`; figures lose
      `figwidth` and `align`.
- [ ] Topic and sidebar titles become bold paragraphs; nested line blocks
      are indented with no-break spaces.
- [ ] Option values of unknown directives are kept as strings.
- [ ] Diagnostics about included files have no span.

- [ ] Task lists (`- [ ] item`, `- [x] item`) in reStructuredText: a
      docutils-style extension, so both parsers have them (Markdown's come
      from pulldown-cmark).

### Bindings

Their known limitations are listed in each package's README
(`bindings/python/README.md`, `bindings/node/README.md`,
`bindings/lua/README.md`); keep
those lists current. To fix:
- [ ] Python: exceptions from extensions are collected per `Parser`, so
      two threads parsing with one `Parser` at once can get each other's
      exception. Collect them per `parse()` call.
- [ ] Both: extensions can't be async (Node.js) or run on the calling
      thread's context (Python directives run on a worker thread).
- [ ] Lua: install the prebuilt modules build-rocks.yml makes through the
      rockspec on luarocks.org (it builds from source with cargo for now),
      and publish them at release (tag.yml).
- [ ] Lua: Windows modules (they link the Lua library, so build-rocks.yml
      needs an MSVC Lua 5.4 and LuaJIT to link to), and LuaJIT specs on
      Windows (`nur test lua` skips them: mlua's vendored LuaJIT needs an
      MSVC developer prompt).
