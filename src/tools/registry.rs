//! The registry: makes each tool by its name.

use super::{Bash, ListFiles, ReadFile, Tool, WebSearch, WriteFile};

/// The names of all tools that [`tool_by_name`] can make.
pub const TOOL_NAMES: [&str; 5] = [
    "bash",
    "read_file",
    "write_file",
    "list_files",
    "web_search",
];

/// Makes the tool with the name `name`, or returns `None` if no tool has
/// that name. [`TOOL_NAMES`] has all names. Each call makes a new tool, so
/// that two agents do not share the state of a tool.
pub fn tool_by_name(name: &str) -> Option<Box<dyn Tool>> {
    let tool: Box<dyn Tool> = match name {
        "bash" => Box::new(Bash::default()),
        "read_file" => Box::new(ReadFile),
        "write_file" => Box::new(WriteFile::default()),
        "list_files" => Box::new(ListFiles),
        "web_search" => Box::new(WebSearch::default()),
        _ => return None,
    };
    Some(tool)
}
