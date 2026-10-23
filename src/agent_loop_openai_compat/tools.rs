//! Built-in tools for the agent loop. They run the same actors as the
//! actions of a job file.

use serde_json::{Value, json};

use super::Tool;
use crate::actor::{Actor, AgentType, ApiVerb};

/// Most characters of a tool result that the model gets. A longer result is
/// cut, so that one web page or one command cannot fill the model's context.
pub const MAX_RESULT_CHARS: usize = 20_000;

/// The tools that each agent type gets:
///
/// - `websearch` gets [`FetchUrl`].
/// - `coder` gets [`Bash`].
pub fn default_tools(agent: AgentType) -> Vec<Box<dyn Tool>> {
    match agent {
        AgentType::WebSearch => vec![Box::new(FetchUrl)],
        AgentType::Coder => vec![Box::new(Bash)],
    }
}

/// `fetch_url`: sends a `GET` request and returns the response body.
pub struct FetchUrl;

impl Tool for FetchUrl {
    fn name(&self) -> &str {
        "fetch_url"
    }

    fn description(&self) -> &str {
        "Sends an HTTP GET request to a URL and returns the response body."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"url": {"type": "string", "description": "The URL to get."}},
            "required": ["url"],
        })
    }

    fn call(&mut self, arguments: &Value) -> Result<String, String> {
        let url = string_argument(arguments, "url")?;
        let body = Actor::ApiCall {
            verb: ApiVerb::GET,
            url,
            input_file: None,
            payload: None,
            auth: None,
        }
        .act("")?;
        Ok(truncate(body))
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
