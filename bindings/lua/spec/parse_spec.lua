-- Parsing, the tree as plain tables, settings and writers.
-- Each spec file returns a table of test functions, run by tests/specs.rs.
local rst = require("lsdocs_rst")

local DOC = "Title\n=====\n\nHello, *world*.\n"

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

local function eq(actual, expected, what)
  if actual ~= expected then
    error(string.format("%s: expected %q, got %q", what, tostring(expected), tostring(actual)), 2)
  end
end

local tests = {}

function tests.module_example()
  local doc, messages = rst.parse(DOC, { transforms = true })
  eq(doc.kind, "document", "kind")
  eq(doc.attributes.title, "Title", "title")
  eq(#messages, 0, "messages")
  local emphasis = findall(doc, "emphasis")
  eq(#emphasis, 1, "emphasis count")
  eq(emphasis[1].children[1].text, "world", "emphasis text")
  -- Spans are 0-based byte offsets, end exclusive.
  eq(emphasis[1].span.start, 20, "span start")
  eq(emphasis[1].span["end"], 27, "span end")
  eq(DOC:sub(emphasis[1].span.start + 1, emphasis[1].span["end"]), "*world*", "span text")
  local head = '<document ids="title" names="title" title="Title">\n'
  eq(rst.pformat(doc):sub(1, #head), head, "pformat")
end

function tests.plain_tables()
  local doc = rst.parse("A *b*.\n")
  eq(getmetatable(doc), nil, "document metatable")
  eq(getmetatable(doc.children), nil, "children metatable")
  eq(doc.children[1].kind, "paragraph", "first child")
  eq(doc.children[1].children[1].text, "A ", "text node")
end

function tests.parser_object()
  local parser = rst.parser({ settings = { tab_width = 4 } })
  eq(parser:settings().tab_width, 4, "tab_width")
  local doc = parser:parse("x\n")
  eq(doc.children[1].kind, "paragraph", "parsed")
end

function tests.messages()
  -- Messages from the transforms (a missing target) come back apart.
  local _, messages = rst.parse("See missing_.\n", { transforms = true })
  eq(#messages, 1, "one message")
  eq(messages[1].kind, "system_message", "message kind")
  eq(messages[1].children[1].children[1].text, 'Unknown target name: "missing".', "message text")
end

function tests.round_trip_pformat()
  local doc = rst.parse(DOC)
  -- A tree edited in Lua goes back through the writers.
  doc.children[#doc.children + 1] =
    { kind = "paragraph", attributes = {}, children = { { text = "added" } } }
  local out = rst.pformat(doc)
  eq(out:find("added", 1, true) ~= nil, true, "edited tree")
end

function tests.to_xml()
  local xml = rst.to_xml(rst.parse("x\n"))
  eq(xml:find("<paragraph>x</paragraph>", 1, true) ~= nil, true, "xml")
end

function tests.helpers()
  eq(rst.make_id("Hello World!"), "hello-world", "make_id")
  eq(type(rst.default_settings().tab_width), "number", "default_settings")
  eq(type(rst.version), "string", "version")
end

function tests.bad_tree_errors()
  local ok = pcall(rst.pformat, { nonsense = true })
  eq(ok, false, "pformat rejects a non-node")
end

function tests.readme_example()
  local doc = rst.parse(DOC, { transforms = true })
  local emphasis = doc.children[2].children[2]
  eq(emphasis.kind, "emphasis", "kind")
  eq(DOC:sub(emphasis.span.start + 1, emphasis.span["end"]), "*world*", "span text")
end

return tests
