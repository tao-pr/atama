//! Built-in tools for the agent loop. They run the same actors as the
//! actions of a job file.

use serde_json::{Value, json};

use super::Tool;
use crate::actor::{Actor, AgentType};

/// Most characters of a tool result that the model gets. A longer result is
/// cut, so that one command cannot fill the model's context.
pub const MAX_RESULT_CHARS: usize = 20_000;

/// The tools that each agent type gets:
///
/// - `coder` gets [`Bash`].
/// - `summarizer` gets no tools, because it only rewrites its input.
pub fn default_tools(agent: AgentType) -> Vec<Box<dyn Tool>> {
    match agent {
        AgentType::Coder => vec![Box::new(Bash)],
        AgentType::Summarizer => Vec::new(),
    }
}

/// `bash`: runs a command in bash in the current directory and returns its
/// stdout.
pub struct Bash;

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
        let output = Actor::BashScript {
            cmd: string_argument(arguments, "command")?,
            params: Vec::new(),
        }
        .act("")?;
        Ok(truncate(output))
    }
}

fn string_argument(arguments: &Value, name: &str) -> Result<String, String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("missing string argument `{name}`"))
}

/// Cuts `text` to [`MAX_RESULT_CHARS`] characters and says so at the end.
pub fn truncate(text: String) -> String {
    match text.char_indices().nth(MAX_RESULT_CHARS) {
        None => text,
        Some((end, _)) => format!(
            "{}\n[cut: the result has more than {MAX_RESULT_CHARS} characters]",
            &text[..end]
        ),
    }
}
