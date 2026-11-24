-- Directives, roles and file loaders written in Lua (the Lua counterpart
-- of the Python package's tests/test_extensions.py).
local rst = require("lsdocs_rst")

local function eq(actual, expected, what)
  if actual ~= expected then
    error(string.format("%s: expected %q, got %q", what, tostring(expected), tostring(actual)), 2)
  end
end

local function contains(haystack, needle, what)
  if not tostring(haystack):find(needle, 1, true) then
    error(string.format("%s: %q not in %q", what, needle, tostring(haystack)), 2)
  end
end

-- Every element of `kind` below `node`, in document order.
local function findall(node, kind, out)
  out = out or {}
  if node.kind == kind then
    out[#out + 1] = node
  end
  for _, child in ipairs(node.children or {}) do
    findall(child, kind, out)
  end
  return out
end

-- The kinds of every element below `node`, in document order.
local function kinds(node, out)
  out = out or {}
  if node.kind then
    out[#out + 1] = node.kind
  end
  for _, child in ipairs(node.children or {}) do
    kinds(child, out)
  end
  return out
end

local function astext(node)
  if node.text then
    return node.text
  end
  local out = {}
  for _, child in ipairs(node.children or {}) do
    out[#out + 1] = astext(child)
  end
  return table.concat(out)
end

local function append(list, more)
  for _, v in ipairs(more) do
    list[#list + 1] = v
  end
  return list
end

local shout = {
  required_arguments = 1,
  final_argument_whitespace = true,
  options = { class = "class_option", level = { "low", "high" }, loud = "flag" },
  run = function(ctx)
    local nodes, messages = ctx:parse_inline(ctx.arguments[1]:upper())
    local paragraph = {
      kind = "paragraph",
      attributes = { classes = ctx.options.class or {} },
      children = nodes,
    }
    if ctx.options.loud then
      paragraph.attributes.loud = ctx.options.level or "low"
    end
    return append({ paragraph }, messages)
  end,
}

-- Parses its content into a container, like docutils' `container`.
local box = {
  has_content = true,
  run = function(ctx)
    if #ctx.content == 0 then
      return nil, 'The "' .. ctx.name .. '" directive is empty.', 2
    end
    local container = { kind = "container", attributes = { classes = { "box" } }, children = {} }
    return { ctx:parse_content(container) }
  end,
}

local tests = {}

function tests.directive_with_options()
  local parser = rst.parser({ directives = { shout = shout } })
  local doc =
    parser:parse(".. shout:: hello *world*\n   :class: Big One\n   :loud:\n   :level: HIGH\n")
  eq(
    rst.pformat(doc),
    "<document>\n"
      .. '    <paragraph classes="big one" loud="high">\n'
      .. "        HELLO \n"
      .. "        <emphasis>\n"
      .. "            WORLD\n",
    "pformat"
  )
  -- A bad option value is reported like docutils' own options.
  doc = parser:parse(".. shout:: x\n   :level: medium\n")
  local messages = findall(doc, "system_message")
  eq(#messages, 1, "messages")
  eq(messages[1].attributes.type, "ERROR", "type")
  contains(
    astext(messages[1]),
    "invalid option value: (option: \"level\"; value: 'medium')",
    "text"
  )
end

function tests.nested_lua_directives()
  local parser = rst.parser({ directives = { box = box, shout = shout } })
  local doc = parser:parse(".. box::\n\n   .. box::\n\n      .. shout:: deep\n\n   Para.\n")
  eq(table.concat(kinds(doc), " "), "document container container paragraph paragraph", "kinds")
  eq(astext(doc), "DEEPPara.", "text")
end

function tests.directive_errors_are_messages()
  local doc = rst.parse(".. box::\n", { directives = { box = box } })
  local messages = findall(doc, "system_message")
  eq(#messages, 1, "messages")
  eq(messages[1].attributes.type, "WARNING", "type")
  contains(astext(messages[1]), 'The "box" directive is empty.', "text")
end

function tests.errors_propagate()
  local broken = {
    run = function()
      error("oops")
    end,
  }
  local ok, err = pcall(rst.parse, ".. broken::\n", { directives = { broken = broken } })
  eq(ok, false, "fails")
  contains(err, "oops", "error")
end

function tests.context_is_dead_after_run()
  local kept
  local keep = {
    run = function(ctx)
      kept = ctx
      return {}
    end,
  }
  rst.parse(".. keep::\n", { directives = { keep = keep } })
  eq(kept.name, "keep", "data stays")
  local ok = pcall(kept.parse_inline, kept, "x")
  eq(ok, false, "methods fail after run")
end

function tests.roles()
  local function kbd(ctx)
    if ctx.text == "?" then
      return nil, "unknown key in " .. ctx.rawsource
    end
    return {
      {
        kind = "literal",
        attributes = { classes = { "kbd" } },
        children = { ctx.text },
        span = ctx.span,
      },
    }
  end
  local parser = rst.parser({ roles = { kbd = kbd } })
  local doc = parser:parse("Press :kbd:`Ctrl\\+C`.\n")
  local literal = findall(doc, "literal")[1]
  eq(astext(literal), "Ctrl+C", "text")
  eq(literal.attributes.classes[1], "kbd", "classes")
  eq(literal.span.start, 6, "span start")
  eq(literal.span["end"], 20, "span end")
  -- An error becomes a problematic element and a message, as in docutils.
  doc = parser:parse("Press :kbd:`?`.\n")
  eq(astext(findall(doc, "problematic")[1]), ":kbd:`?`", "problematic")
  eq(astext(findall(doc, "system_message")[1]), "unknown key in :kbd:`?`", "message")
end

function tests.file_loader_functions()
  local function read(path)
    if path == "a.txt" then
      return "From *a*.\n"
    end
    return nil, "No such file or directory", "not_found"
  end
  local parser = rst.parser({ files = read })
  eq(astext(parser:parse(".. include:: a.txt\n")), "From a.", "include")
  contains(
    astext(parser:parse(".. include:: nope.txt\n")),
    "No such file or directory",
    "missing file"
  )
  -- Errors raised are bugs: raised after the parse.
  local ok, err = pcall(rst.parse, ".. include:: a.txt\n", {
    files = function()
      error("broken loader")
    end,
  })
  eq(ok, false, "raises")
  contains(err, "broken loader", "error")
  ok = pcall(rst.parser, { root = ".", files = read })
  eq(ok, false, "root and files together")
end

function tests.parse_inside_a_directive()
  -- A directive can parse with another parser; each parse keeps its own
  -- extensions' errors.
  local inner = {
    run = function()
      local ok = pcall(rst.parse, ".. bad::\n", {
        directives = {
          bad = {
            run = function()
              error("inner")
            end,
          },
        },
      })
      return { { kind = "paragraph", attributes = {}, children = { tostring(ok) } } }
    end,
  }
  local doc = rst.parse(".. inner::\n", { directives = { inner = inner } })
  eq(astext(doc), "false", "inner parse failed alone")
end

return tests
