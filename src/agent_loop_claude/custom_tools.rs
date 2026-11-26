//! The local tools of a `claude-agent` action, from `local_tools=`.
//!
//! A local tool is a tool of the shared registry ([`crate::tools`]) that
//! runs in Atama. The agent gets it as a custom tool with the name
//! `local_<name>`, as in `local_bash`. The API rejects a custom tool with the
//! name of a native tool, and `bash` is both. The prefix also tells the model
//! where the tool runs.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use serde_json::{Value, json};

use super::native_tools::split_list;
use crate::protect::Protect;
use crate::tools::{
    Approval, Approver, Autonomy, DEFAULT_MAX_CALLS, DEFAULT_MAX_FAILURES, Limited, TOOL_NAMES,
    Tool, tool_by_name,
};

/// The prefix of the name of each local tool in the API.
pub const LOCAL_PREFIX: &str = "local_";

/// The name of the local tool `name` in the API, as in `local_bash`.
pub fn api_name(name: &str) -> String {
    format!("{LOCAL_PREFIX}{name}")
}

/// The custom tool entry of `tool` in the `tools` of an agent.
pub fn custom_tool_json(tool: &dyn Tool) -> Value {
    json!({
        "type": "custom",
        "name": api_name(tool.name()),
        "description": tool.description(),
        "input_schema": tool.parameters(),
    })
}

/// Reads the value of `local_tools=`: names of the shared registry that
/// commas separate.
///
/// Fails on an empty list, a name that is not in [`TOOL_NAMES`], or a name
/// that is in the list twice.
pub fn parse_names(value: &str) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = Vec::new();
    for name in split_list(value) {
        if !TOOL_NAMES.contains(&name) {
            let known: Vec<String> = TOOL_NAMES.iter().map(|name| format!("`{name}`")).collect();
            return Err(format!(
                "`{name}` is not a local tool, expected {}",
                known.join(", ")
            ));
        }
        if names.iter().any(|known| known == name) {
            return Err(format!("local tool `{name}` is in `local_tools=` twice"));
        }
        names.push(name.to_string());
    }
    if names.is_empty() {
        return Err("`local_tools=` has no tool".to_string());
    }
    Ok(names)
}

/// The local tools of one run.
pub struct LocalTools {
    tools: Vec<Box<dyn Tool>>,
}

impl LocalTools {
    /// Makes the tools `names` from the registry. Each one is in a
    /// [`Limited`] with [`DEFAULT_MAX_CALLS`] and [`DEFAULT_MAX_FAILURES`],
    /// and in an [`Approval`] with `autonomy` and `approver`. A denied call
    /// does not count as a call of the tool.
    ///
    /// Fails if a name is not in the registry.
    pub fn from_names(
        names: &[String],
        autonomy: Autonomy,
        approver: Rc<RefCell<dyn Approver>>,
    ) -> Result<LocalTools, String> {
        let tools = names
            .iter()
            .map(|name| {
                let tool =
                    tool_by_name(name).ok_or_else(|| format!("`{name}` is not a local tool"))?;
                let limited =
                    Limited::new(tool, Some(DEFAULT_MAX_CALLS), Some(DEFAULT_MAX_FAILURES));
                let tool: Box<dyn Tool> =
                    Box::new(Approval::new(Box::new(limited), autonomy, approver.clone()));
                Ok(tool)
            })
            .collect::<Result<_, String>>()?;
        Ok(LocalTools { tools })
    }

    /// Local tools from `tools`, without limits or approval. Tests use it
    /// to give a run mock tools.
    pub fn with_tools(tools: Vec<Box<dyn Tool>>) -> LocalTools {
        LocalTools { tools }
    }

    /// The custom tool entries of the tools, for the `tools` of an agent.
    pub fn definitions(&self) -> Vec<Value> {
        self.tools
            .iter()
            .map(|tool| custom_tool_json(tool.as_ref()))
            .collect()
    }

    /// Whether a tool has the API name `api_name`.
    pub fn has(&self, api_name: &str) -> bool {
        self.find(api_name).is_some()
    }

    fn find(&self, api_name: &str) -> Option<usize> {
        let name = api_name.strip_prefix(LOCAL_PREFIX)?;
        self.tools.iter().position(|tool| tool.name() == name)
    }

    /// Calls the tool with the API name `api_name` with `input`.
    ///
    /// Fails if no tool has the name, or the tool fails.
    pub fn call(&mut self, api_name: &str, input: &Value) -> Result<String, String> {
        let index = self
            .find(api_name)
            .ok_or_else(|| format!("unknown tool `{api_name}`"))?;
        self.tools[index].call(input)
    }

    /// Gives each tool the deadline of the run.
    pub fn set_deadline_time(&mut self, deadline_time: Option<Instant>) {
        for tool in &mut self.tools {
            tool.set_deadline_time(deadline_time);
        }
    }

    /// Gives each tool the protected paths of the run.
    pub fn set_protect(&mut self, protect: Option<Arc<Protect>>) {
        for tool in &mut self.tools {
            tool.set_protect(protect.clone());
        }
    }
}
