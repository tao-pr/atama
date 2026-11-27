//! The `bash` tool.

use std::sync::Arc;
use std::time::Instant;

use serde_json::{Value, json};

use super::{Tool, string_argument, truncate};
use crate::actor::{run_bash, run_bash_in_sandbox};
use crate::protect::Protect;

/// `bash`: runs a command in bash in the current directory and returns its
/// stdout. With a deadline, it stops the command and its child processes at
/// the deadline (see [`Tool::set_deadline_time`]). With protected paths, it
/// runs the command inside `sandbox-exec`, which denies the writes to them
/// (see [`Tool::set_protect`]).
#[derive(Default)]
pub struct Bash {
    deadline_time: Option<Instant>,
    protect: Option<Arc<Protect>>,
}

impl Tool for Bash {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Runs a command in bash in the current directory and returns its stdout. \
         A command that exits with a non-zero status returns an error with its stderr."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"command": {"type": "string", "description": "The command to run."}},
            "required": ["command"],
        })
    }

    fn call(&mut self, arguments: &Value) -> Result<String, String> {
        let command = string_argument(arguments, "command")?;
        let output = match &self.protect {
            None => run_bash(&command, &[], "", self.deadline_time)?,
            Some(protect) => {
                run_bash_in_sandbox(&command, &protect.sandbox_profile(), self.deadline_time)?
            }
        };
        Ok(truncate(output))
    }

    fn set_deadline_time(&mut self, deadline_time: Option<Instant>) {
        self.deadline_time = deadline_time;
    }

    fn set_protect(&mut self, protect: Option<Arc<Protect>>) {
        self.protect = protect;
    }
}
