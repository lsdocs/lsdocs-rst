//! Access to files for the directives that read them: `include`, `raw`
//! and `csv-table` with a `:file:` (or `:url:`) option.
//!
//! Parsing reads nothing unless the caller gives the [`Parser`] a
//! [`FileLoader`]; without one, these directives behave as docutils' do
//! with `file_insertion_enabled` off.
//!
//! [`Parser`]: crate::Parser

use std::io;
use std::path::PathBuf;

/// Reads the files (and URLs) directives ask for.
///
/// Paths are as docutils computes them: relative to the directory of the
/// including document (or of the file being included from), with `/`
/// separators and `.` and `..` resolved, or absolute as written.
pub trait FileLoader: Send + Sync {
    /// The contents of the file at `path`.
    fn read_file(&self, path: &str) -> io::Result<Vec<u8>>;

    /// The contents at `url` (a `:url:` option). The default refuses.
    fn read_url(&self, url: &str) -> io::Result<Vec<u8>> {
        let _ = url;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "URL access is not supported",
        ))
    }
}

/// A function reading files is a [`FileLoader`] (without URL access).
impl<F> FileLoader for F
where
    F: Fn(&str) -> io::Result<Vec<u8>> + Send + Sync,
{
    fn read_file(&self, path: &str) -> io::Result<Vec<u8>> {
        self(path)
    }
}

/// A [`FileLoader`] reading files from a directory: relative paths are
/// taken from it, absolute paths as they are. It does not keep paths
/// inside the directory.
#[derive(Debug, Clone)]
pub struct DirectoryLoader {
    root: PathBuf,
}

impl DirectoryLoader {
    /// A loader reading files relative to `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl FileLoader for DirectoryLoader {
    fn read_file(&self, path: &str) -> io::Result<Vec<u8>> {
        std::fs::read(self.root.join(path))
    }
}

/// `path` relative to the directory of `source` (docutils' `adapt_path`,
/// with paths kept relative to the document's directory rather than the
/// working directory).
pub(crate) fn adapt_path(path: &str, source: Option<&str>) -> String {
    let path = path.replace('\\', "/");
    if path.starts_with('/') {
        return normalize(&path);
    }
    let source = source.unwrap_or("").replace('\\', "/");
    let dir = match source.rfind('/') {
        Some(i) => &source[..i],
        None => "",
    };
    if dir.is_empty() {
        normalize(&path)
    } else {
        normalize(&format!("{dir}/{path}"))
    }
}

/// `path` with `.` and inner `..` components resolved.
fn normalize(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|p| *p != "..") => {
                parts.pop();
            }
            ".." if absolute => {}
            _ => parts.push(part),
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{joined}")
    } else if joined.is_empty() {
        ".".to_owned()
    } else {
        joined
    }
}

/// An error opening a file as Python prints it (`[Errno 2] No such file
/// or directory: 'x'`).
pub(crate) fn os_error(error: &io::Error, path: &str) -> String {
    let path = crate::util::py_repr(path);
    let (errno, text) = match error.kind() {
        io::ErrorKind::NotFound => (Some(2), "No such file or directory".to_owned()),
        io::ErrorKind::PermissionDenied => (Some(13), "Permission denied".to_owned()),
        io::ErrorKind::IsADirectory => (Some(21), "Is a directory".to_owned()),
        _ => (error.raw_os_error(), error.to_string()),
    };
    match errno {
        Some(n) => format!("[Errno {n}] {text}: {path}"),
        None => format!("{text}: {path}"),
    }
}

/// Decodes file contents as Python's text mode does: in `encoding`
/// (UTF-8 if `None`), with universal newlines. The error is Python's.
pub(crate) fn decode(bytes: &[u8], encoding: Option<&str>) -> Result<String, String> {
    let encoding = encoding.unwrap_or("utf-8").to_lowercase().replace('_', "-");
    let text = match encoding.as_str() {
        "utf-8" | "utf8" | "u8" => utf8(bytes, "utf-8")?,
        "utf-8-sig" => {
            let text = utf8(bytes, "utf-8")?;
            text.strip_prefix('\u{feff}')
                .map(str::to_owned)
                .unwrap_or(text)
        }
        "latin-1" | "latin1" | "iso-8859-1" | "iso8859-1" => {
            bytes.iter().map(|&b| char::from(b)).collect()
        }
        "ascii" | "us-ascii" => match bytes.iter().position(|b| !b.is_ascii()) {
            None => bytes.iter().map(|&b| char::from(b)).collect(),
            Some(i) => {
                return Err(format!(
                    "UnicodeDecodeError: 'ascii' codec can't decode byte {:#04x} in position {i}: ordinal not in range(128)",
                    bytes[i]
                ));
            }
        },
        other => return Err(format!("LookupError: unsupported encoding: {other}")),
    };
    Ok(text.replace("\r\n", "\n").replace('\r', "\n"))
}

