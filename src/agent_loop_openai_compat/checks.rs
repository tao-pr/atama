//! Final checks: rules that a reply without tool calls must pass before it
//! is the output of the agent loop.
//!
//! If a check fails, the model gets the reason as a user message, and the
//! loop goes on. [`SchemaCheck`] makes sure that the reply matches an
//! output schema.

use std::process::Command;
use std::time::Instant;

use super::Message;
use super::judge::Judge;
use crate::actor::run_process;

use crate::schema::OutputSchema;

/// Most characters of the output of a failed success check that the model
/// gets. A longer output keeps its end, because test runners print the
/// summary last.
pub const MAX_CHECK_CHARS: usize = 5_000;

/// The result of a [`FinalCheck`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckResult {
    /// The reply passes. The value is the output, which can be different
    /// from the reply, for example the JSON part of it.
    Done(String),
    /// The reply fails. The model gets `message` as a user message, and the
    /// loop goes on. `error` is the reason, for the debug output and the
    /// error of the run.
    Retry { message: String, error: String },
}

/// A rule that a reply without tool calls must pass.
pub trait FinalCheck {
    /// Examines `reply`, the text of a reply without tool calls. A check
    /// that asks a model uses `judge`. `messages` is the conversation so
    /// far, without the reply.
    fn check(&mut self, reply: &str, judge: &mut Judge, messages: &[Message]) -> CheckResult;
    /// Says what failed, for the debug header of a failed check, as in
    /// "reply does not match output schema `article`".
    fn rejection(&self) -> String;
    /// Says what the run did not do if the last failed check is this one,
    /// as in "did not reply with JSON that matches output schema `article`".
    fn failure(&self) -> String;
    /// Whether a failed check ends the iteration, so that the next one
    /// starts. A check of the form of a reply, such as [`SchemaCheck`],
    /// does not end it. A check of the work, such as [`CommandCheck`], ends
    /// it.
    fn ends_iteration(&self) -> bool {
        false
    }
}

/// A failed check of [`run_checks`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    /// The index of the check that failed.
    pub index: usize,
    /// The user message for the model.
    pub message: String,
    /// The reason.
    pub error: String,
}

/// Runs `checks` in order on `reply`, and returns the output. Each check
/// gets the output of the check before it. Without checks, the output is
/// `reply`.
///
/// Fails at the first check that gives [`CheckResult::Retry`].
pub fn run_checks(
    checks: &mut [Box<dyn FinalCheck>],
    reply: &str,
    judge: &mut Judge,
    messages: &[Message],
) -> Result<String, Rejection> {
    let mut output = reply.to_string();
    for (index, check) in checks.iter_mut().enumerate() {
        match check.check(&output, judge, messages) {
            CheckResult::Done(next) => output = next,
            CheckResult::Retry { message, error } => {
                return Err(Rejection {
                    index,
                    message,
                    error,
                });
            }
        }
    }
    Ok(output)
}

/// Makes sure that the reply is JSON that matches an output schema (see
/// [`OutputSchema::check_reply`]). The output is the JSON value.
pub struct SchemaCheck {
    schema: OutputSchema,
}

impl SchemaCheck {
    pub fn new(schema: OutputSchema) -> Self {
        SchemaCheck { schema }
    }
}

impl FinalCheck for SchemaCheck {
    fn check(&mut self, reply: &str, _: &mut Judge, _: &[Message]) -> CheckResult {
        match self.schema.check_reply(reply) {
            Ok(json) => CheckResult::Done(json),
            Err(error) => CheckResult::Retry {
                message: format!(
                    "Your reply is not valid: {error}\nReply again with only the JSON value."
                ),
                error,
            },
        }
    }

    fn rejection(&self) -> String {
        format!("reply does not match output schema `{}`", self.schema.name)
    }

    fn failure(&self) -> String {
        format!(
            "did not reply with JSON that matches output schema `{}`",
            self.schema.name
        )
    }
}

/// The success check of an `autonomous` agent: runs a bash command in the
/// current directory, and passes if the command exits with status 0. The
/// output is the reply, unchanged. If the command fails, the error has its
/// exit status, and its stdout and stderr together (see [`tail`]).
pub struct CommandCheck {
    command: String,
    /// The deadline of the run, if set.
    deadline_time: Option<Instant>,
}

impl CommandCheck {
    pub fn new(command: String) -> Self {
        CommandCheck {
            command,
            deadline_time: None,
        }
    }

    /// Sets the deadline of the run. If the command still runs at the
    /// deadline, Atama stops it and its child processes, and the check
    /// fails (see [`run_process`]).
    pub fn with_deadline_time(mut self, deadline_time: Option<Instant>) -> Self {
        self.deadline_time = deadline_time;
        self
    }

    /// Runs the command. Fails with the exit status and the output.
    fn run(&self) -> Result<(), String> {
        let mut command = Command::new("bash");
        command.arg("-c").arg(&self.command);
        let output = run_process(command, "bash", &self.command, "", self.deadline_time)?;
        if output.status.success() {
            return Ok(());
        }
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Err(format!(
            "{}\n{}",
            output.status,
            tail(text.trim(), MAX_CHECK_CHARS)
        ))
    }
}

impl FinalCheck for CommandCheck {
    fn check(&mut self, reply: &str, _: &mut Judge, _: &[Message]) -> CheckResult {
        match self.run() {
            Ok(()) => CheckResult::Done(reply.to_string()),
            Err(error) => CheckResult::Retry {
                message: format!(
                    "The task is not done yet, because the success check `{}` failed:\n\
                     {error}\nContinue the work. When you are done, reply without tool calls.",
                    self.command
                ),
                error,
            },
        }
    }

    fn rejection(&self) -> String {
        format!("success check `{}` failed", self.command)
    }

    fn failure(&self) -> String {
        format!("did not pass its success check `{}`", self.command)
    }

    fn ends_iteration(&self) -> bool {
        true
    }
}

/// The last `max` characters of `text`. A cut text starts with a line that
/// says so.
pub fn tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let start: String = text.chars().skip(count - max).collect();
    format!("[cut: the output has {count} characters, these are the last {max}]\n{start}")
}
