"""A fast reStructuredText parser that builds docutils' document tree.

lsdocs-rst is a Rust port of docutils' parser and transforms. It builds
the tree docutils builds (the same node kinds and attributes, printed as
the same pseudo-XML) and records where in the source each node came from::

    import lsdocs_rst

    doc = lsdocs_rst.parse("Title\\n=====\\n\\nHello, *world*.\\n", transforms=True)
    print(doc.pformat())
    for emphasis in doc.findall("emphasis"):
        print(emphasis.astext(), emphasis.span)   # world (20, 27)

Settings are docutils' (``tab_width``, ``pep_references``, ...), given as
keyword arguments. Directives and roles can be written in Python (see
:class:`Directive` and :meth:`Parser.role`), and directives such as
``include`` read files only through a loader you give (``root=`` or
``files=``). With docutils installed, :meth:`Element.to_docutils` turns a
tree into a ``docutils.nodes.document`` for docutils' writers.
"""

from __future__ import annotations

import os
from collections.abc import Callable, Iterator, Mapping, Sequence
from typing import Any, Union

from . import _native

__all__ = [
    "Directive",
    "DirectiveContext",
    "DirectiveError",
    "Element",
    "Node",
    "Parser",
    "RoleContext",
    "Text",
    "default_settings",
    "make_id",
    "parse",
]

Span = tuple[int, int]

#: Message levels.
INFO, WARNING, ERROR, SEVERE = 1, 2, 3, 4


def _span_from(data: Mapping[str, int] | None) -> Span | None:
    return None if data is None else (data["start"], data["end"])


def _span_to(span: Span | None) -> dict[str, int] | None:
    return None if span is None else {"start": span[0], "end": span[1]}


class Text:
    """Text in the tree."""

    __slots__ = ("text", "span")

    def __init__(self, text: str, span: Span | None = None) -> None:
        self.text = text
        #: ``(start, end)`` byte offsets in the parsed text, if known.
        self.span = span

    def astext(self) -> str:
        return self.text

    def to_dict(self) -> dict[str, Any]:
        data: dict[str, Any] = {"text": self.text}
        if self.span is not None:
            data["span"] = _span_to(self.span)
        return data

    def pformat(self) -> str:
        return _native.pformat(self.to_dict())

    def __eq__(self, other: object) -> bool:
        return isinstance(other, Text) and (self.text, self.span) == (other.text, other.span)

    def __repr__(self) -> str:
        return f"Text({self.text!r})"


