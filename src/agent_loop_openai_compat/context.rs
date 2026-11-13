//! Compaction of the context: if the conversation of an agent gets longer
//! than `max_context=<n>` tokens, the loop replaces it with a summary.
//!
//! Before each request, the loop estimates the tokens of the conversation
//! with [`estimate_tokens`]. If the estimate is more than the limit, the
//! loop sends [`summary_request`] to the model at once, and goes on with
//! [`compacted`]: the system prompt, the task, and one user message with
//! the summary. Without `max_context=`, the loop never summarizes.

use super::{Message, openai};

/// The characters of one token in [`estimate_tokens`]. Atama has no
/// tokenizer, and many models use about 4 characters of English text for
/// one token.
pub const CHARS_PER_TOKEN: usize = 4;

/// The first line of the message that holds the summary.
pub const SUMMARY_HEADER: &str = "Summary of the work so far:";

/// The system prompt of the summary request.
pub const SUMMARY_PROMPT: &str = "You compress the conversation of an agent, so that the \
     agent can go on with its task with less context. Write a summary of the work so far: \
     what the agent did and found, the files that it read or changed, the commands that it \
     ran and their results, the errors, the feedback that it got, and what is left to do. \
     Keep names, paths, numbers, commands and error messages exactly. Do not add facts. Reply \
     with the summary only.";

/// The estimated tokens of `messages`: the characters of their JSON form,
/// divided by [`CHARS_PER_TOKEN`], rounded up.
pub fn estimate_tokens(messages: &[Message]) -> u64 {
    let chars: usize = messages
        .iter()
        .map(|message| openai::message_json(message).to_string().chars().count())
        .sum();
    chars.div_ceil(CHARS_PER_TOKEN) as u64
}

/// The messages after the system prompt and the task as text, one line or
/// block for each message: `assistant: <text>`, `call <name>: <arguments>`,
/// `result: <text>` and `user: <text>`.
pub fn transcript(messages: &[Message]) -> String {
    let mut lines = Vec::new();
    for message in messages {
        match message {
            Message::System(_) => {}
            Message::User(text) => lines.push(format!("user: {text}")),
            Message::Assistant(reply) => {
                if let Some(text) = reply.content.as_deref().filter(|text| !text.is_empty()) {
                    lines.push(format!("assistant: {text}"));
                }
                for call in &reply.tool_calls {
                    lines.push(format!("call {}: {}", call.name, call.arguments));
                }
            }
            Message::Tool { content, .. } => lines.push(format!("result: {content}")),
        }
    }
    lines.join("\n")
}

/// The messages of the summary request: `task` is the user prompt of the
/// agent, and `after` the messages after it.
pub fn summary_request(task: &str, after: &[Message]) -> [Message; 2] {
    [
        Message::System(SUMMARY_PROMPT.to_string()),
        Message::User(format!(
            "Task of the agent:\n{task}\n\nConversation after the task:\n{}",
            transcript(after)
        )),
    ]
}

/// The conversation after compaction: the system prompt and the task of
/// `conversation`, and one user message with `summary`.
pub fn compacted(conversation: &[Message], summary: &str) -> Vec<Message> {
    let mut messages = conversation[..2].to_vec();
    messages.push(Message::User(format!(
        "{SUMMARY_HEADER}\n{summary}\n\nGo on with the task."
    )));
    messages
}
