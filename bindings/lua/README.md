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
  without it, or loader functions; see Extensions), `directives` and
  `roles`.
- The tree is plain tables: an element is `{ kind, attributes, children,
  span }`, text is `{ text, span }`. `span` is `{ start, ["end"] }`, 0-based
  byte offsets with `end` exclusive (so `text:sub(start + 1, end)`), or
  `nil` for nodes without a source position. Arrays have no metatable;
  missing values are `nil`. The one exception: an attribute value that is
  Python's `None` in docutils (only in `pending` nodes' details) is
  `rst.null`, so the key is kept.
- Writers take a tree in that form, edited or not (a string in `children`
  is a text node): `pformat(tree)`
  (pseudo-XML) and `to_xml(tree, options)` (Docutils XML).
- `make_id(name)` (docutils' `make_id`), `default_settings()`, `version`,
  `null`.

## Extensions

Directives, roles and file access for `include` and friends can be Lua
functions, given in the parser's options:

```lua
local shout = {
  required_arguments = 1,
  final_argument_whitespace = true,
  options = { class = "class_option", level = { "low", "high" }, loud = "flag" },
  run = function(ctx)
    local nodes, messages = ctx:parse_inline(ctx.arguments[1]:upper())
    local result = { { kind = "paragraph", attributes = {}, children = nodes } }
    for _, m in ipairs(messages) do
      result[#result + 1] = m
    end
    return result
  end,
}

local function kbd(ctx)
  if ctx.text == "?" then
    return nil, "unknown key in " .. ctx.rawsource
  end
  return { { kind = "literal", attributes = { classes = { "kbd" } }, children = { ctx.text } } }
end

local parser = rst.parser({ directives = { shout = shout }, roles = { kbd = kbd } })
```

- `directives`: name -> a table with `run(ctx)` and what the directive
  accepts: `required_arguments`, `optional_arguments`,
  `final_argument_whitespace`, `has_content`, and `options` (name -> a
  docutils conversion function's name: `"flag"`, `"unchanged"`,
  `"unchanged_required"`, `"path"`, `"uri"`, `"int"`, `"nonnegative_int"`,
  `"percentage"`, `"length_or_unitless"`,
  `"length_or_percentage_or_unitless"`, `"class_option"`; or a list of
  choices). `ctx` has `name`, `arguments`, `options` (a flag is `true`),
  `content` (lines), `lineno`, `block_text`, `span`, `in_substitution`,
  and the methods `ctx:parse_content(container)` (the content parsed into
  that element), `ctx:parse_inline(text)` (`nodes, messages`) and
  `ctx:message(level, text)`. The methods work only while `run` runs.
- `roles`: name -> `function(ctx)`, with `name`, `text`, `raw_text`,
  `rawsource`, `options`, `lineno`, `span`.
- Both return a list of nodes (a string stands for a text node), or
  `nil, message, level` to report a problem in the document as docutils
  does for its own (`level` 1 info to 4 severe; 3, error, by default).
- `files` and `urls`: `function(path)` returning the contents, or `nil,
  message, kind` for a failed read reported in the document (`kind`:
  `"not_found"`, `"permission_denied"`, `"is_a_directory"`, or nil). Not
  with `root`.
- A Lua error raised in any of them is reported in the document and
  raised again by `parse` once it is done.

## Building

`nur test lua` runs the specs (`spec/*_spec.lua`) in an embedded Lua
5.4, and in LuaJIT (on Windows, from an MSVC developer prompt). The
module for an external Lua builds with `cargo build -p lsdocs-rst-lua
--release --no-default-features --features lua54,module` (or
`luajit,module`); the library (`liblsdocs_rst_lua.so`) is installed as
`lsdocs_rst.so`. On macOS, link with `-C link-arg=-undefined -C
link-arg=dynamic_lookup`. On Windows, the module binds `lua54.dll`
(`lua51.dll` for LuaJIT) by name (mlua's raw-dylib linking): no Lua
library is needed to build it.

## Known limitations

- Extensions run on the thread that called `parse`, inside it; a parse
  started on another thread (none can be, from Lua) would find no Lua
  state and report each Lua extension as raising an error.
- `luarocks install lsdocs-rst` installs the prebuilt module attached to the
  GitHub release (`lsdocs-rst-lua/v<version>`) for Lua 5.4 or LuaJIT on Linux
  (glibc) and macOS (x86_64 and aarch64) and Windows (x86_64, a Lua whose
  DLL is `lua54.dll`, or LuaJIT's `lua51.dll`). Elsewhere, or where that
  module does not load, it builds from source with cargo.
  `LSDOCS_PREBUILT=0` always builds from source.

[reStructuredText]: https://docutils.sourceforge.io/rst.html
[docutils]: https://docutils.sourceforge.io/
[mlua]: https://github.com/mlua-rs/mlua