class Element:
    """An element: a docutils node kind (``"paragraph"``, ``"section"``,
    ...), attributes, children and a span.

    Like a docutils node, ``element[i]`` is a child and ``element["ids"]``
    an attribute.
    """

    __slots__ = ("kind", "attributes", "children", "span")

    def __init__(
        self,
        kind: str,
        attributes: dict[str, Any] | None = None,
        children: list[Node] | None = None,
        span: Span | None = None,
    ) -> None:
        #: The docutils tag name.
        self.kind = kind
        self.attributes = {} if attributes is None else attributes
        self.children = [] if children is None else children
        #: ``(start, end)`` byte offsets in the parsed text, if known.
        self.span = span

    @property
    def tagname(self) -> str:
        """The kind, under docutils' name for it."""
        return self.kind

    # -- conversion ----------------------------------------------------------

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> Element:
        """An element from its dict form (see :meth:`to_dict`)."""
        return _node_from(data)  # type: ignore[return-value]

    def to_dict(self) -> dict[str, Any]:
        """The dict form: ``kind``, and ``attributes``, ``children`` and
        ``span`` when present. Text is ``{"text": ..., "span": ...}``. The
        same form as the Rust crate's serde feature, so it round-trips
        through JSON."""
        data: dict[str, Any] = {"kind": self.kind}
        if self.attributes:
            data["attributes"] = self.attributes
        if self.children:
            data["children"] = [c.to_dict() for c in self.children]
        if self.span is not None:
            data["span"] = _span_to(self.span)
        return data

    def pformat(self) -> str:
        """Pseudo-XML, exactly as docutils' ``pformat()`` prints it."""
        return _native.pformat(self.to_dict())

    def to_xml(
        self,
        *,
        newlines: bool = False,
        indents: bool = False,
        xml_declaration: bool = True,
        doctype_declaration: bool = True,
    ) -> str:
        """Docutils XML, as ``docutils --writer=xml`` writes it."""
        options = {
            "newlines": newlines,
            "indents": indents,
            "xml_declaration": xml_declaration,
            "doctype_declaration": doctype_declaration,
        }
        return _native.to_xml(self.to_dict(), options)

    def to_docutils(self, settings: Any = None) -> Any:
        """The tree as docutils nodes (needs docutils). A ``document``
        element becomes a ``docutils.nodes.document`` with ``settings``
        (docutils' defaults if ``None``), ready for docutils' writers."""
        from ._docutils import to_docutils

        return to_docutils(self, settings)

    # -- reading -------------------------------------------------------------

    def astext(self) -> str:
        """The text of all descendants, concatenated."""
        return "".join(c.astext() for c in self.children)

    def traverse(self) -> Iterator[Element]:
        """This element and its descendant elements, in document order."""
        stack: list[Element] = [self]
        while stack:
            element = stack.pop()
            yield element
            stack.extend(reversed([c for c in element.children if isinstance(c, Element)]))

    def findall(self, kind: str | None = None) -> Iterator[Element]:
        """This element and its descendants of ``kind`` (all if ``None``),
        in document order, as docutils' ``findall``."""
        return (e for e in self.traverse() if kind is None or e.kind == kind)

    def get(self, name: str, default: Any = None) -> Any:
        return self.attributes.get(name, default)

    def __getitem__(self, key: int | str) -> Any:
        if isinstance(key, str):
            return self.attributes[key]
        return self.children[key]

    def __len__(self) -> int:
        return len(self.children)

    def __iter__(self) -> Iterator[Node]:
        return iter(self.children)

    def __eq__(self, other: object) -> bool:
        return isinstance(other, Element) and (
            self.kind,
            self.attributes,
            self.children,
            self.span,
        ) == (other.kind, other.attributes, other.children, other.span)

    def __repr__(self) -> str:
        return f"<Element {self.kind} with {len(self.children)} children>"


Node = Union[Element, Text]


def _node_from(data: Mapping[str, Any]) -> Node:
    if "kind" in data:
        return Element(
            data["kind"],
            dict(data.get("attributes") or {}),
            [_node_from(c) for c in data.get("children") or ()],
            _span_from(data.get("span")),
        )
    return Text(data["text"], _span_from(data.get("span")))


def _node_to(node: Node | str) -> dict[str, Any]:
    if isinstance(node, str):
        return {"text": node}
    if isinstance(node, (Element, Text)):
        return node.to_dict()
    raise TypeError(f"expected an Element, Text or str, not {type(node).__name__}")


# -- extensions ---------------------------------------------------------------


class DirectiveError(Exception):
    """Raised by a directive or role: reported in the document as a system
    message of ``level`` (1 info, 2 warning, 3 error, 4 severe), as
    docutils reports its own directives' errors."""

    def __init__(self, message: str, level: int = ERROR) -> None:
        super().__init__(message)
        self.message = message
        self.level = level


class DirectiveContext:
    """What a directive's :meth:`Directive.run` gets: what was written, and
    access to the parser."""

    def __init__(self, native: Any) -> None:
        self._native = native
        #: The directive name as written.
        self.name: str = native.name
        #: The arguments.
        self.arguments: list[str] = native.arguments
        #: The options, converted as their kinds say (``None`` for flags).
        self.options: dict[str, Any] = native.options
        #: The content lines, indentation removed.
        self.content: list[str] = native.content
        #: The line number of the directive's first line.
        self.lineno: int = native.lineno
        #: The directive's whole text.
        self.block_text: str = native.block_text
        #: Where the directive is in the source.
        self.span: Span | None = _span_from(native.span)
        #: Whether the directive is in a substitution definition.
        self.in_substitution: bool = native.in_substitution

    def parse_content(self, container: Element) -> Element:
        """Parses the content as body elements into ``container``."""
        return _node_from(self._native.parse_content(container.to_dict()))  # type: ignore[return-value]

    def parse_inline(self, text: str) -> tuple[list[Node], list[Element]]:
        """Parses ``text`` as inline markup: the nodes, and messages to put
        after the element holding them."""
        nodes, messages = self._native.parse_inline(text)
        return [_node_from(n) for n in nodes], [_node_from(m) for m in messages]  # type: ignore[misc]

    def message(self, level: int, text: str) -> Element:
        """A system message (1 info to 4 severe) for the directive's line."""
        return _node_from(self._native.message(level, text))  # type: ignore[return-value]


