# AGENTS.md

Guidance for AI agents (and humans) working in lsdocs-rst, one repository of
the lsdocs organization (https://github.com/lsdocs). It was split from the
lsdocs monorepo at commit b95f771; its history before that is the
monorepo's, filtered to these files. README.md says what it is and lists
its known limitations and TODO items.

## Working across repositories

Other lsdocs repositories are git dependencies
(`{ git = "https://github.com/lsdocs/<repo>", branch = "main" }`);
`Cargo.lock` pins their revisions (`nur bump [crate]` moves them).

- `nur link ../<repo>` builds against a local checkout (a `[patch]` in the
  untracked `.cargo/config.toml`); `nur unlink` undoes it.
- With `LSDOCS_ORG_DIR` set (e.g. `C:/dev`), Cargo fetches the
  organization's repositories from the local clones there (their
  committed `main`) instead of GitHub.
- `nur check-dupes` fails if a crate of the organization is built twice;
  `nur check-git-deps` fails on a dependency that is not a path inside
  this repository or a git dependency on the repository holding it
  (`scripts/org.nuon`), and on a committed `[patch]` table. `nur msrv`
  checks each crate on its own `rust-version`; `nur deny` runs
  cargo-deny (both are CI jobs of their own).

## Dev tasks

Dev tasks are nushell scripts run with `nur` (`nurfile`); `nur --list`
shows them. `nur ci` runs what CI runs: run it before committing. Tools:
`mise install` (see `.config/mise.toml`); Rust from rustup.

## Conventions

- Commits: Conventional Commits, `type(scope): summary`.
- Line endings: LF everywhere; scripts that write files write LF.
- Comments and docs: ASCII only.
- MSRV is per crate (`rust-version`); the workspace's is the base.
- Dependencies are declared in `[workspace.dependencies]` and used with
  `workspace = true`.
- Lints: the workspace lints apply to every crate; `missing_docs` is on;
  `unsafe_code` is denied.
- No panics outside tests: clippy denies `unwrap_used`, `expect_used` and
  `panic` in library, binary and example code. Tests opt in per module
  (`#![allow(clippy::unwrap_used)]`).
- Licensing: `Apache-2.0 OR MIT`.
- Publishing: merging release-plz's release PR publishes the bumped
  crates to crates.io (`.github/workflows/release.yml`) (not those
  with `publish = false`); a `lsdocs-rst-python/v*` tag publishes the
  Python package to PyPI, a `lsdocs-rst-node/v*` tag the Node.js packages
  to npm, and a `lsdocs-rst-lua/v*` tag attaches the Lua modules and the
  rockspec to its GitHub release (`tag.yml`).
- Record what is left open in README.md's TODO section, and what the code
  does not do (or does differently) under Known limitations.

## This repository

| Path | What |
|---|---|
| `crates/rst` | `lsdocs-rst`: standalone reStructuredText parser (docutils port). Depends on nothing of lsdocs; keep it that way. |
| `crates/parser-rst` | `lsdocs-parser-rst`: the lsdocs plugin on it. |
| `bindings/python`, `bindings/node`, `bindings/lua` | `lsdocs-rst` for Python, Node.js and Lua; their tests run the conformance cases. |
| `scripts/rst-*.py` | The docutils bridges behind `nur gen rst` and `nur fuzz rst`. |

Tasks: `nur test python`, `nur test node`, `nur gen node`; `nur gen rst`
regenerates the expected trees of the conformance cases and
`crates/rst/src/tables.rs` with docutils (via uv; versions `DOCUTILS` and
`PILLOW` in the nurfile); `nur fuzz rst [--count N] [--seed N]
[--transforms]` compares with docutils on random documents (mismatches go
to `target/rst-fuzz`).

### Generated files

Never hand-edit these; regenerate and review the diff.

- `crates/rst/tests/cases/*.pxml`, `*.tpxml` and `*.xml`: `nur gen rst`
  after adding or changing a `.rst` case (the parser's tree, the tree after
  docutils' transforms, and that tree through docutils' XML writer). A
  case's `<name>.settings.json` (hand-written) holds docutils settings for
  it. Cases named `file_*` may read files (from `cases/files/`; included
  files there end in `.txt`). Cases that cannot match yet go in
  `cases/PENDING` or `cases/PENDING-TRANSFORMS`.
- `crates/rst/src/tables.rs`: also `nur gen rst`.
- `bindings/node/native.js`, `native.d.ts`: `nur gen node`.

### Bindings

They carry trees as the `serde` form of `lsdocs-rst` and keep the
parser's behavior in Rust; a Python directive runs on a scoped worker
thread that asks the parsing thread for parses (see
`bindings/python/src/lib.rs`); in Node.js the parser runs on a worker
thread that asks the JS thread to run JS extensions (see
`bindings/node/src/lib.rs`). A uv workspace is rooted at
`pyproject.toml`.
