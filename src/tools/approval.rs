//! The levels of autonomy of an agent, from `autonomy=`, and the approval
//! of the tool calls that must wait.
//!
//! [`Autonomy`] tells whether a call must wait for approval. An
//! [`Approver`], from `on_ask=`, answers the call. [`Approval`] wraps a tool,
//! so that each call that must wait asks the approver first.

use std::cell::RefCell;
use std::fmt;
use std::io::{self, BufRead, IsTerminal, Write};
use std::rc::Rc;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::Value;

use super::Tool;
use crate::protect::Protect;

/// When a tool call must wait for approval, from `autonomy=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Autonomy {
    /// `no_ask`: each call runs.
    NoAsk,
    /// `ask_if_risky`: a call of a tool that only reads runs. A call of a
    /// tool that can change something waits.
    AskIfRisky,
    /// `always_ask`: each call waits.
    AlwaysAsk,
}

impl Autonomy {
    /// Whether a call of `tool` must wait for approval.
    pub fn must_ask(self, tool: &dyn Tool) -> bool {
        match self {
            Autonomy::NoAsk => false,
            Autonomy::AskIfRisky => !tool.read_only(),
            Autonomy::AlwaysAsk => true,
        }
    }
}

impl FromStr for Autonomy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, String> {
        match value {
            "no_ask" => Ok(Autonomy::NoAsk),
            "ask_if_risky" => Ok(Autonomy::AskIfRisky),
            "always_ask" => Ok(Autonomy::AlwaysAsk),
            _ => Err(format!(
                "`autonomy={value}` is not `no_ask`, `ask_if_risky` or `always_ask`"
            )),
        }
    }
}

impl fmt::Display for Autonomy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Autonomy::NoAsk => "no_ask",
            Autonomy::AskIfRisky => "ask_if_risky",
            Autonomy::AlwaysAsk => "always_ask",
        })
    }
}

/// Answers a tool call that waits for approval.
pub trait Approver {
    /// `Ok(())` allows the call of `tool` with `input`. `Err(reason)`
    /// denies it, and the model gets the reason.
    fn approve(&mut self, tool: &str, input: &Value) -> Result<(), String>;
}

/// `on_ask=deny`: denies each call.
pub struct DenyApprover;

impl Approver for DenyApprover {
    fn approve(&mut self, tool: &str, _input: &Value) -> Result<(), String> {
        Err(format!(
            "the call of `{tool}` was denied, because `on_ask=deny` denies each call that \
             waits for approval"
        ))
    }
}

/// Makes the prompts of parallel actions come one after the other.
static PROMPT_LOCK: Mutex<()> = Mutex::new(());

/// `on_ask=prompt`: asks the user on the terminal. If stdin is not a
/// terminal, for example in CI, it denies each call.
pub struct PromptApprover {
    is_terminal: bool,
    input: Box<dyn BufRead>,
    output: Box<dyn Write>,
}

impl PromptApprover {
    /// Asks on stdin, and writes the prompt to stderr.
    pub fn new() -> Self {
        let stdin = io::stdin();
        PromptApprover {
            is_terminal: stdin.is_terminal(),
            input: Box::new(stdin.lock()),
            output: Box::new(io::stderr()),
        }
    }

    /// Asks on `input`, and writes the prompt to `output`. `is_terminal`
    /// tells whether a person can answer. Tests use it.
    pub fn with_io(is_terminal: bool, input: Box<dyn BufRead>, output: Box<dyn Write>) -> Self {
        PromptApprover {
            is_terminal,
            input,
            output,
        }
    }
}

impl Default for PromptApprover {
    fn default() -> Self {
        Self::new()
    }
}

impl Approver for PromptApprover {
    fn approve(&mut self, tool: &str, input: &Value) -> Result<(), String> {
        if !self.is_terminal {
            return Err(format!(
                "the call of `{tool}` was denied, because no terminal can ask the user \
                 (`on_ask=prompt`)"
            ));
        }
        let _lock = PROMPT_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let denied = || format!("the user denied the call of `{tool}`");
        let input = serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string());
        write!(
            self.output,
            "The agent wants to call `{tool}` with:\n{input}\nallow? [y/N] "
        )
        .and_then(|()| self.output.flush())
        .map_err(|_| denied())?;
        let mut answer = String::new();
        self.input.read_line(&mut answer).map_err(|_| denied())?;
        match answer.trim().to_lowercase().as_str() {
            "y" | "yes" => Ok(()),
            _ => Err(denied()),
        }
    }
}

/// A tool whose calls ask an [`Approver`] first, as [`Autonomy`] says. A
/// denied call does not run, and fails with the reason of the approver.
pub struct Approval {
    tool: Box<dyn Tool>,
    autonomy: Autonomy,
    approver: Rc<RefCell<dyn Approver>>,
}

impl Approval {
    /// Wraps `tool`. All tools of one run share one `approver`.
    pub fn new(
        tool: Box<dyn Tool>,
        autonomy: Autonomy,
        approver: Rc<RefCell<dyn Approver>>,
    ) -> Self {
        Approval {
            tool,
            autonomy,
            approver,
        }
    }
}

impl Tool for Approval {
    fn name(&self) -> &str {
        self.tool.name()
    }

    fn description(&self) -> &str {
        self.tool.description()
    }

    fn parameters(&self) -> Value {
        self.tool.parameters()
    }

    fn call(&mut self, arguments: &Value) -> Result<String, String> {
        if self.autonomy.must_ask(self.tool.as_ref()) {
            self.approver
                .borrow_mut()
                .approve(self.tool.name(), arguments)?;
        }
        self.tool.call(arguments)
    }

    fn available(&self) -> bool {
        self.tool.available()
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
