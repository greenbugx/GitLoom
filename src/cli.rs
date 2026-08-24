//! Command line parsing.
//!
//! Kept deliberately small and dependency-free: GitLoom takes one optional
//! path and two informational flags, so hand-matching them is less machinery
//! than a parser crate would be. Parsing happens *before* the terminal enters
//! raw mode, so `--help` prints normally instead of being swallowed by the
//! alternate screen or treated as a repository named `--help`.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cli {
    /// Open the TUI, on the given path or the current directory.
    Run(Option<PathBuf>),
    Help,
    Version,
}

pub const HELP: &str = "\
gitloom - a keyboard-first terminal Git history explorer

USAGE:
    gitloom [OPTIONS] [PATH]

ARGS:
    <PATH>    Repository to open. Defaults to the current directory; any
              directory inside a working tree will do.

OPTIONS:
    -h, --help       Print this help and exit
    -V, --version    Print the version and exit

Press ? inside gitloom for the full keymap.";

pub fn version() -> String {
    format!("gitloom {}", env!("CARGO_PKG_VERSION"))
}

/// Parse an argument list including the program name at index 0, the shape
/// `std::env::args()` produces.
///
/// Unknown flags are an error rather than being taken as a path: a mistyped
/// `--verison` should say so, not try to open a repository by that name. A
/// lone `-` stays a path so it isn't mistaken for a flag.
pub fn parse<I>(args: I) -> Result<Cli, String>
where
    I: IntoIterator<Item = String>,
{
    let mut path: Option<PathBuf> = None;

    for arg in args.into_iter().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Cli::Help),
            "-V" | "--version" => return Ok(Cli::Version),
            other if other.starts_with('-') && other != "-" => {
                return Err(format!("unknown option `{other}`"));
            }
            other => {
                if path.is_some() {
                    return Err(format!("unexpected second path `{other}`"));
                }
                path = Some(PathBuf::from(other));
            }
        }
    }

    Ok(Cli::Run(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Cli, String> {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn no_arguments_opens_the_current_directory() {
        assert_eq!(parse_args(&["gitloom"]), Ok(Cli::Run(None)));
    }

    #[test]
    fn a_positional_path_is_the_repository_to_open() {
        assert_eq!(
            parse_args(&["gitloom", "/src/project"]),
            Ok(Cli::Run(Some(PathBuf::from("/src/project"))))
        );
    }

    #[test]
    fn help_and_version_flags_are_recognised_in_both_forms() {
        assert_eq!(parse_args(&["gitloom", "--help"]), Ok(Cli::Help));
        assert_eq!(parse_args(&["gitloom", "-h"]), Ok(Cli::Help));
        assert_eq!(parse_args(&["gitloom", "--version"]), Ok(Cli::Version));
        assert_eq!(parse_args(&["gitloom", "-V"]), Ok(Cli::Version));
    }

    /// The bug this parser exists to fix: `--help` used to be taken as a path
    /// and handed to `Repository::discover`.
    #[test]
    fn a_flag_is_never_treated_as_a_path() {
        assert!(!matches!(
            parse_args(&["gitloom", "--help"]),
            Ok(Cli::Run(_))
        ));
        assert!(parse_args(&["gitloom", "--nope"]).is_err());
    }

    #[test]
    fn a_flag_wins_over_a_path_given_alongside_it() {
        assert_eq!(parse_args(&["gitloom", "/src", "--help"]), Ok(Cli::Help));
    }

    #[test]
    fn two_paths_are_rejected_rather_than_silently_ignored() {
        assert!(parse_args(&["gitloom", "/one", "/two"]).is_err());
    }

    #[test]
    fn a_bare_dash_is_a_path_not_a_flag() {
        assert_eq!(
            parse_args(&["gitloom", "-"]),
            Ok(Cli::Run(Some(PathBuf::from("-"))))
        );
    }
}
