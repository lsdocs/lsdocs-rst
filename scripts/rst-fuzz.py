"""Differential testing of lsdocs-rst against docutils.

Generates random documents from fragments of block-level markup, parses
each with docutils and with lsdocs-rst (through the crate's `pformat`
example), and compares the pseudo-XML. Documents whose docutils tree has
inline markup elements are skipped while lsdocs-rst does not parse inline
markup.

Run through `nur fuzz rst`, which builds the example and pins the docutils
version. Mismatches are written to `<work>/mismatch-<n>.{rst,want,got}`.
"""

import argparse
import io
import random
import subprocess
from pathlib import Path

from docutils.frontend import get_default_settings
from docutils.parsers.rst import Parser
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
]
PREFIXES = ["", "", "", "", "  ", "    ", "   "]

# Elements only inline markup produces.
INLINE = (
    "<substitution_reference", "<reference", "<footnote_reference",
    "<citation_reference", "<emphasis", "<strong", "<literal>",
    "<problematic", "<title_reference", "<inline",
)


def document(rng: random.Random) -> str:
    lines = []
    for _ in range(rng.randint(1, 60)):
        prefix = rng.choice(PREFIXES) if rng.random() < 0.45 else ""
        lines.append(prefix + rng.choice(FRAGMENTS))
    return "\n".join(lines) + "\n"


def docutils_pxml(text: str) -> str:
    settings = get_default_settings(Parser)
    settings.warning_stream = io.StringIO()
    settings.report_level = 5
    settings.halt_level = 5
    doc = new_document("test data", settings)
    Parser().parse(text, doc)
    return doc.pformat()


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("binary")
    ap.add_argument("work", type=Path)
    ap.add_argument("--count", type=int, default=1000)
    ap.add_argument("--seed", type=int, default=0)
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
        subprocess.run([args.binary, "--out", str(outputs)] + files[i:i + 40], check=True)

    skipped = mismatches = 0
    for name, text in texts.items():
        want = docutils_pxml(text)
        if any(tag in want for tag in INLINE):
            skipped += 1
            continue
        got = (outputs / f"{name}.pxml").read_bytes().decode()
        if want != got:
            base = args.work / f"mismatch-{mismatches}"
            base.with_suffix(".rst").write_bytes(text.encode())
            base.with_suffix(".want").write_bytes(want.encode())
            base.with_suffix(".got").write_bytes(got.encode())
            mismatches += 1
    compared = args.count - skipped
    print(f"seed {args.seed}: {compared - mismatches} of {compared} match docutils "
          f"({skipped} skipped for inline markup)")
    if mismatches:
        print(f"mismatches written to {args.work}/mismatch-*")
        raise SystemExit(1)


if __name__ == "__main__":
    main()
