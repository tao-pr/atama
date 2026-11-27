//! The events of a session, as Atama reads them.
//!
//! Each event is a JSON object with a `type`, an `id` (`sevt_...`) and other
//! fields of its type. [`parse_event`] reads the events that a run must
//! handle into a [`SessionEvent`]. Each other type becomes
//! [`SessionEvent::Other`], so that a new event type of the API does not stop
//! a run.

use serde_json::Value;

/// One event of a session.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    /// `agent.message`: text from the agent.
    AgentMessage {
        id: String,
        /// The text blocks of the content, joined.
        text: String,
    },
    /// `agent.tool_use`: the agent calls a native tool.
    AgentToolUse {
        id: String,
        name: String,
        input: Value,
        permission: Permission,
    },
    /// `agent.tool_result`: the result of a native tool.
    AgentToolResult { id: String, is_error: bool },
    /// `agent.custom_tool_use`: the agent calls a custom tool. Atama runs
    /// it, and answers with a `user.custom_tool_result` event with this
    /// `id`.
    CustomToolUse {
        id: String,
        name: String,
        input: Value,
    },
    /// `session.status_running`.
    StatusRunning { id: String },
    /// `session.status_idle`, with the reason.
    StatusIdle { id: String, stop: IdleStop },
    /// `session.status_rescheduled`: a transient error, and the session
    /// tries again.
    StatusRescheduled { id: String },
    /// `session.status_terminated`: the session accepts no more events.
    StatusTerminated { id: String },
    /// `session.usage`: the usage of the session up to now.
    Usage { id: String, usage: Usage },
    /// `session.error`.
    Error {
        id: String,
        /// The message of the error, else its type, else its JSON.
        message: String,
        /// The `type` of `error.retry_status`, if the event has it.
        retry_status: Option<String>,
    },
    /// Each other type, as the event types that Atama sent.
    Other { id: String, kind: String },
}

impl SessionEvent {
    /// The `id` of the event. It is empty if the event has no `id`.
    pub fn id(&self) -> &str {
        match self {
            SessionEvent::AgentMessage { id, .. }
            | SessionEvent::AgentToolUse { id, .. }
            | SessionEvent::AgentToolResult { id, .. }
            | SessionEvent::CustomToolUse { id, .. }
            | SessionEvent::StatusRunning { id }
            | SessionEvent::StatusIdle { id, .. }
            | SessionEvent::StatusRescheduled { id }
            | SessionEvent::StatusTerminated { id }
            | SessionEvent::Usage { id, .. }
            | SessionEvent::Error { id, .. }
            | SessionEvent::Other { id, .. } => id,
        }
    }
}

/// The result of the permission policy for a call of a native tool, from
/// `evaluated_permission`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Permission {
    /// The tool runs. An event without `evaluated_permission` is also
    /// `Allow`.
    Allow,
    /// The call waits for a `user.tool_confirmation` event.
    Ask,
    /// The tool does not run.
    Deny,
    /// A value that Atama does not know.
    Other(String),
}

/// Why a session is idle, from `stop_reason.type` of `session.status_idle`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdleStop {
    /// The agent finished its turn.
    EndTurn,
    /// The session waits for a custom tool result or a tool confirmation.
    /// `event_ids` are the events that wait.
    RequiresAction { event_ids: Vec<String> },
    /// The session failed after its retries.
    RetriesExhausted,
    /// The session used its budget, and is paused.
    BudgetReached,
    /// A value that Atama does not know.
    Other(String),
}

/// The usage of a session up to now, from the `usage` object of a
/// `session.usage` event. A missing count is 0.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    /// The sum of the counts in `cache_creation`.
    pub cache_creation_input_tokens: u64,
    /// From `list_cost.amount`, in cents. `None` if the event has no cost.
    pub list_cost_cents: Option<u64>,
}

impl Usage {
    /// Reads a `usage` object.
    pub fn from_json(usage: &Value) -> Usage {
        let count = |value: &Value| value.as_u64().unwrap_or(0);
        let cache_creation = match usage.get("cache_creation") {
            Some(Value::Object(counts)) => counts.values().map(count).sum(),
            _ => 0,
        };
        Usage {
            input_tokens: count(&usage["input_tokens"]),
            output_tokens: count(&usage["output_tokens"]),
            cache_read_input_tokens: count(&usage["cache_read_input_tokens"]),
            cache_creation_input_tokens: cache_creation,
            list_cost_cents: usage["list_cost"]["amount"]
                .as_str()
                .and_then(|amount| amount.parse().ok()),
        }
    }

    /// All tokens: input, output, cache reads and cache writes.
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            + self.output_tokens
            + self.cache_read_input_tokens
            + self.cache_creation_input_tokens
    }
}

/// Reads one event. Fails if the event has no `type`.
pub fn parse_event(event: &Value) -> Result<SessionEvent, String> {
    let kind = event["type"]
        .as_str()
        .ok_or_else(|| format!("event without a type: {event}"))?;
    let id = event["id"].as_str().unwrap_or_default().to_string();
    let name = || event["name"].as_str().unwrap_or_default().to_string();
    let input = || event.get("input").cloned().unwrap_or(Value::Null);
    let parsed = match kind {
        "agent.message" => SessionEvent::AgentMessage {
            id,
            text: content_text(&event["content"]),
        },
        "agent.tool_use" => SessionEvent::AgentToolUse {
            id,
            name: name(),
            input: input(),
            permission: match event["evaluated_permission"].as_str() {
                None | Some("allow") => Permission::Allow,
                Some("ask") => Permission::Ask,
                Some("deny") => Permission::Deny,
                Some(other) => Permission::Other(other.to_string()),
            },
        },
        "agent.tool_result" => SessionEvent::AgentToolResult {
            id,
            is_error: event["is_error"].as_bool().unwrap_or(false),
        },
        "agent.custom_tool_use" => SessionEvent::CustomToolUse {
            id,
            name: name(),
            input: input(),
        },
        "session.status_running" => SessionEvent::StatusRunning { id },
        "session.status_idle" => SessionEvent::StatusIdle {
            id,
            stop: idle_stop(&event["stop_reason"]),
        },
        "session.status_rescheduled" => SessionEvent::StatusRescheduled { id },
        "session.status_terminated" => SessionEvent::StatusTerminated { id },
        "session.usage" => SessionEvent::Usage {
            id,
            usage: Usage::from_json(&event["usage"]),
        },
        "session.error" => {
            let error = &event["error"];
            let message = error["message"]
                .as_str()
                .or_else(|| error["type"].as_str())
                .map(str::to_string)
                .unwrap_or_else(|| error.to_string());
            let retry_status = &error["retry_status"];
            SessionEvent::Error {
                id,
                message,
                retry_status: retry_status["type"]
                    .as_str()
                    .or_else(|| retry_status.as_str())
                    .map(str::to_string),
            }
        }
        _ => SessionEvent::Other {
            id,
            kind: kind.to_string(),
        },
    };
    Ok(parsed)
}

/// The text blocks of `content`, joined. It skips the other blocks.
fn content_text(content: &Value) -> String {
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect()
}

fn idle_stop(stop_reason: &Value) -> IdleStop {
    match stop_reason["type"].as_str().unwrap_or_default() {
        "end_turn" => IdleStop::EndTurn,
        "requires_action" => IdleStop::RequiresAction {
            event_ids: stop_reason["event_ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|id| id.as_str().map(str::to_string))
                .collect(),
        },
        "retries_exhausted" => IdleStop::RetriesExhausted,
        "budget_reached" => IdleStop::BudgetReached,
        other => IdleStop::Other(other.to_string()),
    }
}
