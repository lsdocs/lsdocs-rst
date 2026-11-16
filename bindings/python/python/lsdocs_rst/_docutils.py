"""Conversion of lsdocs-rst trees to docutils nodes (needs docutils)."""

from __future__ import annotations

from typing import TYPE_CHECKING, Any

from docutils import frontend, nodes, utils
from docutils.parsers.rst import Parser

if TYPE_CHECKING:
    from . import Element, Node


def to_docutils(element: Element, settings: Any = None) -> Any:
    """``element`` as docutils nodes; a ``document`` element becomes a
    ``docutils.nodes.document`` with ``settings`` (docutils' defaults for
    the reStructuredText parser if ``None``).

    Each kind becomes the docutils node class of that name, or a plain
    ``docutils.nodes.Element`` with that tag name if docutils has none.
    ``pending`` elements lose their transform, so run the transforms
    (``transforms=True``) before converting a tree that has them.
    """
    if element.kind == "document":
        if settings is None:
            settings = frontend.get_default_settings(Parser)
        document = utils.new_document(element.get("source", "<string>"), settings)
        _fill(document, element)
        _register(document, document)
        return document
    return _convert(element)


def _convert(node: Node) -> Any:
    from . import Text

    if isinstance(node, Text):
        return nodes.Text(node.text)
    cls = getattr(nodes, node.kind, None)
    if isinstance(cls, type) and issubclass(cls, nodes.Element):
        out = cls()
    else:
        out = nodes.Element()
        out.tagname = node.kind
    _fill(out, node)
    return out


def _fill(out: Any, element: Element) -> None:
    for name, value in element.attributes.items():
        out[name] = list(value) if isinstance(value, list) else value
    for child in element.children:
        out += _convert(child)


def _register(document: Any, root: Any) -> None:
    """Registers ids and names with the document, as parsing would have."""
    for node in root.findall(nodes.Element):
        for id_ in node["ids"]:
            document.ids.setdefault(id_, node)
        for name in node["names"]:
            if node["ids"]:
                document.nameids.setdefault(name, node["ids"][0])
                document.nametypes.setdefault(name, True)
