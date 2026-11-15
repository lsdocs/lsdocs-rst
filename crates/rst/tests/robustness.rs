//! The parser never panics: random text made of characters that matter to
//! reStructuredText (markup, whitespace, wide and combining characters)
//! must parse to some tree.

/// A small deterministic generator (xorshift), so failures reproduce.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const PIECES: &[&str] = &[
    ".. note::",
    ".. image:: ",
    ".. figure::",
    ".. code::",
    ".. |s| replace::",
    ".. role:: ",
    ".. meta::",
    ".. contents::",
    ".. header::",
    ":class:",
    ":name:",
    ":width:",
    " ",
    " ",
    "  ",
    "    ",
    "\n",
    "\n",
    "\n\n",
    "\t",
    "a",
    "word",
    "Title",
    "*",
    "**",
    "`",
    "``",
    "_",
    "__",
    "|",
    ":",
    "::",
    "..",
    ".. ",
    "[",
    "]",
    "#",
    "-",
    "--",
    "=",
    "===",
    "+",
    "+--",
    "-+",
    "<",
    ">",
    "\\",
    "(",
    ")",
    "1.",
    "a)",
    "(i)",
    ">>>",
    "--opt",
    "-o",
    "http://x.example.com",
    "me@x.org",
    "\u{65e5}",
    "\u{301}",
    "\u{2014}",
    "\u{ab}",
    "\u{bb}",
    "\u{b}",
    "\u{c}",
    "\r",
    "\u{2028}",
    "\0",
];

#[test]
fn random_input_never_panics() {
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    for _ in 0..3000 {
        let mut text = String::new();
        for _ in 0..rng.below(120) {
            text.push_str(PIECES[rng.below(PIECES.len())]);
        }
        let result = std::panic::catch_unwind(|| lsdocs_rst::parse(&text).pformat());
        assert!(result.is_ok(), "panicked on input {text:?}");
    }
}
