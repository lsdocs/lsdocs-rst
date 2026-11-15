"""Differential testing of lsdocs-rst against docutils.

Generates random documents from fragments of block-level markup and
sentences of inline markup, parses each with docutils and with lsdocs-rst
(through the crate's `pformat` example), and compares the pseudo-XML.
docutils gets a time limit per document (some inputs make its transforms
loop forever); documents over it are skipped and counted.

Run through `nur fuzz rst`, which builds the example and pins the docutils
version. Mismatches are written to `<work>/mismatch-<n>.{rst,want,got}`.
"""

import argparse
import io
import multiprocessing
import random
import subprocess
from pathlib import Path

from docutils.core import publish_doctree
from docutils.frontend import get_default_settings
from docutils.parsers.rst import Parser, roles
from docutils.utils import new_document

FRAGMENTS = [
    "Para text.", "Another line here", "", "", "", "    indented text",
    "  two spaces", "- item", "* star item", "+ plus", "1. one", "2. two",
    "3. three", "#. auto", "a) alpha", "b) beta", "(i) roman",
    "(ii) roman two", "IV. upper", ":field: value", ":f:", ":Other: x",
    "-o  option", "--long=x  desc", "-a, --all  both", "/V  dos",
    ">>> code", "... more", "| line", "|   deep", "|", ".. comment", "..",
    ".. [1] fn", ".. [#] auto", ".. [#lbl] named", ".. [*] sym",
    ".. [CIT] cite", ".. _t: x.html", ".. _t:", ".. __: y.html",
    "__ z.html", ".. |s| nosuch:: x", ".. |s| text", ".. unknown:: arg",
    "   :opt: v", "Title", "Title Two", "=====", "-----", "~~~~~~~~", "===",
    "--", "##########", "Term", "Term : cls", "    def", "Para::", "::",
    "Text ::", "> quoted", "-- Attribution", "    -- attr", "\ttabbed",
    "Wide \u65e5\u672c\u8a9e", "==============",
    "=====\nTitle\n=====", "-----\n Sub\n-----", "~~~~\nT\n~~~~",
    "Term : a : b", "back\\slash", "esc\\ aped", ":a\\:b: c",
    "\\- not bullet", ">> quoted", ">", "iii. roman", "v. five", "I. one",
    "(z) last", "0. zero", "10. ten", "#) auto", "- ", "* ", "-- ",
    "--- Dash", "\u2014 em", "....", ".. _a: b_", ".. _`p q`: r_",
    ".. __: anon_", ".. [#] ", ".. [2]", ".. |x|", ".. |y| nosuch::",
    "   continued", "      deep", "\t- tab bullet", "| ", "|  |",
    "Text\n\n::\n\n    lit", "Para::\n\n> q\n> r",
    # Tables, whole and in pieces.
    "+---+---+\n| a | b |\n+---+---+", "+---+\n| x |\n+===+\n| y |\n+---+",
    "+-----+-----+\n| a   | b   |\n+-----+     +\n| c   | d   |\n+-----+-----+",
    "+------+\n| - li |\n| - st |\n+------+", "=== ===\na   b\n=== ===",
    "===  ===\nx    y\n---  ---\nz\n===  ===", "=====  =====\nA      B\n=====  =====\n1      2\n=====  =====",
    "+--+\n|a|\n+--+", "=====  =====", "+-----+-----+", "| a   | b   |",
    "a      b", "+=====+=====+", "-----  -----",
    # Directives, with options and content.
    ".. note:: Note text.", ".. note::", ".. warning::\n   :class: x\n\n   Body.",
    ".. admonition:: Title", ".. topic:: Topic", ".. sidebar:: Side\n   :subtitle: s",
    ".. rubric:: Rubric", ".. epigraph::", ".. compound::", ".. container:: a b",
    ".. parsed-literal::", ".. code:: python", ".. code::\n   :number-lines:",
    ".. math::", ".. line-block::", ".. image:: a.png", ".. image:: a.png\n   :width: 50%",
    ".. image:: a.png\n   :target: t_", ".. figure:: f.png", ".. figure:: f.png\n   :figwidth: 10",
    ".. |r| replace:: text", ".. |u| unicode:: U+00A9", ".. |i| image:: i.png",
    ".. raw:: html", ".. include:: x.rst", ".. title:: T", ".. class:: c", ".. class:: !",
    ".. meta::\n   :k: v", ".. role:: r", ".. role:: e(emphasis)", ".. default-role:: strong",
    ".. contents::", ".. contents:: Toc\n   :depth: 1", ".. sectnum::", ".. target-notes::",
    ".. header:: h", ".. footer:: f", ".. rst-class:: c", ".. code-block:: c",
    "   :name: n", "   :class: k", "   :nosuch: v", "   :depth: x",
    ".. table:: T", ".. table::\n   :widths: auto", ".. list-table::",
    ".. list-table:: L\n   :header-rows: 1", ".. csv-table::", ".. csv-table:: C\n   :header: a, b",
    "   a, b, c", "   \"q, r\", s", "   * - x\n     - y", "   :widths: 1 2", "   :stub-columns: 1",
]
PREFIXES = ["", "", "", "", "  ", "    ", "   "]