class Directive:
    """A directive: subclass it, set what it accepts, and implement
    :meth:`run`::

        class Shout(lsdocs_rst.Directive):
            required_arguments = 1
            final_argument_whitespace = True

            def run(self, ctx):
                nodes, messages = ctx.parse_inline(ctx.arguments[0].upper())
                return [lsdocs_rst.Element("paragraph", children=nodes), *messages]

        parser = lsdocs_rst.Parser().directive("shout", Shout)

    ``option_spec`` maps option names to docutils' conversion functions,
    by name (``"flag"``, ``"unchanged"``, ``"unchanged_required"``,
    ``"path"``, ``"uri"``, ``"int"``, ``"nonnegative_int"``,
    ``"percentage"``, ``"length_or_unitless"``,
    ``"length_or_percentage_or_unitless"``, ``"class_option"``), by the
    functions themselves (``docutils.parsers.rst.directives.flag``, ...),
    or to a list of choices.
    """

    required_arguments: int = 0
    optional_arguments: int = 0
    final_argument_whitespace: bool = False
    option_spec: Mapping[str, Any] = {}
    has_content: bool = False

    def run(self, ctx: DirectiveContext) -> Sequence[Node | str]:
        """The directive's nodes; raise :class:`DirectiveError` to report a
        problem instead."""
        raise NotImplementedError

    @classmethod
    def _spec(cls) -> dict[str, Any]:
        return {
            "required_arguments": cls.required_arguments,
            "optional_arguments": cls.optional_arguments,
            "final_argument_whitespace": cls.final_argument_whitespace,
            "has_content": cls.has_content,
            "options": {name: _option_kind(kind) for name, kind in cls.option_spec.items()},
        }


def _option_kind(kind: Any) -> Any:
    if isinstance(kind, str):
        return kind
    if kind is int:
        return "int"
    if callable(kind):
        return kind.__name__
    return list(kind)


class RoleContext:
    """What a role function gets."""

    def __init__(self, data: Mapping[str, Any]) -> None:
        #: The role name as written.
        self.name: str = data["name"]
        #: The text, escapes removed.
        self.text: str = data["text"]
        #: The text with its backslashes, as written.
        self.raw_text: str = data["raw_text"]
        #: The whole interpreted text as written, role included.
        self.rawsource: str = data["rawsource"]
        #: Options from the ``role`` directive that made the role.
        self.options: dict[str, Any] = data["options"]
        #: The line number the text is on.
        self.lineno: int = data["lineno"]
        #: Where the text is in the source.
        self.span: Span | None = _span_from(data["span"])


def _outcome(run: Callable[[], Sequence[Node | str]]) -> tuple[Any, ...]:
    try:
        nodes = run()
    except DirectiveError as error:
        return ("error", error.level, error.message)
    return ("ok", [_node_to(n) for n in nodes])


def _directive_runner(directive: Any) -> Callable[[Any], tuple[Any, ...]]:
    def run(native: Any) -> tuple[Any, ...]:
        instance = directive() if isinstance(directive, type) else directive
        return _outcome(lambda: instance.run(DirectiveContext(native)))

    return run


def _role_runner(role: Callable[[RoleContext], Sequence[Node | str]]) -> Callable[[Any], tuple[Any, ...]]:
    return lambda data: _outcome(lambda: role(RoleContext(data)))


# -- parsing ------------------------------------------------------------------


Loader = Callable[[str], Union[bytes, str]]


