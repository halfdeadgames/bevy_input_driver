//! Loading a JSON Lines command script named at launch.

use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};

use bevy::{log::info, prelude::*};

use crate::{
    commands::{CommandParseError, DriverCommand, parse_command_text},
    plugin::DriverSettings,
    real_input::RealInputBlocking,
    status::DriverStatus,
    steps::DriverQueue,
};

const SCRIPT_ARGUMENT: &str = "--input-script";

/// Why a launch script could not be used.
#[derive(Debug)]
pub enum ScriptFileError {
    /// `--input-script` was the last argument, with no path after it.
    MissingPathArgument,
    /// The file could not be read.
    Read {
        /// Script path.
        path: PathBuf,
        /// IO failure.
        source: io::Error,
    },
    /// A line is not a valid command.
    Line {
        /// Script path.
        path: PathBuf,
        /// One-based line number.
        line: usize,
        /// Parse failure.
        source: CommandParseError,
    },
}

impl fmt::Display for ScriptFileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPathArgument => {
                write!(formatter, "{SCRIPT_ARGUMENT} needs a file path after it")
            }
            Self::Read { path, source } => {
                write!(
                    formatter,
                    "script {}: cannot read file: {source}",
                    path.display()
                )
            }
            Self::Line { path, line, source } => {
                write!(formatter, "script {} line {line}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for ScriptFileError {}

/// Finds the script path in process arguments, as `--input-script <path>` or
/// `--input-script=<path>`. The first argument is the program name and is ignored by callers.
///
/// # Errors
///
/// Returns [`ScriptFileError::MissingPathArgument`] when the flag has no value.
pub fn script_path_from_arguments(
    arguments: impl IntoIterator<Item = String>,
) -> Result<Option<PathBuf>, ScriptFileError> {
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        if argument == SCRIPT_ARGUMENT {
            return arguments
                .next()
                .map(|path| Some(PathBuf::from(path)))
                .ok_or(ScriptFileError::MissingPathArgument);
        }
        if let Some(path) = argument.strip_prefix("--input-script=") {
            return Ok(Some(PathBuf::from(path)));
        }
    }
    Ok(None)
}

/// Parses script text: blank lines and lines starting with `//` or `#` are skipped, every other
/// line is one command. Parses everything before returning anything.
///
/// # Errors
///
/// Returns [`ScriptFileError::Line`] naming the first bad line.
pub fn parse_script_text(path: &Path, text: &str) -> Result<Vec<DriverCommand>, ScriptFileError> {
    let mut commands = Vec::new();
    for (index, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        let command = parse_command_text(line).map_err(|source| ScriptFileError::Line {
            path: path.to_path_buf(),
            line: index + 1,
            source,
        })?;
        commands.push(command);
    }
    Ok(commands)
}

fn load_script_commands(path: &Path) -> Result<Vec<DriverCommand>, ScriptFileError> {
    let text = fs::read_to_string(path).map_err(|source| ScriptFileError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    parse_script_text(path, &text)
}

/// Startup system: loads the launch script, if one is named, and queues its commands.
///
/// The `--input-script` process argument overrides the path set on the plugin. On any error
/// nothing is queued and the error is recorded in [`DriverStatus::errors`]. A loaded script also
/// starts blocking real input.
pub fn load_script_file(
    settings: Res<DriverSettings>,
    mut queue: ResMut<DriverQueue>,
    mut status: ResMut<DriverStatus>,
    mut blocking: ResMut<RealInputBlocking>,
) {
    let from_arguments = match script_path_from_arguments(std::env::args().skip(1)) {
        Ok(path) => path,
        Err(problem) => {
            status.record_error(problem.to_string());
            return;
        }
    };
    let Some(path) = from_arguments.or_else(|| settings.script_path.clone()) else {
        return;
    };
    match load_script_commands(&path) {
        Ok(commands) => {
            queue.enqueue_commands(&commands);
            blocking.block_for_first_command();
            status.script_loaded = Some(path.display().to_string());
        }
        Err(problem) => status.record_error(problem.to_string()),
    }
}

/// Startup system: logs the listening address and whether a script was loaded.
pub fn log_driver_startup(settings: Res<DriverSettings>, status: Res<DriverStatus>) {
    let listening = if settings.enable_remote {
        format!("listening on 127.0.0.1:{}", settings.port)
    } else {
        "remote control disabled".to_owned()
    };
    let script = status.script_loaded.as_deref().map_or_else(
        || "no script loaded".to_owned(),
        |path| format!("script loaded: {path}"),
    );
    info!("input driver {listening}, {script}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn skips_comments_and_blank_lines() {
        let text = "// header\n\n# note\n{\"cmd\":\"quit\"}\n   \n{\"cmd\":\"release_all\"}\n";
        let commands = parse_script_text(Path::new("t.jsonl"), text).expect("script should parse");
        assert_eq!(
            commands,
            vec![DriverCommand::Quit {}, DriverCommand::ReleaseAll {}]
        );
    }

    #[test]
    fn errors_name_file_and_line() {
        let text = "// c\n{\"cmd\":\"quit\"}\n{\"cmd\":\"bogus\"}\n";
        let message = parse_script_text(Path::new("t.jsonl"), text)
            .expect_err("bad line should fail")
            .to_string();
        assert!(message.contains("t.jsonl"), "{message}");
        assert!(message.contains("line 3"), "{message}");
        assert!(message.contains("bogus"), "{message}");
    }

    #[test]
    fn finds_script_argument_in_both_forms() {
        assert_eq!(
            script_path_from_arguments(arguments(&["--input-script", "a.jsonl"]))
                .expect("flag with value"),
            Some(PathBuf::from("a.jsonl"))
        );
        assert_eq!(
            script_path_from_arguments(arguments(&["--input-script=b.jsonl"]))
                .expect("flag with equals"),
            Some(PathBuf::from("b.jsonl"))
        );
        assert_eq!(
            script_path_from_arguments(arguments(&["--other"])).expect("no flag"),
            None
        );
        assert!(script_path_from_arguments(arguments(&["--input-script"])).is_err());
    }
}
