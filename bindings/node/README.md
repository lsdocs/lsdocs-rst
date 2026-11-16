# @lsdocs/rst

A fast [reStructuredText] parser for Node.js that builds [docutils]'
document tree: the same node kinds and attributes, printed as the same
pseudo-XML, with the source position of every node. It is a Rust port of
docutils' parser and transforms
([lsdocs-rst](../../../crates/rst/README.md)), checked against docutils'
own output.

```js
const rst = require('@lsdocs/rst')

const doc = rst.parse('Title\n=====\n\nHello, *world*.\n', { transforms: true })
console.log(rst.pformat(doc))
for (const emphasis of rst.findAll(doc, 'emphasis')) {
  console.log(rst.astext(emphasis), emphasis.span) // world { start: 20, end: 27 }
}
```

- The tree is plain data: elements are `{ kind, attributes?, children?,
  span? }` and text is `{ text, span? }` (types in `index.d.ts`), so it
  goes through `JSON.stringify` unchanged.
- `parse(text, options)`, `parseWithMessages`, and `new Parser(options)`:
  `transforms: true` runs docutils' transforms as `publish_doctree` does;
  `settings` takes the docutils settings the parser honors under their
  docutils names (`{ tab_width: 4, pep_references: true }`).
- Output: `pformat(node)` (pseudo-XML) and `toXml(element, options)`
  (Docutils XML).
- Extensions: `directives` (objects with what they accept and a `run`
  function), `roles` (functions), and file access for `include` and
  friends (`root`, a directory, or `files`, a function). Nothing is read
  from disk without one.

```js
const shout = {
  requiredArguments: 1,
  finalArgumentWhitespace: true,
  run(ctx) {
    const [nodes, messages] = ctx.parseInline(ctx.arguments[0].toUpperCase())
    return [{ kind: 'paragraph', children: nodes }, ...messages]
  },
}
const parser = new rst.Parser({ directives: { shout } })
console.log(rst.pformat(parser.parse('.. shout:: hello *world*\n')))
```

Parsing is synchronous. A directive throws `DirectiveError` to report a
problem in the document; any other exception is thrown by `parse`.

## Known limitations

- **Parsing blocks the event loop**, like any synchronous call. Parse
  large documents in a worker thread (`node:worker_threads`) if that
  matters.
- **Extensions are synchronous.** A directive's `run`, a role and a file
  loader must return their result, not a Promise; with an `async`
  function, `parse` throws a `TypeError`.
- **A context only works during its directive's `run`.** Keep it and call
  `parseInline` (or another method) after `run` returns, and it throws,
  even from inside another directive of the same parse.
- **Exceptions are thrown after the parse.** A `DirectiveError` is
  reported in the document, as docutils reports its own errors. Any other
  exception from a directive, role or loader doesn't stop the parse: the
  directive is reported as having failed, the parse finishes, and `parse`
  then throws the first such exception.
- **Loader errors are told apart by `code`.** An error whose `code`
  starts with `E` (as Node.js system errors' do: `ENOENT`, `EACCES`,
  `EISDIR`, ...) is a failed read, reported in the document with Python's
  wording, as docutils would. Any other error is thrown from `parse`. A
  loader returning a string has it read as UTF-8.
- **Settings use docutils' names** (`tab_width`, not `tabWidth`), unlike
  the rest of the API, so docutils configurations carry over. Unknown
  settings throw.
- **Settings not supported:** `smart_quotes`,
  `character_level_inline_markup`, `language_code` (labels and
  bibliographic field names are English), `root_prefix`,
  `syntax_highlight` (`code` blocks are as with `none`),
  `expose_internals`, and the footer settings (`generator`, `datestamp`,
  `source_link`, `source_url`). Settings that only change docutils'
  output, not its tree (`report_level`, `halt_level`, `strip_classes`,
  `strip_elements_with_classes`), aren't settings here either.
- **PEP and RFC references** in plain text (`pep_references`,
  `rfc_references`) recognize ASCII digits only; docutils also takes other
  Unicode decimal digits.
- **`toXml` doesn't check raw XML** (a `raw` directive with the `xml`
  format) for well-formedness; docutils' writer warns about it.
- **File access goes only through the loader you give.** `root` reads
  relative paths below that directory, but absolute paths as they are, and
  it doesn't keep `..` from leaving it; pass a `files` function to
  restrict access. docutils' standard include files (`<isonum.txt>`, ...)
  are not bundled.

[reStructuredText]: https://docutils.sourceforge.io/rst.html
[docutils]: https://docutils.sourceforge.io/
