//! [`Limited`]: a tool with a limit of calls and of failed calls.

use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;

use super::Tool;
use crate::protect::Protect;

/// The default most calls of each tool in one run, for example of an
/// `autonomous` agent.
pub const DEFAULT_MAX_CALLS: u32 = 50;
/// The default most failed calls of each tool in one run, for example of an
/// `autonomous` agent.
pub const DEFAULT_MAX_FAILURES: u32 = 10;

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

    fn set_protect(&mut self, protect: Option<Arc<Protect>>) {
        self.tool.set_protect(protect);
    }

    fn read_only(&self) -> bool {
        self.tool.read_only()
    }
}
