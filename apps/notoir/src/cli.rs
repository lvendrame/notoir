//! Command line: `notoir [paths…] [--new-window] [--goto file:line[:col]]`
//! (docs/specs/08 §Open File).

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cli {
    pub version: bool,
    pub new_window: bool,
    /// Files/folders to open with an optional 1-based line/column.
    pub paths: Vec<(PathBuf, Option<(usize, usize)>)>,
}

/// Splits `file:line[:col]`; plain paths (even with ':' in them) stay whole.
fn goto_target(spec: &str) -> (String, Option<(usize, usize)>) {
    let parts: Vec<&str> = spec.rsplitn(3, ':').collect();
    match parts.as_slice() {
        [col, line, file] if line.parse::<usize>().is_ok() && col.parse::<usize>().is_ok() => (
            (*file).to_owned(),
            Some((line.parse().unwrap_or(1), col.parse().unwrap_or(1))),
        ),
        [line, file, ..] if line.parse::<usize>().is_ok() => {
            let file = spec.rsplit_once(':').map_or(*file, |(f, _)| f);
            (file.to_owned(), Some((line.parse().unwrap_or(1), 1)))
        }
        _ => (spec.to_owned(), None),
    }
}

/// Parses arguments (without the program name) relative to `cwd`.
pub fn parse(args: &[String], cwd: &Path) -> Cli {
    let mut cli = Cli::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--version" | "-V" => cli.version = true,
            "--new-window" | "-n" => cli.new_window = true,
            "--goto" | "-g" => {
                if let Some(spec) = it.next() {
                    let (file, at) = goto_target(spec);
                    cli.paths.push((cwd.join(file), at));
                }
            }
            a if a.starts_with('-') => tracing::warn!(arg = a, "unknown argument"),
            a => cli.paths.push((cwd.join(a), None)),
        }
    }
    cli
}

pub fn paths(args: &[String], cwd: &Path) -> Vec<(PathBuf, Option<(usize, usize)>)> {
    parse(args, cwd).paths
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn parses_paths_flags_and_goto() {
        let cwd = Path::new("/home/me");
        let cli = parse(
            &s(&[
                "notes.md",
                "--new-window",
                "--goto",
                "src/a.rs:12:4",
                "-g",
                "b.txt:3",
            ]),
            cwd,
        );
        assert!(cli.new_window);
        assert_eq!(
            cli.paths,
            vec![
                (PathBuf::from("/home/me/notes.md"), None),
                (PathBuf::from("/home/me/src/a.rs"), Some((12, 4))),
                (PathBuf::from("/home/me/b.txt"), Some((3, 1))),
            ]
        );
        assert_eq!(
            parse(&s(&["/abs/x.md"]), cwd).paths[0].0,
            PathBuf::from("/abs/x.md")
        );
        assert!(parse(&s(&["-V"]), cwd).version);
    }
}
