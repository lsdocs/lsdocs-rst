"""Writes docutils' pseudo-XML for every `.rst` case under a directory.

The bridge between `nur gen rst` and docutils: for each `<name>.rst` it
parses the text with docutils' reStructuredText parser (no transforms, as
`lsdocs-rst` does not run them yet) and writes the tree as `<name>.pxml`.
The Rust conformance tests (crates/rst/tests/conformance.rs) compare
against these files, so they run without Python.

Run through `nur gen rst`, which pins the docutils version.
"""

import io
import sys
from pathlib import Path

import docutils
from docutils.frontend import get_default_settings
from docutils.parsers.rst import Parser
from docutils.utils import new_document


def pseudoxml(text: str) -> str:
    parser = Parser()
    settings = get_default_settings(Parser)
    settings.warning_stream = io.StringIO()  # problems stay in the tree
    settings.report_level = 5
    settings.halt_level = 5
    document = new_document("test data", settings)
    parser.parse(text, document)
    return document.pformat()


def main(root: Path) -> None:
    cases = sorted(root.rglob("*.rst"))
    for case in cases:
        text = case.read_bytes().decode("utf-8")
        case.with_suffix(".pxml").write_bytes(pseudoxml(text).encode("utf-8"))
    print(f"docutils {docutils.__version__}: wrote {len(cases)} expected trees")


if __name__ == "__main__":
    main(Path(sys.argv[1]))
