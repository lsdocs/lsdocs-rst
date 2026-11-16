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

[reStructuredText]: https://docutils.sourceforge.io/rst.html
[docutils]: https://docutils.sourceforge.io/
