//! The tools of each agent type of the OpenAI compatible loop.
//!
//! The tools are in the shared tool layer, [`crate::tools`], so that every
//! agent loop can use them. This module selects the tools of each
//! [`AgentType`], and re-exports the shared tools at their old paths.

pub use crate::tools::{
    Bash, DEFAULT_MAX_CALLS, DEFAULT_MAX_FAILURES, Limited, ListFiles, MAX_RESULT_CHARS, ReadFile,
    TOOL_NAMES, WriteFile, tool_by_name, truncate,
};

use super::Tool;
use crate::actor::AgentType;

/// The tools of an `autonomous` agent.
pub const AUTONOMOUS_DEFAULT_TOOLS: [&str; 4] = ["bash", "read_file", "write_file", "list_files"];

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
