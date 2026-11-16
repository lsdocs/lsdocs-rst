//! Prints the pseudo-XML of each reStructuredText file given, as docutils
//! would (`source="test data"`). With `--out DIR`, writes `DIR/<name>.pxml`
//! instead; with `--transforms`, the transformed tree; with `--files DIR`,
//! directives may read files from `DIR`. Used for differential testing
//! against docutils.

use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut out: Option<PathBuf> = None;
    let mut transforms = false;
    let mut files: Option<PathBuf> = None;
    let mut inputs = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "--transforms" {
            transforms = true;
        } else if arg == "--files" {
            files = args.next().map(PathBuf::from);
        } else if arg == "--out" {
            out = args.next().map(PathBuf::from);
        } else {
            inputs.push(PathBuf::from(arg));
        }
    }
    let mut parser = lsdocs_rst::Parser::new()
        .source("test data")
        .transforms(transforms);
    if let Some(dir) = files {
        parser = parser.file_loader(lsdocs_rst::DirectoryLoader::new(dir));
    }
    for file in inputs {
        let text = std::fs::read_to_string(&file)?;
        let tree = parser.parse(&text);
        match &out {
            Some(dir) => {
                let name = file.file_stem().unwrap_or_default().to_string_lossy();
                std::fs::write(dir.join(format!("{name}.pxml")), tree.pformat())?;
            }
            None => print!("{}", tree.pformat()),
        }
    }
    Ok(())
}
