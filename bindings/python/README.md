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

[reStructuredText]: https://docutils.sourceforge.io/rst.html
[docutils]: https://docutils.sourceforge.io/
