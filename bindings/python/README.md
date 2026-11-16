# lsdocs-rst for Python

A fast [reStructuredText] parser that builds [docutils]' document tree: the
same node kinds and attributes, printed as the same pseudo-XML, with the
source position of every node. It is a Rust port of docutils' parser and
transforms ([lsdocs-rst](../../../crates/rst/README.md)), checked against
docutils' own output.

```python
import lsdocs_rst

doc = lsdocs_rst.parse("Title\n=====\n\nHello, *world*.\n", transforms=True)
print(doc.pformat())
for emphasis in doc.findall("emphasis"):
    print(emphasis.astext(), emphasis.span)   # world (20, 27)
```

- `parse(text, **options)` and `Parser(**options)`: `transforms=True` runs
  docutils' transforms as `publish_doctree` does; any docutils setting the
  parser honors goes in as a keyword (`tab_width=4`,
  `pep_references=True`, ...; see `default_settings()`).
- The tree: `Element` (`kind`, `attributes`, `children`, `span`) and
  `Text` (`text`, `span`); `findall`, `traverse`, `astext`, and
  `element[i]` / `element["ids"]` as in docutils. `to_dict()` and
  `Element.from_dict()` give a JSON-ready form.
- Output: `pformat()` (pseudo-XML), `to_xml()` (Docutils XML), and with
  docutils installed (`lsdocs-rst[docutils]`), `to_docutils()`, a
  `docutils.nodes.document` for docutils' writers.
- Extensions: directives (subclass `Directive`, register with
  `Parser.directive`), roles (`Parser.role`), and file access for
  `include` and friends (`root=` a directory, or `files=` a function).
  Nothing is read from disk without one.

```python
class Shout(lsdocs_rst.Directive):
    required_arguments = 1
    final_argument_whitespace = True

    def run(self, ctx):
        nodes, messages = ctx.parse_inline(ctx.arguments[0].upper())
        return [lsdocs_rst.Element("paragraph", children=nodes), *messages]

parser = lsdocs_rst.Parser().directive("shout", Shout)
print(parser.parse(".. shout:: hello *world*\n").pformat())
```

Parsing runs in Rust with the GIL released.

## Known limitations

- **Directives run on another thread.** A Python directive's `run` is
  called on a short-lived worker thread, not the thread that called
  `parse()` (that is how its context can ask the running parse for
  `parse_content` and the rest without `unsafe` code). `threading.local`
  values and `contextvars` set by the caller are not visible inside
  `run`. Roles and file loaders run on the calling thread.
- **A context only works during its directive's `run`.** Keep it and call
  `parse_inline` (or another method) later, and it raises `RuntimeError`.
- **Exceptions are raised after the parse.** A `DirectiveError` (and an
  `OSError` from a loader) is reported in the document, as docutils
  reports its own errors. Any other exception from a directive, role or
  loader doesn't stop the parse: the directive is reported as having
  failed, the parse finishes, and `parse()` then raises the first such
  exception.
- **Don't share a `Parser` between threads that parse at once.** The
  exceptions above are collected per `Parser`, so with two concurrent
  `parse()` calls on the same parser, one call can raise an exception from
  the other's directive. Use one `Parser` per thread (creating one is
  cheap).
- **`to_docutils()` drops `pending` elements' transforms**, so convert a
  tree parsed with `transforms=True` if it has `pending` elements (from
  `contents`, `sectnum`, `target-notes` or `class`).
- **Settings not supported** (they raise `ValueError`, as unknown settings
  do): `smart_quotes`, `character_level_inline_markup`, `language_code`
  (labels and bibliographic field names are English), `root_prefix`,
  `syntax_highlight` (`code` blocks are as with `none`),
  `expose_internals`, and the footer settings (`generator`, `datestamp`,
  `source_link`, `source_url`). Settings that only change docutils'
  output, not its tree (`report_level`, `halt_level`, `strip_classes`,
  `strip_elements_with_classes`), aren't settings here either.
- **PEP and RFC references** in plain text (`pep_references`,
  `rfc_references`) recognize ASCII digits only; docutils also takes other
  Unicode decimal digits.
- **`to_xml()` doesn't check raw XML** (a `raw` directive with the `xml`
  format) for well-formedness; docutils' writer warns about it.
- **File access goes only through the loader you give.** `root=` reads
  relative paths below that directory, but absolute paths as they are, and
  it doesn't keep `..` from leaving it; pass a `files=` function to
  restrict access. docutils' standard include files (`<isonum.txt>`, ...)
  are not bundled.

[reStructuredText]: https://docutils.sourceforge.io/rst.html
[docutils]: https://docutils.sourceforge.io/