fn utf8(bytes: &[u8], codec: &str) -> Result<String, String> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.to_owned()),
        Err(e) => {
            let at = e.valid_up_to();
            let reason = match e.error_len() {
                None if bytes.len() - at > 1 => {
                    return Err(format!(
                        "UnicodeDecodeError: '{codec}' codec can't decode bytes in position {at}-{}: unexpected end of data",
                        bytes.len() - 1
                    ));
                }
                None => "unexpected end of data",
                Some(_) if matches!(bytes[at], 0xc2..=0xf4) => "invalid continuation byte",
                Some(_) => "invalid start byte",
            };
            Err(format!(
                "UnicodeDecodeError: '{codec}' codec can't decode byte {:#04x} in position {at}: {reason}",
                bytes[at]
            ))
        }
    }
}

/// The width in pixels of a PNG, GIF, JPEG, BMP or WebP image, from its
/// header (what `figure :figwidth: image` needs; docutils asks PIL).
pub(crate) fn image_width(bytes: &[u8]) -> Option<u32> {
    let u16be = |i: usize| Some(u16::from_be_bytes(bytes.get(i..i + 2)?.try_into().ok()?));
    let u16le = |i: usize| Some(u16::from_le_bytes(bytes.get(i..i + 2)?.try_into().ok()?));
    let u32be = |i: usize| Some(u32::from_be_bytes(bytes.get(i..i + 4)?.try_into().ok()?));
    let u32le = |i: usize| Some(u32::from_le_bytes(bytes.get(i..i + 4)?.try_into().ok()?));
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.get(12..16) == Some(b"IHDR") {
        return u32be(16);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return u16le(6).map(u32::from);
    }
    if bytes.starts_with(b"BM") {
        return match u32le(14)? {
            12 => u16le(18).map(u32::from),
            _ => u32le(18).map(|w| (w as i32).unsigned_abs()),
        };
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return match bytes.get(12..16)? {
            b"VP8 " => u16le(26).map(|w| u32::from(w & 0x3fff)),
            b"VP8L" => {
                let (a, b) = (*bytes.get(21)?, *bytes.get(22)?);
                Some(1 + (u32::from(a) | (u32::from(b & 0x3f) << 8)))
            }
            b"VP8X" => {
                let w = bytes.get(24..27)?;
                Some(1 + (u32::from(w[0]) | u32::from(w[1]) << 8 | u32::from(w[2]) << 16))
            }
            _ => None,
        };
    }
    if bytes.starts_with(b"\xff\xd8") {
        // Segments up to a start-of-frame marker.
        let mut i = 2;
        while i + 4 <= bytes.len() {
            if bytes[i] != 0xff {
                return None;
            }
            let marker = bytes[i + 1];
            if marker == 0xff {
                i += 1;
                continue;
            }
            if matches!(marker, 0xd0..=0xd9 | 0x01) {
                i += 2;
                continue;
            }
            let len = usize::from(u16be(i + 2)?);
            if matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
                return u16be(i + 7).map(u32::from);
            }
            i += 2 + len;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn adapts_paths() {
        assert_eq!(adapt_path("a.rst", Some("test data")), "a.rst");
        assert_eq!(adapt_path("a.rst", Some("doc/index.rst")), "doc/a.rst");
        assert_eq!(adapt_path("../a.rst", Some("doc/index.rst")), "a.rst");
        assert_eq!(adapt_path("../a.rst", None), "../a.rst");
        assert_eq!(adapt_path("./x/./y.rst", Some("d\\i.rst")), "d/x/y.rst");
        assert_eq!(adapt_path("/abs/a.rst", Some("doc/i.rst")), "/abs/a.rst");
    }

    #[test]
    fn decodes_like_python() {
        assert_eq!(decode(b"a\r\nb\rc", None).unwrap(), "a\nb\nc");
        assert_eq!(decode(b"\xef\xbb\xbfx", Some("utf-8-sig")).unwrap(), "x");
        assert_eq!(decode(b"\xe9", Some("latin-1")).unwrap(), "\u{e9}");
        assert_eq!(
            decode(b"a\xff", None).unwrap_err(),
            "UnicodeDecodeError: 'utf-8' codec can't decode byte 0xff in position 1: invalid start byte"
        );
    }
}
