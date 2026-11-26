# lsdocs-parser-rst

The lsdocs plugin parsing reStructuredText, on `lsdocs-rst` (a port of
docutils' parser and transforms). What it does and its settings are in
the lsdocs documentation (https://lsdocs.github.io, "parser-rst").

## Settings

It reads docutils' own configuration file,
`.config/lsdocs/<name>/docutils.conf` (`<name>` being its name in
`lsdocs.toml`): the settings `lsdocs-rst` implements from the
`[general]`, `[parsers]` and `[restructuredtext parser]` sections, and
`file_insertion_enabled`. Other sections are skipped; docutils settings
it does not implement are reported once, as a warning.

Task lists (`- [ ] item`, `- [x] item`, `- [X] item`; also in enumerated
lists) are on: lsdocs-rst's `task_lists` setting, which docutils does not
have and which is off in `lsdocs-rst` itself. Turn them off with

```ini
[restructuredtext parser]
task_lists: no
```

## Known differences from docutils

Keep this list current. Reading `docutils.conf` (`src/conf.rs`):

- Section names are matched case-insensitively; docutils' are
  case-sensitive.
- A continued value (indented lines after a setting) is joined with
  spaces, and blank lines inside it are dropped; docutils keeps the
  line breaks.
- When sections set the same setting, the later one in the file wins;
  docutils ranks them `[general]` < `[parsers]` <
  `[restructuredtext parser]` whatever their order.
- An empty boolean value reads as false; docutils rejects it.
- `task_lists` is lsdocs-rst's own setting, not docutils'.
- `doctitle_xform` is always off (lsdocs takes the title itself), so a
  `doctitle_xform` setting is overridden, without a warning.
- Warnings about settings it does not implement are logged at the first
  parse of a session, so a build served entirely from the cache does not
  show them.
