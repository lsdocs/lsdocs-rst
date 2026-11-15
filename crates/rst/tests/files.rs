//! File access through a caller's `FileLoader` (`include`, `raw` and
//! `csv-table` files). Conformance with docutils is checked by the
//! `file_*` cases in `tests/conformance.rs`; these test the API.

use std::collections::HashMap;
use std::io;

use lsdocs_rst::{FileLoader, Kind, Parser, Span};

/// Files in memory.
fn files(entries: &[(&str, &str)]) -> impl Fn(&str) -> io::Result<Vec<u8>> + Send + Sync + use<> {
    let map: HashMap<String, Vec<u8>> = entries
        .iter()
        .map(|(k, v)| (k.to_string(), v.as_bytes().to_vec()))
        .collect();
    move |path: &str| {
        map.get(path)
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
}

#[test]
fn without_a_loader_files_are_disabled() {
    let tree = Parser::new().parse(".. include:: x.rst\n");
    assert!(tree.pformat().contains("\"include\" directive disabled."));
}

#[test]
fn includes_resolve_against_the_including_file() {
    let loader = files(&[
        ("docs/part.rst", "Part.\n\n.. include:: sub/inner.rst\n"),
        ("docs/sub/inner.rst", "Inner *text*.\n"),
    ]);
    let tree = Parser::new()
        .source("docs/index.rst")
        .file_loader(loader)
        .parse("Main.\n\n.. include:: part.rst\n");
    let out = tree.pformat();
    assert!(out.contains("Part."), "{out}");
    assert!(out.contains("Inner "), "{out}");
    assert!(!out.contains("system_message"), "{out}");
}

#[test]
fn included_nodes_have_empty_spans_at_the_directive() {
    let text = "Main.\n\n.. include:: part.rst\n";
    let tree = Parser::new()
        .file_loader(files(&[("part.rst", "Included paragraph.\n")]))
        .parse(text);
    let paragraphs: Vec<_> = tree
        .elements()
        .filter(|e| e.kind == Kind::Paragraph)
        .collect();
    assert_eq!(paragraphs.len(), 2);
    let at = text.find(".. include").unwrap();
    assert_eq!(paragraphs[1].span, Some(Span::new(at, at)));
    assert_eq!(paragraphs[0].span, Some(Span::new(0, 5)));
}

#[test]
fn messages_name_the_included_file() {
    let tree = Parser::new()
        .source("index.rst")
        .file_loader(files(&[("part.rst", "One.\n\nTwo `bad.\n")]))
        .parse(".. include:: part.rst\n");
    let out = tree.pformat();
    assert!(out.contains("line=\"3\" source=\"part.rst\""), "{out}");
}

#[test]
fn urls_go_to_the_loader() {
    struct Web;
    impl FileLoader for Web {
        fn read_file(&self, _: &str) -> io::Result<Vec<u8>> {
            Err(io::ErrorKind::NotFound.into())
        }
        fn read_url(&self, url: &str) -> io::Result<Vec<u8>> {
            Ok(format!("<p>{url}</p>").into_bytes())
        }
    }
    let tree = Parser::new()
        .file_loader(Web)
        .parse(".. raw:: html\n   :url: https://example.org/a\n");
    let out = tree.pformat();
    assert!(out.contains("source=\"https://example.org/a\""), "{out}");
    assert!(out.contains("<p>https://example.org/a</p>"), "{out}");
}

#[test]
fn unsupported_urls_are_errors() {
    let tree = Parser::new()
        .file_loader(files(&[]))
        .parse(".. raw:: html\n   :url: https://example.org/a\n");
    let out = tree.pformat();
    assert!(
        out.contains("Problems with \"raw\" directive URL \"https://example.org/a\""),
        "{out}"
    );
}

#[test]
fn circular_inclusion_is_reported() {
    let tree = Parser::new()
        .file_loader(files(&[
            ("a.rst", ".. include:: b.rst\n"),
            ("b.rst", ".. include:: a.rst\n"),
        ]))
        .parse(".. include:: a.rst\n");
    let out = tree.pformat();
    assert!(
        out.contains("circular inclusion in \"include\" directive"),
        "{out}"
    );
}
