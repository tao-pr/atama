//! A [`ChatClient`] for any server with an OpenAI compatible
//! `/chat/completions` endpoint, for example OpenAI, Ollama, vLLM or
//! LM Studio.

use serde_json::{Value, json};
use ureq::Agent;

use super::{ChatClient, Message, Reply, Tool, ToolCall};

/// Sends each step of the conversation to `<base_url>/chat/completions`.
pub struct OpenAiClient {
    url: String,
    model: String,
    api_key: Option<String>,
    agent: Agent,
}

impl OpenAiClient {
    /// Makes a client for `model` at `base_url`, for example
    /// `http://localhost:11434/v1`. If `api_key` is set, each request sends
    /// it as a bearer token.
    pub fn new(base_url: &str, model: &str, api_key: Option<String>) -> Self {
        OpenAiClient {
            url: format!("{}/chat/completions", base_url.trim_end_matches('/')),
            model: model.to_string(),
            api_key,
            agent: Agent::new_with_config(
                Agent::config_builder().http_status_as_error(false).build(),
            ),
        }
    }
}

impl ChatClient for OpenAiClient {
    fn chat(&mut self, messages: &[Message], tools: &[Box<dyn Tool>]) -> Result<Reply, String> {
        let body = request_body(&self.model, messages, tools);
        let mut request = self.agent.post(&self.url);
        if let Some(key) = &self.api_key {
            request = request.header("Authorization", format!("Bearer {key}"));
        }
        let url = &self.url;
        let mut response = request
            .content_type("application/json")
            .send(body.to_string())
            .map_err(|err| format!("request to `{url}` failed: {err}"))?;
        let status = response.status();
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|err| format!("cannot read response from `{url}`: {err}"))?;
        if !status.is_success() {
            return Err(format!("`{url}` responded with {status}: {}", text.trim()));
        }
        let json: Value = serde_json::from_str(&text)
            .map_err(|err| format!("response from `{url}` is not JSON: {err}"))?;
        parse_reply(&json).ok_or_else(|| format!("response from `{url}` has no message"))
    }
}

/// The JSON body of a chat completions request.
pub fn request_body(model: &str, messages: &[Message], tools: &[Box<dyn Tool>]) -> Value {
    let mut body = json!({
        "model": model,
        "messages": messages.iter().map(message_json).collect::<Vec<_>>(),
    });
    if !tools.is_empty() {
        body["tools"] = tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name(),
                        "description": tool.description(),
                        "parameters": tool.parameters(),
                    },
                })
            })
            .collect();
    }
    body
}

fn message_json(message: &Message) -> Value {
    match message {
        Message::System(content) => json!({"role": "system", "content": content}),
        Message::User(content) => json!({"role": "user", "content": content}),
        Message::Assistant(reply) => {
            let mut json = json!({"role": "assistant", "content": reply.content});
            if !reply.tool_calls.is_empty() {
                json["tool_calls"] = reply
                    .tool_calls
                    .iter()
                    .map(|call| {
                        json!({
                            "id": call.id,
                            "type": "function",
                            "function": {"name": call.name, "arguments": call.arguments},
                        })
                    })
                    .collect();
            }
            json
        }
        Message::Tool { call_id, content } => {
            json!({"role": "tool", "tool_call_id": call_id, "content": content})
        }
    }
}

/// Reads the reply from `choices[0].message` of a chat completions response.
/// Returns `None` if the response has no message.
pub fn parse_reply(json: &Value) -> Option<Reply> {
    let message = json.get("choices")?.get(0)?.get("message")?;
    let tool_calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .map(|call| ToolCall {
                    id: text(call, &["id"]),
                    name: text(call, &["function", "name"]),
                    arguments: text(call, &["function", "arguments"]),
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Reply {
        content: message
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_string),
        tool_calls,
    })
}

/// The string at `path` in `json`, or an empty string.
fn text(json: &Value, path: &[&str]) -> String {
    path.iter()
        .try_fold(json, |json, key| json.get(key))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}
