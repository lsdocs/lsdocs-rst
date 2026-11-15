"""Writes docutils' pseudo-XML for every `.rst` case under a directory.

The bridge between `nur gen rst` and docutils: for each `<name>.rst` it
writes two trees, `<name>.pxml` from docutils' reStructuredText parser
alone and `<name>.tpxml` after docutils' transforms too (what
`publish_doctree` returns). Cases named `file_*` may read files: they
run with file insertion on and the cases directory as the working
directory (included files there end in `.txt`). The Rust conformance tests
(crates/rst/tests/conformance.rs) compare against these files, so they
run without Python.

Run through `nur gen rst`, which pins the docutils version.
"""

import io
import os
import sys
from pathlib import Path

import docutils
from docutils.core import publish_doctree
from docutils.frontend import get_default_settings
from docutils.parsers.rst import Parser, roles
from docutils.utils import new_document

# What lsdocs-rst does differently by default: no syntax highlighting and
# no file access.
OVERRIDES = {
    "syntax_highlight": "none",
    "file_insertion_enabled": False,
    "report_level": 5,
    "halt_level": 5,
}


def pseudoxml(text: str, files: bool) -> str:
    parser = Parser()
    settings = get_default_settings(Parser)
    settings.warning_stream = io.StringIO()  # problems stay in the tree
    for key, value in OVERRIDES.items():
        setattr(settings, key, value)
    settings.file_insertion_enabled = files
    roles._roles.clear()  # custom roles are process-wide in docutils
    document = new_document("test data", settings)
    parser.parse(text, document)
    return document.pformat()


def transformed(text: str, files: bool) -> str:
    roles._roles.clear()
    document = publish_doctree(
        text,
        source_path="test data",
        settings_overrides={
            **OVERRIDES,
            "file_insertion_enabled": files,
            "warning_stream": io.StringIO(),
        },
    )
    return document.pformat()


def main(root: Path) -> None:
    cases = sorted(root.rglob("*.rst"))
    for case in cases:
        text = case.read_bytes().decode("utf-8")
        files = case.stem.startswith("file_")
        cwd = os.getcwd()
        os.chdir(case.parent)
        try:
            pxml, tpxml = pseudoxml(text, files), transformed(text, files)
        finally:
            os.chdir(cwd)
        case.with_suffix(".pxml").write_bytes(pxml.encode("utf-8"))
        case.with_suffix(".tpxml").write_bytes(tpxml.encode("utf-8"))
    print(f"docutils {docutils.__version__}: wrote {len(cases)} cases' expected trees")


if __name__ == "__main__":
    main(Path(sys.argv[1]))