class Parser:
    """A configured parser. Each method returns a new parser, so a parser
    can be shared and specialized::

        parser = Parser(transforms=True, tab_width=4).role("shout", shout)
        doc = parser.parse(text)

    Keyword arguments: ``source`` (the document's name, used in messages),
    ``transforms`` (apply docutils' transforms, as ``publish_doctree``
    does), ``keep_unknown_directives`` (keep them as ``directive``
    elements instead of reporting them), ``root`` (let directives read
    files below this directory), ``files`` and ``urls`` (or read them with
    these functions, which return bytes or str and raise ``OSError``), and
    any docutils setting the parser honors (``tab_width``,
    ``pep_references``, ``id_prefix``, ...; see
    :func:`default_settings`).
    """

    def __init__(
        self,
        *,
        source: str | None = None,
        transforms: bool = False,
        keep_unknown_directives: bool = False,
        root: str | os.PathLike[str] | None = None,
        files: Loader | None = None,
        urls: Loader | None = None,
        **settings: Any,
    ) -> None:
        self._options: dict[str, Any] = {
            "source": source,
            "transforms": transforms,
            "keep_unknown_directives": keep_unknown_directives,
            "root": None if root is None else os.fspath(root),
            "files": files,
            "urls": urls,
        }
        self._settings = settings
        self._directives: dict[str, Any] = {}
        self._roles: dict[str, Any] = {}
        self._native: Any = None

    def _copy(self) -> Parser:
        new = Parser.__new__(Parser)
        new._options = dict(self._options)
        new._settings = dict(self._settings)
        new._directives = dict(self._directives)
        new._roles = dict(self._roles)
        new._native = None
        return new

    def directive(self, name: str, directive: type[Directive] | Directive) -> Parser:
        """A parser with ``directive`` (a :class:`Directive` subclass or
        instance) under ``name``, before any standard one of that name."""
        new = self._copy()
        new._directives[name] = directive
        return new

    def role(self, name: str, role: Callable[[RoleContext], Sequence[Node | str]]) -> Parser:
        """A parser with the role function ``role`` under ``name``. It gets
        a :class:`RoleContext` and returns inline nodes (or raises
        :class:`DirectiveError`)."""
        new = self._copy()
        new._roles[name] = role
        return new

    def _parser(self) -> Any:
        if self._native is None:
            files, urls = self._options["files"], self._options["urls"]
            self._native = _native.NativeParser(
                source=self._options["source"],
                transforms=self._options["transforms"],
                keep_unknown_directives=self._options["keep_unknown_directives"],
                settings=self._settings,
                directives={
                    name: (d._spec(), _directive_runner(d)) for name, d in self._directives.items()
                },
                roles={name: _role_runner(r) for name, r in self._roles.items()},
                root=self._options["root"],
                files=files,
                urls=urls,
            )
        return self._native

    @property
    def settings(self) -> dict[str, Any]:
        """The docutils settings in use."""
        return self._parser().settings()

    def parse(self, text: str) -> Element:
        """Parses ``text`` into a ``document`` element. Problems are
        ``system_message`` elements in the tree; parsing itself only fails
        if a Python directive, role or loader raises (other than
        :class:`DirectiveError` and ``OSError``)."""
        return self.parse_with_messages(text)[0]

    def parse_with_messages(self, text: str) -> tuple[Element, list[Element]]:
        """:meth:`parse`, plus the messages docutils' transforms report
        without putting them in the tree (``problematic`` elements refer to
        them by id)."""
        document, messages = self._parser().parse(text)
        return _node_from(document), [_node_from(m) for m in messages]  # type: ignore[return-value]


def parse(text: str, **options: Any) -> Element:
    """Parses ``text``; the keyword arguments are :class:`Parser`'s."""
    return Parser(**options).parse(text)


def make_id(name: str) -> str:
    """``name`` as docutils makes ids of names (``docutils.nodes.make_id``)."""
    return _native.make_id(name)


def default_settings() -> dict[str, Any]:
    """The docutils settings the parser honors, with docutils' defaults."""
    return _native.default_settings()
