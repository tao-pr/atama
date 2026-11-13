//! The judges of an `autonomous` agent: model requests that decide whether
//! the work meets the success criteria (`success=`), and whether a tool
//! call breaks a guardrail (`guard=judge`).
//!
//! A judge request is separate from the conversation of the agent. It has
//! a system prompt, one user message and no tools, and the reply must match
//! [`verdict_schema`]. The judge uses the client and the model of the
//! agent, and its tokens count in the token budget of the run.

use serde_json::{Value, json};

use super::checks::{CheckResult, FinalCheck, tail};
use super::context::SUMMARY_HEADER;
use super::{ChatClient, Message, ToolCall};
use crate::schema::OutputSchema;

/// Most characters of the tool calls and results that the success judge
/// gets as evidence. A longer evidence keeps its end.
pub const MAX_EVIDENCE_CHARS: usize = 10_000;

/// The decision of a judge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    /// For the success judge, whether the work meets the criteria. For the
    /// guard, whether the call can run.
    pub pass: bool,
    pub reason: String,
}

/// The schema of the reply of a judge: `{"pass": <bool>, "reason": <text>}`.
pub fn verdict_schema() -> OutputSchema {
    OutputSchema {
        name: "verdict".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "pass": {"type": "boolean"},
                "reason": {"type": "string"},
            },
            "required": ["pass", "reason"],
        }),
    }
}

/// Sends judge requests with a client, and keeps the token count of each
/// reply, so that the loop can add them to the budget.
pub struct Judge<'c> {
    client: &'c mut dyn ChatClient,
    /// The token count of each reply, in order.
    pub tokens: Vec<Option<u64>>,
}

impl<'c> Judge<'c> {
    pub fn new(client: &'c mut dyn ChatClient) -> Self {
        Judge {
            client,
            tokens: Vec::new(),
        }
    }

    /// Asks the model for a verdict. `system` says what the judge decides,
    /// and `question` has the facts.
    ///
    /// Fails if the client fails, or the reply does not match
    /// [`verdict_schema`].
    pub fn ask(&mut self, system: &str, question: &str) -> Result<Verdict, String> {
        let schema = verdict_schema();
        let messages = [
            Message::System(format!("{system}\n\n{}", schema.instruction())),
            Message::User(question.to_string()),
        ];
        let reply = self.client.chat_without_tools(&messages, Some(&schema))?;
        self.tokens.push(reply.tokens);
        let text = reply.content.unwrap_or_default();
        let json = schema
            .check_reply(&text)
            .map_err(|err| format!("the reply of the judge is not valid: {err}"))?;
        let value: Value = serde_json::from_str(&json).expect("check_reply gives JSON");
        Ok(Verdict {
            pass: value["pass"] == true,
            reason: value["reason"].as_str().unwrap_or_default().to_string(),
        })
    }
}

/// The system prompt of the success judge.
const SUCCESS_JUDGE_PROMPT: &str = "You are a strict judge. You decide whether the work of \
     an agent meets the success criteria of its task. Use only the evidence: the tool calls \
     of the agent, their results, and its final reply. If the evidence does not show that a \
     criterion is met, it is not met.";

/// The success check of `success=<criteria>`: a judge decides whether the
/// work meets the criteria. The output is the reply, unchanged. A failed
/// check ends the iteration.
pub struct SuccessJudge {
    task: String,
    criteria: String,
}

impl SuccessJudge {
    pub fn new(task: &str, criteria: &str) -> Self {
        SuccessJudge {
            task: task.to_string(),
            criteria: criteria.to_string(),
        }
    }
}

impl FinalCheck for SuccessJudge {
    fn check(&mut self, reply: &str, judge: &mut Judge, messages: &[Message]) -> CheckResult {
        let question = format!(
            "Task:\n{}\n\nSuccess criteria:\n{}\n\nThe tool calls of the agent and their \
             results, the most recent last:\n{}\n\nThe final reply of the agent:\n{reply}\n\n\
             Is \"pass\" true? It is true only if the work meets all the criteria.",
            self.task,
            self.criteria,
            evidence(messages),
        );
        let error = match judge.ask(SUCCESS_JUDGE_PROMPT, &question) {
            Ok(verdict) if verdict.pass => return CheckResult::Done(reply.to_string()),
            Ok(verdict) => verdict.reason,
            Err(err) => err,
        };
        CheckResult::Retry {
            message: format!(
                "The task is not done yet, because the work does not meet the success \
                 criteria:\n{error}\nContinue the work. When you are done, reply without \
                 tool calls."
            ),
            error,
        }
    }

    fn rejection(&self) -> String {
        "success criteria are not met".to_string()
    }

    fn failure(&self) -> String {
        "did not meet its success criteria".to_string()
    }

    fn ends_iteration(&self) -> bool {
        true
    }
}

/// The tool calls and the results of `messages`, one per line, as
/// `call <name>: <arguments>` and `result: <text>`, and the summary of a
/// compacted conversation (see [`super::context`]). Keeps the last
/// [`MAX_EVIDENCE_CHARS`] characters.
pub fn evidence(messages: &[Message]) -> String {
    let mut text = String::new();
    for message in messages {
        match message {
            Message::Assistant(reply) => {
                for call in &reply.tool_calls {
                    text.push_str(&format!("call {}: {}\n", call.name, call.arguments));
                }
            }
            Message::Tool { content, .. } => text.push_str(&format!("result: {content}\n")),
            // The summary of a compacted conversation has the earlier work.
            Message::User(summary) if summary.starts_with(SUMMARY_HEADER) => {
                text.push_str(&format!("{summary}\n"));
            }
            _ => {}
        }
    }
    if text.is_empty() {
        return "(no tool calls)".to_string();
    }
    tail(text.trim_end(), MAX_EVIDENCE_CHARS)
}

/// The system prompt of the guard.
const GUARD_PROMPT: &str = "You are a guard. You decide whether one tool call of an agent \
     breaks one of the guardrails. \"pass\" is true if the call breaks no guardrail. If it \
     breaks one, \"pass\" is false, and the reason names the guardrail.";

/// Asks the guard whether `call` breaks one of `guardrails`. Returns the
/// reason to block the call, or `None` if the call can run. A guard that
/// fails blocks the call, with the error as the reason.
pub fn review(judge: &mut Judge, guardrails: &str, call: &ToolCall) -> Option<String> {
    let question = format!(
        "Guardrails:\n{guardrails}\n\nTool: {}\nArguments:\n{}",
        call.name, call.arguments
    );
    match judge.ask(GUARD_PROMPT, &question) {
        Ok(verdict) if verdict.pass => None,
        Ok(verdict) => Some(verdict.reason),
        Err(err) => Some(err),
    }
}
