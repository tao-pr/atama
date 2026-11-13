//! Final checks: rules that a reply without tool calls must pass before it
//! is the output of the agent loop.
//!
//! If a check fails, the model gets the reason as a user message, and the
//! loop goes on. [`SchemaCheck`] makes sure that the reply matches an
//! output schema.

use crate::schema::OutputSchema;

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
    /// Examines `reply`, the text of a reply without tool calls.
    fn check(&mut self, reply: &str) -> CheckResult;
    /// Says what failed, for the debug header of a failed check, as in
    /// "reply does not match output schema `article`".
    fn rejection(&self) -> String;
    /// Says what the run did not do if the last failed check is this one,
    /// as in "did not reply with JSON that matches output schema `article`".
    fn failure(&self) -> String;
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
pub fn run_checks(checks: &mut [Box<dyn FinalCheck>], reply: &str) -> Result<String, Rejection> {
    let mut output = reply.to_string();
    for (index, check) in checks.iter_mut().enumerate() {
        match check.check(&output) {
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
    fn check(&mut self, reply: &str) -> CheckResult {
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
