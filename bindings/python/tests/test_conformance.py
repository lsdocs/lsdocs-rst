"""The conformance cases of the Rust crate, through the Python package: each
case parses to docutils' trees (crates/rst/tests/cases, written by docutils
with `nur gen rst`)."""

import json
from pathlib import Path

import pytest

import lsdocs_rst

CASES = Path(__file__).resolve().parents[3] / "crates" / "rst" / "tests" / "cases"


def pending(name: str) -> set[str]:
    path = CASES / name
    if not path.exists():
        return set()
    lines = (line.strip() for line in path.read_text().splitlines())
    return {line for line in lines if line and not line.startswith("#")}


def parser(case: Path, transforms: bool) -> lsdocs_rst.Parser:
    settings_file = case.with_suffix(".settings.json")
    settings = json.loads(settings_file.read_text()) if settings_file.exists() else {}
    root = CASES if case.stem.startswith("file_") else None
    return lsdocs_rst.Parser(source="test data", transforms=transforms, root=root, **settings)


def expected(case: Path, suffix: str) -> str:
    return case.with_suffix(suffix).read_bytes().decode("utf-8")


CASE_FILES = sorted(CASES.glob("*.rst"))


@pytest.mark.parametrize("case", CASE_FILES, ids=lambda p: p.stem)
def test_parser(case: Path) -> None:
    if case.stem in pending("PENDING"):
        pytest.skip("pending in the Rust crate")
    doc = parser(case, False).parse(case.read_text(encoding="utf-8"))
    assert doc.pformat() == expected(case, ".pxml")


@pytest.mark.parametrize("case", CASE_FILES, ids=lambda p: p.stem)
def test_transforms_and_xml(case: Path) -> None:
    if case.stem in pending("PENDING-TRANSFORMS"):
        pytest.skip("pending in the Rust crate")
    doc = parser(case, True).parse(case.read_text(encoding="utf-8"))
    assert doc.pformat() == expected(case, ".tpxml")
    assert doc.to_xml(indents=True) == expected(case, ".xml")


def test_there_are_cases() -> None:
    assert len(CASE_FILES) > 50
