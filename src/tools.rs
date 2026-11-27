//! The tools that the model of an agent can call.
//!
//! The tools do not depend on an API. Each agent loop has an adapter that
//! writes a [`Tool`] in the tool format of its API. The OpenAI compatible
//! loop (see [`crate::agent_loop_openai_compat`]) uses [`Tool::name`],
//! [`Tool::description`] and [`Tool::parameters`] in its requests.
//!
//! The registry ([`TOOL_NAMES`] and [`tool_by_name`]) makes each tool by its
//! name. To add a tool for all agents, do these steps:
//!
//! 1. Write a type that implements [`Tool`] in a file of `src/tools/`.
//! 2. Add the tool to [`TOOL_NAMES`] and [`tool_by_name`] in
//!    `src/tools/registry.rs`.
//! 3. Add tests to `tests/tools.rs`.

use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;

use crate::protect::Protect;

mod approval;
mod bash;
mod files;
mod limited;
mod registry;
mod web_search;

pub use approval::{Approval, Approver, Autonomy, DenyApprover, PromptApprover};
pub use bash::Bash;
pub use files::{ListFiles, ReadFile, WriteFile};
pub use limited::{DEFAULT_MAX_CALLS, DEFAULT_MAX_FAILURES, Limited};
pub use registry::{TOOL_NAMES, tool_by_name};
pub use web_search::WebSearch;

/// A tool that the model can call.
pub trait Tool {
    /// The name that the model uses to call the tool.
    fn name(&self) -> &str;
    /// Tells the model what the tool does.
    fn description(&self) -> &str;
    /// JSON schema of the arguments.
    fn parameters(&self) -> Value;
    /// Runs the tool and returns its result for the model.
    fn call(&mut self, arguments: &Value) -> Result<String, String>;
    /// Whether the model can call the tool now. A request offers only the
    /// available tools. A tool at its call limit is not available (see
    /// [`Limited`]).
    fn available(&self) -> bool {
        true
    }
    /// Sets the deadline of the run. A tool that runs a command stops it
    /// at the deadline, as `bash` does (see [`crate::actor::run_process`]).
    /// The default does nothing.
    fn set_deadline_time(&mut self, _deadline_time: Option<Instant>) {}
    /// Sets the protected paths of the run, from `protect=`. A tool that
    /// writes files must not write them, as `bash` and `write_file` do (see
    /// [`crate::protect`]). The default does nothing.
    fn set_protect(&mut self, _protect: Option<Arc<Protect>>) {}
    /// Whether the tool only reads, and changes nothing. With
    /// `autonomy=ask_if_risky`, a call of a tool that is not read-only
    /// waits for approval (see [`Approval`]). The default is `false`, so a
    /// new tool waits for approval until its author marks it.
    fn read_only(&self) -> bool {
        false
    }
}

/// Most characters of a tool result that the model gets. A longer result is
/// cut, so that one command cannot fill the model's context.
pub const MAX_RESULT_CHARS: usize = 20_000;

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

/// Reads the string argument `name` of a tool call.
fn string_argument(arguments: &Value, name: &str) -> Result<String, String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("missing string argument `{name}`"))
}
