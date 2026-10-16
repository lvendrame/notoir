//! Command line (docs/specs/38-command-line-interface.md, 08 §Open File):
//! `notoir [paths…] [--new-window] [--goto file:line[:col]] [--wait]`, plus
//! `--help`, `--version` and the `mcp-bridge` subcommand (spec 29).

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cli {
    pub version: bool,
    pub help: bool,
    pub new_window: bool,
    /// Keep the terminal process until this invocation's files are closed.
    pub wait: bool,
    /// Files/folders to open with an optional 1-based line/column.
    pub paths: Vec<(PathBuf, Option<(usize, usize)>)>,
    /// `--command <id> [--args <json>]`: run in the running instance when it
    /// was started with `NOTOIR_TEST_COMMANDS=1` (crash suite, automation).
    pub commands: Vec<(String, String)>,
}

/// A command line Notoir cannot run (printed with a usage hint, exit 2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CliError {
    UnknownOption(String),
    MissingValue(String),
}

impl CliError {
    /// The message in `lang`.
    pub fn message(&self, lang: crate::i18n::Language) -> String {
        use crate::i18n::tf_in;
        match self {
            CliError::UnknownOption(o) => tf_in(
                lang,
                "cli:unknownOption",
                &[("option", o as &dyn std::fmt::Display)],
            ),
            CliError::MissingValue(o) => tf_in(
                lang,
                "cli:missingValue",
                &[("option", o as &dyn std::fmt::Display)],
            ),
        }
    }
}

/// `notoir --help`.
pub fn usage(lang: crate::i18n::Language) -> String {
    use crate::i18n::t_in;
    let row = |flag: &str, key: &str| format!("  {flag:<28}{}\n", t_in(lang, key));
    let mut out = format!(
        "{}\n\n{}\n  notoir [{}…] [--new-window] [--goto {}] [--wait]\n  notoir mcp-bridge --token <token> [--port <port>]\n\n{}\n",
        t_in(lang, "cli:about"),
        t_in(lang, "cli:usage"),
        t_in(lang, "cli:paths"),
        t_in(lang, "cli:fileLineCol"),
        t_in(lang, "cli:options"),
    );
    out.push_str(&row("-n, --new-window", "cli:optNewWindow"));
    out.push_str(&row("-g, --goto <file:line[:col]>", "cli:optGoto"));
    out.push_str(&row("-w, --wait", "cli:optWait"));
    out.push_str(&row("-h, --help", "cli:optHelp"));
    out.push_str(&row("-V, --version", "cli:optVersion"));
    out.push('\n');
    out.push_str(&t_in(lang, "cli:pathsHelp"));
    out.push('\n');
    out
}

/// `notoir --version`.
pub fn version_line() -> String {
    format!("Notoir {}", env!("CARGO_PKG_VERSION"))
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
pub fn parse(args: &[String], cwd: &Path) -> Result<Cli, CliError> {
    let mut cli = Cli::default();
    let mut it = args.iter();
    let mut only_paths = false;
    while let Some(arg) = it.next() {
        if only_paths {
            cli.paths.push((cwd.join(arg), None));
            continue;
        }
        match arg.as_str() {
            "--version" | "-V" => cli.version = true,
            "--help" | "-h" => cli.help = true,
            "--new-window" | "-n" => cli.new_window = true,
            "--wait" | "-w" => cli.wait = true,
            "--goto" | "-g" => {
                let spec = it
                    .next()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| CliError::MissingValue(arg.clone()))?;
                let (file, at) = goto_target(spec);
                cli.paths.push((cwd.join(file), at));
            }
            "--command" => {
                let id = it
                    .next()
                    .ok_or_else(|| CliError::MissingValue(arg.clone()))?;
                cli.commands.push((id.clone(), "{}".into()));
            }
            "--args" => {
                let json = it
                    .next()
                    .ok_or_else(|| CliError::MissingValue(arg.clone()))?;
                match cli.commands.last_mut() {
                    Some(last) => last.1 = json.clone(),
                    None => return Err(CliError::UnknownOption(arg.clone())),
                }
            }
            "--" => only_paths = true,
            a if a.starts_with('-') && a.len() > 1 => {
                return Err(CliError::UnknownOption(a.to_owned()));
            }
            a => cli.paths.push((cwd.join(a), None)),
        }
    }
    Ok(cli)
}

/// Test-channel commands of a forwarded command line.
pub fn commands(args: &[String], cwd: &Path) -> Vec<(String, String)> {
    parse(args, cwd).map(|c| c.commands).unwrap_or_default()
}

pub fn paths(args: &[String], cwd: &Path) -> Vec<(PathBuf, Option<(usize, usize)>)> {
    parse(args, cwd).map(|c| c.paths).unwrap_or_default()
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
        )
        .unwrap();
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
            parse(&s(&["/abs/x.md"]), cwd).unwrap().paths[0].0,
            PathBuf::from("/abs/x.md")
        );
        assert!(parse(&s(&["-V"]), cwd).unwrap().version);
    }

    #[test]
    fn help_wait_and_errors() {
        let cwd = Path::new("/w");
        let cli = parse(&s(&["--help", "-w", "."]), cwd).unwrap();
        assert!(cli.help && cli.wait);
        assert_eq!(cli.paths, vec![(PathBuf::from("/w/."), None)]);
        assert_eq!(
            parse(&s(&["--frobnicate"]), cwd),
            Err(CliError::UnknownOption("--frobnicate".into()))
        );
        assert_eq!(
            parse(&s(&["--goto"]), cwd),
            Err(CliError::MissingValue("--goto".into()))
        );
        // A colon that is not a line suffix stays part of the path.
        assert_eq!(
            parse(&s(&["--goto", "notes:draft.md"]), cwd).unwrap().paths,
            vec![(PathBuf::from("/w/notes:draft.md"), None)]
        );
        // `--` ends options; `-` alone is a path.
        assert_eq!(
            parse(&s(&["--", "-odd.md", "-"]), cwd).unwrap().paths.len(),
            2
        );
        let en = crate::i18n::Language::En;
        let usage = usage(en);
        assert!(
            usage.contains("--new-window") && usage.contains("--wait"),
            "{usage}"
        );
        assert!(
            CliError::UnknownOption("-x".into())
                .message(en)
                .contains("-x")
        );
        assert!(version_line().starts_with("Notoir "));
    }
}
