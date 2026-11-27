//! The file tools: `read_file`, `write_file` and `list_files`. They run the
//! same actors as the actions of a job file.

use std::sync::Arc;

use serde_json::{Value, json};

use super::{Tool, string_argument, truncate};
use crate::actor::Actor;
use crate::actor::list_files::DEFAULT_PATTERN;
use crate::protect::Protect;

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

    fn read_only(&self) -> bool {
        true
    }
}

/// `write_file`: writes text to a file, and makes the missing parent
/// directories. It does not write a protected path (see
/// [`Protect::check_write`]).
#[derive(Default)]
pub struct WriteFile {
    protect: Option<Arc<Protect>>,
}

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
        if let Some(protect) = &self.protect {
            protect.check_write(&path)?;
        }
        Actor::WriteFile { path: path.clone() }.act(&content)?;
        Ok(format!(
            "wrote {} characters to `{path}`",
            content.chars().count()
        ))
    }

    fn set_protect(&mut self, protect: Option<Arc<Protect>>) {
        self.protect = protect;
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

    fn read_only(&self) -> bool {
        true
    }
}
