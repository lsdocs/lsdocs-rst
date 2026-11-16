"""Directives, roles and file loaders written in Python, and conversion to
docutils."""

import pytest

import lsdocs_rst
from lsdocs_rst import Directive, DirectiveError, Element, Text


class Shout(Directive):
    required_arguments = 1
    final_argument_whitespace = True
    option_spec = {"class": "class_option", "level": ["low", "high"], "loud": "flag"}

    def run(self, ctx):
        nodes, messages = ctx.parse_inline(ctx.arguments[0].upper())
        paragraph = Element("paragraph", {"classes": ctx.options.get("class", [])}, nodes)
        if "loud" in ctx.options:
            paragraph.attributes["loud"] = ctx.options.get("level", "low")
        return [paragraph, *messages]


class Box(Directive):
    """Parses its content into a container, like docutils' `container`."""

    has_content = True

    def run(self, ctx):
        if not ctx.content:
            raise DirectiveError(f'The "{ctx.name}" directive is empty.', level=2)
        return [ctx.parse_content(Element("container", {"classes": ["box"]}))]


def test_directive_with_options() -> None:
    parser = lsdocs_rst.Parser().directive("shout", Shout)
    doc = parser.parse(".. shout:: hello *world*\n   :class: Big One\n   :loud:\n   :level: HIGH\n")
    assert doc.pformat() == (
        "<document>\n"
        '    <paragraph classes="big one" loud="high">\n'
        "        HELLO \n"
        "        <emphasis>\n"
        "            WORLD\n"
    )
    # A bad option value is reported like docutils' own options.
    doc = parser.parse(".. shout:: x\n   :level: medium\n")
    [message] = doc.findall("system_message")
    assert message["type"] == "ERROR"
    assert 'invalid option value: (option: "level"; value: \'medium\')' in message.astext()


def test_nested_python_directives() -> None:
    parser = lsdocs_rst.Parser().directive("box", Box).directive("shout", Shout)
    doc = parser.parse(".. box::\n\n   .. box::\n\n      .. shout:: deep\n\n   Para.\n")
    assert [e.kind for e in doc.traverse()] == [
        "document", "container", "container", "paragraph", "paragraph",
    ]
    assert doc.astext() == "DEEPPara."


def test_directive_errors_are_messages() -> None:
    doc = lsdocs_rst.Parser().directive("box", Box).parse(".. box::\n")
    [message] = doc.findall("system_message")
    assert message["type"] == "WARNING"
    assert message.astext().startswith('The "box" directive is empty.')


def test_exceptions_propagate() -> None:
    class Broken(Directive):
        def run(self, ctx):
            raise KeyError("oops")

    with pytest.raises(KeyError, match="oops"):
        lsdocs_rst.Parser().directive("broken", Broken).parse(".. broken::\n")


def test_context_is_dead_after_run() -> None:
    kept = []

    class Keep(Directive):
        def run(self, ctx):
            kept.append(ctx)
            return []

    lsdocs_rst.Parser().directive("keep", Keep).parse(".. keep::\n")
    with pytest.raises(RuntimeError, match="returned"):
        kept[0].parse_inline("x")


def test_roles() -> None:
    def kbd(ctx):
        if ctx.text == "?":
            raise DirectiveError(f"unknown key in {ctx.rawsource}")
        return [Element("literal", {"classes": ["kbd"]}, [Text(ctx.text)], ctx.span)]

    parser = lsdocs_rst.Parser().role("kbd", kbd)
    doc = parser.parse("Press :kbd:`Ctrl\\+C`.\n")
    [literal] = doc.findall("literal")
    assert (literal.astext(), literal["classes"], literal.span) == ("Ctrl+C", ["kbd"], (6, 20))
    # An error becomes a problematic element and a message, as in docutils.
    doc = parser.parse("Press :kbd:`?`.\n")
    [problematic] = doc.findall("problematic")
    [message] = doc.findall("system_message")
    assert problematic.astext() == ":kbd:`?`"
    assert message.astext() == "unknown key in :kbd:`?`"


def test_file_loader_functions() -> None:
    files = {"a.txt": b"From *a*.\n", "dir.txt": IsADirectoryError(21, "Is a directory")}

    def read(path):
        value = files.get(path, FileNotFoundError(2, "No such file or directory"))
        if isinstance(value, Exception):
            raise value
        return value

    parser = lsdocs_rst.Parser(files=read)
    assert parser.parse(".. include:: a.txt\n").astext() == "From a."
    message = parser.parse(".. include:: nope.txt\n").astext()
    assert "[Errno 2] No such file or directory: 'nope.txt'" in message
    # Other exceptions are bugs: raised after the parse.
    with pytest.raises(ZeroDivisionError):
        lsdocs_rst.Parser(files=lambda path: 1 / 0).parse(".. include:: a.txt\n")


def test_to_docutils() -> None:
    pytest.importorskip("docutils")
    from docutils.core import publish_from_doctree

    text = "Title\n=====\n\nSee `Python <https://python.org>`_ [#]_.\n\n.. [#] A note.\n"
    doc = lsdocs_rst.parse(text, source="test data", transforms=True)
    document = doc.to_docutils()
    assert document.pformat() == doc.pformat()
    html = publish_from_doctree(document, writer="html5").decode()
    assert '<a class="reference external" href="https://python.org">Python</a>' in html
