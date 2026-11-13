//! Built-in tools for the agent loop. They run the same actors as the
//! actions of a job file.

use serde_json::{Value, json};

use super::Tool;
use std::time::Instant;

use crate::actor::list_files::DEFAULT_PATTERN;
use crate::actor::{Actor, AgentType, run_bash};

/// Most characters of a tool result that the model gets. A longer result is
/// cut, so that one command cannot fill the model's context.
pub const MAX_RESULT_CHARS: usize = 20_000;

/// The names of all tools that [`tool_by_name`] can make.
pub const TOOL_NAMES: [&str; 4] = ["bash", "read_file", "write_file", "list_files"];

/// Makes the tool with the name `name`, or returns `None` if no tool has
/// that name. [`TOOL_NAMES`] has all names.
pub fn tool_by_name(name: &str) -> Option<Box<dyn Tool>> {
    let tool: Box<dyn Tool> = match name {
        "bash" => Box::new(Bash::default()),
        "read_file" => Box::new(ReadFile),
        "write_file" => Box::new(WriteFile),
        "list_files" => Box::new(ListFiles),
        _ => return None,
    };
    Some(tool)
}

/// The tools of an `autonomous` agent.
pub const AUTONOMOUS_DEFAULT_TOOLS: [&str; 4] = ["bash", "read_file", "write_file", "list_files"];
/// The most calls of each tool of an `autonomous` agent in one run.
pub const DEFAULT_MAX_CALLS: u32 = 50;
/// The most failed calls of each tool of an `autonomous` agent in one run.
pub const DEFAULT_MAX_FAILURES: u32 = 10;

/// The names of the tools that each agent type gets:
///
/// - `coder` gets [`Bash`].
/// - `summarizer` gets no tools, because it only rewrites its input.
/// - `autonomous` gets [`AUTONOMOUS_DEFAULT_TOOLS`].
pub fn default_tool_names(agent: AgentType) -> &'static [&'static str] {
    match agent {
        AgentType::Coder => &["bash"],
        AgentType::Summarizer => &[],
        AgentType::Autonomous => &AUTONOMOUS_DEFAULT_TOOLS,
    }
}

/// The tools that each agent type gets (see [`default_tool_names`]). The
/// tools of an `autonomous` agent are [`Limited`], with
/// [`DEFAULT_MAX_CALLS`] and [`DEFAULT_MAX_FAILURES`].
pub fn default_tools(agent: AgentType) -> Vec<Box<dyn Tool>> {
    let tools = default_tool_names(agent)
        .iter()
        .filter_map(|name| tool_by_name(name));
    match agent {
        AgentType::Autonomous => tools
            .map(|tool| -> Box<dyn Tool> {
                Box::new(Limited::new(
                    tool,
                    Some(DEFAULT_MAX_CALLS),
                    Some(DEFAULT_MAX_FAILURES),
                ))
            })
            .collect(),
        _ => tools.collect(),
    }
}

/// A tool with a limit of calls and a limit of failed calls for one run.
///
/// A call at a limit does not run the tool, and fails. A tool at a limit is
/// not available (see [`Tool::available`]), so the next request does not
/// offer it to the model. `None` sets no limit.
pub struct Limited {
    tool: Box<dyn Tool>,
    max_calls: Option<u32>,
    max_failures: Option<u32>,
    calls: u32,
    failures: u32,
}

impl Limited {
    pub fn new(tool: Box<dyn Tool>, max_calls: Option<u32>, max_failures: Option<u32>) -> Self {
        Limited {
            tool,
            max_calls,
            max_failures,
            calls: 0,
            failures: 0,
        }
    }

    /// The error of a call at a limit, or `None` if the tool can run.
    fn limit_error(&self) -> Option<String> {
        let name = self.tool.name();
        if let Some(max) = self.max_calls
            && self.calls >= max
        {
            return Some(format!("tool `{name}` used its limit of {max} calls"));
        }
        if let Some(max) = self.max_failures
            && self.failures >= max
        {
            return Some(format!(
                "tool `{name}` failed {max} times, which is its limit"
            ));
        }
        None
    }
}

impl Tool for Limited {
    fn name(&self) -> &str {
        self.tool.name()
    }

    fn description(&self) -> &str {
        self.tool.description()
    }

    fn parameters(&self) -> Value {
        self.tool.parameters()
    }

    /// Counts the call, and counts a failed call. A call at a limit fails
    /// and does not count.
    fn call(&mut self, arguments: &Value) -> Result<String, String> {
        if let Some(error) = self.limit_error() {
            return Err(error);
        }
        self.calls += 1;
        let result = self.tool.call(arguments);
        if result.is_err() {
            self.failures += 1;
        }
        result
    }

    fn available(&self) -> bool {
        self.limit_error().is_none()
    }

    fn set_deadline_time(&mut self, deadline_time: Option<Instant>) {
        self.tool.set_deadline_time(deadline_time);
    }
}

/// `bash`: runs a command in bash in the current directory and returns its
/// stdout. With a deadline, it stops the command and its child processes at
/// the deadline (see [`Tool::set_deadline_time`]).
#[derive(Default)]
pub struct Bash {
    deadline_time: Option<Instant>,
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
        let output = run_bash(&command, &[], "", self.deadline_time)?;
        Ok(truncate(output))
    }

    fn set_deadline_time(&mut self, deadline_time: Option<Instant>) {
        self.deadline_time = deadline_time;
    }
}

/// `read_file`: returns the content of a text file.
pub struct ReadFile;

impl Tool for ReadFile {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Returns the content of a text file. The path is relative to the current directory."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"path": {"type": "string", "description": "The path of the file."}},
            "required": ["path"],
        })
    }

    fn call(&mut self, arguments: &Value) -> Result<String, String> {
        let output = Actor::ReadFile {
            path: Some(string_argument(arguments, "path")?),
        }
        .act("")?;
        Ok(truncate(output))
    }
}

/// `write_file`: writes text to a file, and makes the missing parent
/// directories.
pub struct WriteFile;

impl Tool for WriteFile {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Writes the content to a file, and makes the missing parent directories. \
         The file gets only the new content. The path is relative to the current directory."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "The path of the file."},
                "content": {"type": "string", "description": "The new content of the file."},
            },
            "required": ["path", "content"],
        })
    }

    /// Returns a short confirmation, not the content, so that the model
    /// does not get the content back.
    fn call(&mut self, arguments: &Value) -> Result<String, String> {
        let path = string_argument(arguments, "path")?;
        let content = string_argument(arguments, "content")?;
        Actor::WriteFile { path: path.clone() }.act(&content)?;
        Ok(format!(
            "wrote {} characters to `{path}`",
            content.chars().count()
        ))
    }
}

/// `list_files`: lists the files of one directory whose names match a glob
/// pattern.
pub struct ListFiles;

impl Tool for ListFiles {
    fn name(&self) -> &str {
        "list_files"
    }

    fn description(&self) -> &str {
        "Lists the files of one directory whose names match a glob pattern, one path per \
         line, sorted. It does not go into subdirectories."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "dir": {"type": "string", "description": "The directory."},
                "pattern": {
                    "type": "string",
                    "description": "The glob pattern of the file names, as in `*.rs`. The default is `*`.",
                },
            },
            "required": ["dir"],
        })
    }

    fn call(&mut self, arguments: &Value) -> Result<String, String> {
        let pattern = match arguments.get("pattern") {
            None => DEFAULT_PATTERN.to_string(),
            Some(_) => string_argument(arguments, "pattern")?,
        };
        let output = Actor::ListFiles {
            dir: Some(string_argument(arguments, "dir")?),
            pattern,
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
