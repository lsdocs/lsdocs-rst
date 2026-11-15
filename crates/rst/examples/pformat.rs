//! Prints the pseudo-XML of each reStructuredText file given, as docutils
//! would (`source="test data"`). With `--out DIR`, writes `DIR/<name>.pxml`
//! instead; used for differential testing against docutils.

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut out: Option<PathBuf> = None;
    let mut files = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "--out" {
            out = args.next().map(PathBuf::from);
        } else {
            files.push(PathBuf::from(arg));
        }
    }
    for file in files {
        let text = std::fs::read_to_string(&file).expect("readable UTF-8 file");
        let tree = lsdocs_rst::Parser::new().source("test data").parse(&text);
        match &out {
            Some(dir) => {
                let name = file.file_stem().unwrap().to_string_lossy().into_owned();
                std::fs::write(dir.join(format!("{name}.pxml")), tree.pformat()).unwrap();
            }
            None => print!("{}", tree.pformat()),
        }
    }
}
