//! Passer: The implementation of the data capture or data passer

use std::fs;
use std::process::Command;

/// Where a value comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Passer {
    /// Contents of the text file at this path.
    FromFile(String),
    /// Data written directly in the job file.
    Data(String),
    /// Value of the environment variable with this name.
    FromEnvVar(String),
    /// Standard output of this command, run with `sh -c`.
    FromStdoutOfCommand(String),
}

impl Passer {
    /// Reads the value from its source.
    ///
    /// Trailing newlines are removed from file contents and command output,
    /// as shell `$(...)` does, so a token stored in a file can be used as is.
    /// Fails if the file cannot be read, the variable is not set, or the
    /// command cannot start or exits with a non-zero status.
    pub fn get(&self) -> Result<String, String> {
        match self {
            Passer::Data(data) => Ok(data.clone()),
            Passer::FromFile(path) => fs::read_to_string(path)
                .map(|contents| trim_trailing_newlines(&contents))
                .map_err(|err| format!("cannot read file `{path}`: {err}")),
            Passer::FromEnvVar(name) => std::env::var(name)
                .map_err(|err| format!("cannot read environment variable `{name}`: {err}")),
            Passer::FromStdoutOfCommand(command) => {
                let output = Command::new("sh")
                    .arg("-c")
                    .arg(command)
                    .output()
                    .map_err(|err| format!("failed to run command `{command}`: {err}"))?;
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(format!(
                        "command `{command}` failed with {}: {}",
                        output.status,
                        stderr.trim()
                    ));
                }
                String::from_utf8(output.stdout)
                    .map(|stdout| trim_trailing_newlines(&stdout))
                    .map_err(|_| format!("command `{command}` printed output that is not UTF-8"))
            }
        }
    }
}

fn trim_trailing_newlines(text: &str) -> String {
    text.trim_end_matches(['\n', '\r']).to_string()
}
