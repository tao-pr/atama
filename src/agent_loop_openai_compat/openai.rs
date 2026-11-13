//! A [`ChatClient`] for any server with an OpenAI compatible
//! `/chat/completions` endpoint, for example OpenAI, Ollama, vLLM or
//! LM Studio.

use serde_json::{Value, json};
use ureq::Agent;

use super::{ChatClient, Message, Reply, Tool, ToolCall};
use crate::http;
use crate::schema::OutputSchema;

/// Sends each step of the conversation to `<base_url>/chat/completions`.
pub struct OpenAiClient {
    url: String,
    model: String,
    api_key: Option<String>,
    thinking: Option<bool>,
    output_schema: Option<OutputSchema>,
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
            thinking: None,
            output_schema: None,
            agent: http::agent(),
        }
    }

    /// Sets the `thinking=` option of the action (see [`request_body`]).
    pub fn with_thinking(mut self, thinking: Option<bool>) -> Self {
        self.thinking = thinking;
        self
    }

    /// Sets the schema of the final reply (see [`request_body`]).
    pub fn with_output_schema(mut self, output_schema: Option<OutputSchema>) -> Self {
        self.output_schema = output_schema;
        self
    }
}

impl ChatClient for OpenAiClient {
    fn chat(&mut self, messages: &[Message], tools: &[Box<dyn Tool>]) -> Result<Reply, String> {
        let body = request_body(
            &self.model,
            messages,
            tools,
            self.thinking,
            self.output_schema.as_ref(),
        );
        let mut request = self.agent.post(&self.url);
        if let Some(key) = &self.api_key {
            request = request.header("Authorization", format!("Bearer {key}"));
        }
        let url = &self.url;
        let response = request
            .content_type("application/json")
            .send(body.to_string());
        let text = http::read_response(url, response)?;
        let json: Value = serde_json::from_str(&text)
            .map_err(|err| format!("response from `{url}` is not JSON: {err}"))?;
        parse_reply(&json).ok_or_else(|| format!("response from `{url}` has no message"))
    }
}

/// The JSON body of a chat completions request.
///
/// If `thinking` is `Some(false)`, the body has `"reasoning_effort": "none"`,
/// which turns off the thinking of the model on Ollama and on other servers
/// that know the value. Else the body has no `reasoning_effort`, so the
/// server uses its default.
///
/// If `output_schema` is set, the body has its `response_format` (see
/// [`OutputSchema::response_format`]), which asks the server for a reply
/// that matches the schema.
///
/// The `tools` field has only the available tools (see
/// [`Tool::available`]). Without available tools, the body has no `tools`.
pub fn request_body(
    model: &str,
    messages: &[Message],
    tools: &[Box<dyn Tool>],
    thinking: Option<bool>,
    output_schema: Option<&OutputSchema>,
) -> Value {
    let mut body = json!({
        "model": model,
        "messages": messages.iter().map(message_json).collect::<Vec<_>>(),
    });
    let tools: Vec<_> = tools.iter().filter(|tool| tool.available()).collect();
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
    if let Some(effort) = reasoning_effort(thinking) {
        body["reasoning_effort"] = json!(effort);
    }
    if let Some(schema) = output_schema {
        body["response_format"] = schema.response_format();
    }
    body
}

/// The `reasoning_effort` of a request for the `thinking=` option: `none`
/// for `Some(false)`, else `None`, so that the body has no such field.
pub fn reasoning_effort(thinking: Option<bool>) -> Option<&'static str> {
    (thinking == Some(false)).then_some("none")
}

/// A message in the JSON format of the chat completions API.
pub fn message_json(message: &Message) -> Value {
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

/// Reads the reply from `choices[0].message` of a chat completions response,
/// and its tokens from `usage.total_tokens`. Returns `None` if the response
/// has no message.
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
        tokens: json
            .get("usage")
            .and_then(|usage| usage.get("total_tokens"))
            .and_then(Value::as_u64),
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
