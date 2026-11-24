# lsdocs-rst for Lua

A fast [reStructuredText] parser that builds [docutils]' document tree, for
Lua 5.4 and LuaJIT. It is a Rust port of docutils' parser and transforms
([lsdocs-rst](../../crates/rst/README.md)), bound with [mlua].

```lua
local rst = require("lsdocs_rst")

local text = "Title\n=====\n\nHello, *world*.\n"
local doc, messages = rst.parse(text, { transforms = true })
print(rst.pformat(doc))
local emphasis = doc.children[2].children[2]
print(emphasis.kind, text:sub(emphasis.span.start + 1, emphasis.span["end"]))
```

- `parse(text, options)` returns `document, messages`; `parser(options)`
  makes a parser to reuse, with `:parse(text)` and `:settings()`.
  Options: `transforms` (run docutils' transforms, as `publish_doctree`
  does), `keep_unknown_directives`, `source` (the source name in
  messages), `settings` (a table of docutils setting names: `tab_width`,
  `pep_references`, ...; see `default_settings()`), and `root` (a
  directory `include` and friends read below; nothing is read from disk
  without it).
- The tree is plain tables: an element is `{ kind, attributes, children,
  span }`, text is `{ text, span }`. `span` is `{ start, ["end"] }`, 0-based
  byte offsets with `end` exclusive (so `text:sub(start + 1, end)`), or
  `nil` for nodes without a source position. Arrays have no metatable;
  missing values are `nil`. The one exception: an attribute value that is
  Python's `None` in docutils (only in `pending` nodes' details) is
  `rst.null`, so the key is kept.
- Writers take a tree in that form, edited or not: `pformat(tree)`
  (pseudo-XML) and `to_xml(tree, options)` (Docutils XML).
- `make_id(name)` (docutils' `make_id`), `default_settings()`, `version`,
  `null`.

## Building

`nur test lua` runs the specs (`spec/*_spec.lua`) in an embedded Lua 5.4,
and in LuaJIT except on Windows. The module for an external Lua builds
with `cargo build -p lsdocs-rst-lua --release --no-default-features
--features lua54,module` (or `luajit,module`); the library
(`liblsdocs_rst_lua.so`) is installed as `lsdocs_rst.so`. On macOS, link
with `-C link-arg=-undefined -C link-arg=dynamic_lookup`.

## Known limitations

- No directives, roles or file loaders written in Lua yet.
- No Windows modules yet, and no prebuilt ones on luarocks.org: the
  rockspec builds from source with cargo.

[reStructuredText]: https://docutils.sourceforge.io/rst.html
[docutils]: https://docutils.sourceforge.io/
[mlua]: https://github.com/mlua-rs/mlua