# Words for sentences of inline markup.
WORDS = [
    "word", "text", "and", "of", ",", ".", ";", ":", "(", ")", '"', "'",
    "*em*", "**strong**", "``lit``", "`interp`", "ref_", "anon__",
    "`phrase ref`_", "`a <http://x.example.com>`_", "`b <alias_>`_",
    "`<http://y.example.com>`__", "[1]_", "[#]_", "[#n]_", "[*]_", "[CIT]_",
    "|sub|", "|sub|_", "|sub|__", "_`tgt`", "_`Tgt`", ":emphasis:`r`",
    ":nosuch:`r`", "`x`:strong:", ":code:`c\\d`", ":math:`y`", ":pep:`8`",
    ":rfc:`1`", ":raw:`r`", ":index:`i`", ":t:`t`", "http://example.com",
    "https://a.example.com/p?q=1#f", "me@example.com", "foo:bar", "a*b",
    "*", "**", "`", "``", "|", "_", "__", "\\*", "(*)", "'*'", "<*>", "x_",
    "*a", "b*", "**c", "d**", "`e", "f`", "``g", "h``", "*a b*", "``a b``",
    "PEP 8", "RFC 2822", chr(0x65E5) + chr(0x672C), chr(0x2014),
    chr(0xAB) + "*" + chr(0xBB), "\\", "\\ ", "e.g.", "end_", "_start",
    "x__", "[2]", "[x]_", "`a\nb`_", "*c\nd*",
]


def sentence(rng: random.Random) -> str:
    out = ""
    for _ in range(rng.randint(1, 8)):
        out += rng.choice(WORDS) + rng.choice([" ", " ", " ", ""])
    return out.strip() or "word"


def document(rng: random.Random) -> str:
    lines = []
    for _ in range(rng.randint(1, 60)):
        prefix = rng.choice(PREFIXES) if rng.random() < 0.45 else ""
        if rng.random() < 0.4:
            marker = rng.choice(["", "", "", "- ", "1. ", ":f: ", "| ", ".. [1] "])
            lines.append(prefix + marker + sentence(rng))
        else:
            lines.append(prefix + rng.choice(FRAGMENTS))
    return "\n".join(lines) + "\n"


# Seconds docutils may take on one document.
TIMEOUT = 10


def docutils_pxml(text: str) -> str:
    settings = get_default_settings(Parser)
    settings.warning_stream = io.StringIO()
    settings.report_level = 5
    settings.halt_level = 5
    settings.syntax_highlight = "none"  # lsdocs-rst does not highlight code
    settings.file_insertion_enabled = False  # nor read files by default
    roles._roles.clear()  # custom roles are process-wide in docutils
    doc = new_document("test data", settings)
    Parser().parse(text, doc)
    return doc.pformat()


def docutils_transformed(text: str) -> str:
    roles._roles.clear()
    doc = publish_doctree(text, source_path="test data", settings_overrides={
        "syntax_highlight": "none", "file_insertion_enabled": False,
        "report_level": 5, "halt_level": 5, "warning_stream": io.StringIO(),
    })
    return doc.pformat()


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("binary")
    ap.add_argument("work", type=Path)
    ap.add_argument("--count", type=int, default=1000)
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--transforms", action="store_true",
                    help="compare transformed trees (publish_doctree)")
    args = ap.parse_args()

    rng = random.Random(args.seed)
    inputs, outputs = args.work / "in", args.work / "out"
    for d in (inputs, outputs):
        d.mkdir(parents=True, exist_ok=True)
        for old in d.iterdir():
            old.unlink()
    for old in args.work.glob("mismatch-*"):
        old.unlink()

    texts = {}
    for i in range(args.count):
        texts[f"c{i}"] = text = document(rng)
        (inputs / f"c{i}.rst").write_bytes(text.encode())
    files = [str(inputs / f"{name}.rst") for name in texts]
    for i in range(0, len(files), 40):  # keep command lines short
        chunk = files[i:i + 40]
        flags = ["--transforms"] if args.transforms else []
        if subprocess.run([args.binary, "--out", str(outputs)] + flags + chunk,
                          capture_output=True).returncode != 0:
            # A crash: run the files one by one; a crashing one has no output.
            for f in chunk:
                subprocess.run([args.binary, "--out", str(outputs)] + flags + [f], capture_output=True)

    mismatches = skipped = 0
    convert = docutils_transformed if args.transforms else docutils_pxml
    pool = multiprocessing.Pool(1)
    for name, text in texts.items():
        try:
            want = pool.apply_async(convert, (text,)).get(timeout=TIMEOUT)
        except multiprocessing.TimeoutError:
            pool.terminate()
            pool = multiprocessing.Pool(1)
            print(f"skipped {name}: docutils did not finish in {TIMEOUT} s")
            skipped += 1
            continue
        out = outputs / f"{name}.pxml"
        got = out.read_bytes().decode() if out.exists() else "(lsdocs-rst panicked)"
        if want != got:
            base = args.work / f"mismatch-{mismatches}"
            base.with_suffix(".rst").write_bytes(text.encode())
            base.with_suffix(".want").write_bytes(want.encode())
            base.with_suffix(".got").write_bytes(got.encode())
            mismatches += 1
    pool.terminate()
    compared = args.count - skipped
    print(f"seed {args.seed}: {compared - mismatches} of {compared} match docutils"
          + (f" ({skipped} skipped)" if skipped else ""))
    if mismatches:
        print(f"mismatches written to {args.work}/mismatch-*")
        raise SystemExit(1)


if __name__ == "__main__":
    main()
