//! The OpenAI compatible agent loop that an
//! [`Actor::AgentTool`](crate::Actor::AgentTool) action runs.
//!
//! The loop does not depend on a model. It talks to the model through a
//! [`ChatClient`], and [`openai::OpenAiClient`] works with any server that
//! has an OpenAI compatible `/chat/completions` endpoint. Each step sends the
//! conversation to the model. If the model asks for tool calls, the loop runs
//! them and sends the results back. If the model replies without tool calls,
//! that reply is the output of the loop.
//!
//! # Compatible servers
//!
//! Compatibility is a property of the server, not of the model. Any model on
//! one of these servers works with the loop if the model supports tool calls.
//! In a model profile of the job (see [`crate::profile`]), set `base_url=`
//! to the base URL, and `model=` to a model identifier of that server. These
//! URLs are examples
//! from the providers' documentation. Make sure that they are current before
//! you use them.
//!
//! | Server | Base URL | Notes |
//! | --- | --- | --- |
//! | OpenAI | `https://api.openai.com/v1` | The default base URL. |
//! | Anthropic (Claude) | `https://api.anthropic.com/v1` | Does not return thinking and has no prompt caching. Anthropic calls it a layer to test and compare models, not for production. |
//! | Google Gemini | `https://generativelanguage.googleapis.com/v1beta/openai` | |
//! | Mistral | `https://api.mistral.ai/v1` | |
//! | xAI (Grok) | `https://api.x.ai/v1` | |
//! | DeepSeek | `https://api.deepseek.com/v1` | |
//! | Groq | `https://api.groq.com/openai/v1` | Hosts open-weight models. |
//! | Together AI | `https://api.together.xyz/v1` | Hosts open-weight models. |
//! | OpenRouter | `https://openrouter.ai/api/v1` | Routes to models of many providers. |
//! | Ollama | `http://localhost:11434/v1` | Local. Needs no API key. |
//! | LM Studio | `http://localhost:1234/v1` | Local. Needs no API key. |
//! | vLLM | `http://localhost:8000/v1` | Local. The key is optional. |
//! | llama.cpp (`llama-server`) | `http://localhost:8080/v1` | Local. Start it with `--jinja` to turn on tool calls. |
//!
//! A local port is the server's default port. If you start the server on a
//! different port, change the URL.

use std::io::Write;

use serde_json::Value;

use crate::actor::AgentType;
use crate::profile::ModelProfile;
use crate::schema::{self, OutputSchema};

pub mod agent_options;
pub mod openai;
pub mod tools;

use agent_options::{AgentConfig, DEFAULT_MAX_STEPS};

/// One message of the conversation with the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Instructions for the model.
    System(String),
    /// The task.
    User(String),
    /// A reply from the model.
    Assistant(Reply),
    /// The result of a tool call.
    Tool {
        /// The `id` of the [`ToolCall`] that this result answers.
        call_id: String,
        content: String,
    },
}

/// A reply from the model: text, tool calls, or both.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reply {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

/// A request from the model to run a tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    /// Name of the tool.
    pub name: String,
    /// Arguments as a JSON object, in text form.
    pub arguments: String,
}

/// A tool that the model can call.
pub trait Tool {
    /// The name that the model uses to call the tool.
    fn name(&self) -> &str;
    /// Tells the model what the tool does.
    fn description(&self) -> &str;
    /// JSON schema of the arguments.
    fn parameters(&self) -> Value;
    /// Runs the tool and returns its result for the model.
    fn call(&mut self, arguments: &Value) -> Result<String, String>;
}

/// A chat model that can call tools.
pub trait ChatClient {
    /// Sends the conversation and the available tools, and returns the
    /// model's next reply.
    fn chat(&mut self, messages: &[Message], tools: &[Box<dyn Tool>]) -> Result<Reply, String>;
}

/// Runs one agent of a given type until it finishes its work.
pub struct AgentToolLoop<'a> {
    agent: AgentType,
    tools: Vec<Box<dyn Tool>>,
    max_steps: u32,
    /// The `reasoning_effort` that the client sends, for debug output only.
    reasoning_effort: Option<&'static str>,
    /// The schema block of the job that `output_schema=<name>` names.
    schema_block: Option<OutputSchema>,
    /// The schema that the final reply must match, if set.
    output_schema: Option<OutputSchema>,
    /// Gets the debug output, if set.
    debug: Option<Box<dyn Write + 'a>>,
    /// The name of the model profile of the action, if known. The debug
    /// headers and the errors of the loop name it (see
    /// [`AgentToolLoop::label`]).
    profile: Option<String>,
}

impl<'a> AgentToolLoop<'a> {
    /// Makes a loop with the built-in tools for the agent type (see
    /// [`tools::default_tools`]).
    pub fn new(agent: AgentType) -> Self {
        AgentToolLoop {
            agent,
            tools: tools::default_tools(agent),
            max_steps: DEFAULT_MAX_STEPS,
            reasoning_effort: None,
            schema_block: None,
            output_schema: None,
            debug: None,
            profile: None,
        }
    }

    /// Writes debug output to `out`: the configuration, and for each step the
    /// new messages of the request with its `reasoning_effort` if the request
    /// has one, the model's reply, and each tool call
    /// with its arguments and result.
    /// Messages are pretty JSON in the chat completions format. The API key
    /// is never written.
    pub fn debug(mut self, out: impl Write + 'a) -> Self {
        self.debug = Some(Box::new(out));
        self
    }

    /// Replaces the tools that the model can call.
    pub fn with_tools(mut self, tools: Vec<Box<dyn Tool>>) -> Self {
        self.tools = tools;
        self
    }

    /// Sets the schema block of the job that the `output_schema=<name>` of
    /// the action names. [`AgentToolLoop::run_loop`] uses it if the value of
    /// `output_schema=` is that name (see [`schema::load`]).
    pub fn schema_block(mut self, block: Option<OutputSchema>) -> Self {
        self.schema_block = block;
        self
    }

    /// Sets the schema that the final reply of [`AgentToolLoop::run_with`]
    /// must match. [`AgentToolLoop::run_loop`] sets it from
    /// `output_schema=`.
    pub fn output_schema(mut self, output_schema: Option<OutputSchema>) -> Self {
        self.output_schema = output_schema;
        self
    }

    /// Sets the most model replies that one run asks for.
    pub fn max_steps(mut self, max_steps: u32) -> Self {
        self.max_steps = max_steps;
        self
    }

    /// The type of agent that this loop runs.
    pub fn agent(&self) -> AgentType {
        self.agent
    }

    /// Names the agent in debug headers and errors: `agent:<type>`, and the
    /// name of the model profile in parentheses if it is known, as in
    /// `agent:coder (local-qwen3)`. Then, if several agent actions write to
    /// the same output, each block names its own action.
    pub fn label(&self) -> String {
        match &self.profile {
            Some(profile) => format!("agent:{} ({profile})", self.agent),
            None => format!("agent:{}", self.agent),
        }
    }

    /// Names of the tools that the model can call.
    pub fn tool_names(&self) -> Vec<&str> {
        self.tools.iter().map(|tool| tool.name()).collect()
    }

    /// Runs the agent with `params` from the action statement, the model
    /// `profile` that its `model=` names, and `input`, the output of the
    /// previous action or task, with the environment of the process (see
    /// [`AgentToolLoop::run_loop_with_env`]).
    pub fn run_loop(
        &mut self,
        params: &[String],
        profile: Option<&ModelProfile>,
        input: &str,
    ) -> Result<String, String> {
        self.run_loop_with_env(params, profile, input, &|name| std::env::var(name).ok())
    }

    /// Runs the agent like [`AgentToolLoop::run_loop`]. `env` returns the
    /// value of an environment variable, or `None` if it is not set.
    ///
    /// Reads the configuration with [`AgentConfig::from_params`], talks to
    /// the model with an [`openai::OpenAiClient`], and returns the model's
    /// final reply. A `summarizer` agent adds its expertise and length limit
    /// to the task (see [`agent_options::SummarizerOptions::task`]). With
    /// `output_schema=`, the reply must match the schema (see
    /// [`schema::load`]).
    ///
    /// Fails if the configuration is invalid, the output schema cannot be
    /// loaded, or a `summarizer` agent has no input.
    pub fn run_loop_with_env(
        &mut self,
        params: &[String],
        profile: Option<&ModelProfile>,
        input: &str,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<String, String> {
        let config = AgentConfig::from_params(self.agent, params, profile, env)?;
        self.profile = Some(config.profile.clone());
        self.max_steps = config.max_steps;
        self.reasoning_effort = openai::reasoning_effort(config.thinking);
        self.output_schema = config
            .output_schema
            .as_deref()
            .map(|value| schema::load(value, self.schema_block.as_ref()))
            .transpose()?;
        let summary = config.summary();
        self.write_debug(format!(
            "=== {} configuration ===\n{summary}",
            self.label()
        ));
        let task = match &config.summarizer {
            Some(options) => options.task(&config.task, input)?,
            None => config.task.clone(),
        };
        let mut client =
            openai::OpenAiClient::new(&config.base_url, &config.model, config.api_key.clone())
                .with_thinking(config.thinking)
                .with_output_schema(self.output_schema.clone());
        self.run_with(&mut client, &task, input)
    }

    /// Runs the loop with `client`.
    ///
    /// The model gets a system prompt for the agent type, then the task and
    /// `input` as one user message. Each step asks the model for a reply. If
    /// the reply has tool calls, the loop runs them in order and adds their
    /// results to the conversation. A failed tool call does not stop the
    /// loop: the model gets the error and can try again. The text of the
    /// first reply without tool calls is the output.
    ///
    /// With an output schema (see [`AgentToolLoop::output_schema`]), the
    /// system prompt tells the model the schema, and the output is the JSON
    /// of the first reply without tool calls that matches it (see
    /// [`OutputSchema::check_reply`]). If such a reply does not match, the
    /// model gets the errors and can try again.
    ///
    /// Fails if the task and the input are both empty, the client fails, or
    /// the model has no final reply, or no reply that matches the schema,
    /// after `max_steps` replies.
    pub fn run_with(
        &mut self,
        client: &mut dyn ChatClient,
        task: &str,
        input: &str,
    ) -> Result<String, String> {
        let prompt = user_prompt(task, input)
            .ok_or_else(|| format!("`{}` needs `task=` or an input", self.label()))?;
        let mut system = system_prompt(self.agent).to_string();
        if let Some(schema) = &self.output_schema {
            system = format!("{system}\n\n{}", schema.instruction());
        }
        let mut messages = vec![Message::System(system), Message::User(prompt)];
        // The error of the last final reply that did not match the schema.
        let mut schema_error: Option<String> = None;

        // Number of messages that earlier requests already sent.
        let mut sent = 0;
        for step in 1..=self.max_steps {
            if self.debug.is_some() {
                let tools = self.tool_names().join(", ");
                let new = pretty(&messages[sent..]);
                let effort = self.reasoning_effort.map_or(String::new(), |effort| {
                    format!(", reasoning_effort: {effort}")
                });
                let format = self.output_schema.as_ref().map_or(String::new(), |schema| {
                    format!(", output_schema: {}", schema.name)
                });
                self.write_debug(format!(
                    "=== {} step {step}: request ({} messages, {} new, tools: {tools}{effort}{format}) ===\n{new}",
                    self.label(),
                    messages.len(),
                    messages.len() - sent,
                ));
            }
            sent = messages.len();
            let reply = client.chat(&messages, &self.tools)?;
            if self.debug.is_some() {
                let json = pretty(std::slice::from_ref(&Message::Assistant(reply.clone())));
                self.write_debug(format!(
                    "=== {} step {step}: response ({} tool calls) ===\n{json}",
                    self.label(),
                    reply.tool_calls.len()
                ));
            }
            if reply.tool_calls.is_empty() {
                let text = reply.content.clone().unwrap_or_default();
                let Some(schema) = &self.output_schema else {
                    return Ok(text);
                };
                let error = match schema.check_reply(&text) {
                    Ok(json) => return Ok(json),
                    Err(error) => error,
                };
                self.write_debug(format!(
                    "=== {} step {step}: reply does not match output schema `{}` ===\n{error}",
                    self.label(), schema.name
                ));
                messages.push(Message::Assistant(reply));
                messages.push(Message::User(format!(
                    "Your reply is not valid: {error}\nReply again with only the JSON value."
                )));
                schema_error = Some(error);
                continue;
            }
            let calls = reply.tool_calls.clone();
            messages.push(Message::Assistant(reply));
            let count = calls.len();
            for (index, call) in calls.into_iter().enumerate() {
                let result = self.call_tool(&call);
                if self.debug.is_some() {
                    let outcome = match &result {
                        Ok(content) => format!("ok, {} characters", content.chars().count()),
                        Err(err) => format!("error: {err}"),
                    };
                    self.write_debug(format!(
                        "=== {} step {step}: tool call {} of {count}: {} (id {}) ===\narguments:\n{}\nresult: {outcome}",
                        self.label(),
                        index + 1,
                        call.name,
                        call.id,
                        pretty_json(&call.arguments),
                    ));
                }
                let content = result.unwrap_or_else(|err| format!("error: {err}"));
                messages.push(Message::Tool {
                    call_id: call.id,
                    content,
                });
            }
        }
        if let (Some(error), Some(schema)) = (schema_error, &self.output_schema) {
            return Err(format!(
                "`{}` did not reply with JSON that matches output schema `{}` in {} steps, \
                 the last error: {error}",
                self.label(), schema.name, self.max_steps
            ));
        }
        Err(format!(
            "`{}` did not finish in {} steps",
            self.label(), self.max_steps
        ))
    }

    /// Writes one block of debug output, if debug output is on. A failed
    /// write does not stop the agent.
    fn write_debug(&mut self, text: String) {
        if let Some(out) = &mut self.debug {
            let _ = writeln!(out, "{text}");
        }
    }

    fn call_tool(&mut self, call: &ToolCall) -> Result<String, String> {
        let tool = self
            .tools
            .iter_mut()
            .find(|tool| tool.name() == call.name)
            .ok_or_else(|| format!("unknown tool `{}`", call.name))?;
        // Some models send an empty string for a call without arguments.
        let arguments = if call.arguments.trim().is_empty() {
            Value::Object(Default::default())
        } else {
            serde_json::from_str(&call.arguments)
                .map_err(|err| format!("arguments of `{}` are not JSON: {err}", call.name))?
        };
        tool.call(&arguments)
    }
}

/// The messages as a pretty JSON array in the chat completions format.
fn pretty(messages: &[Message]) -> String {
    let json: Vec<Value> = messages.iter().map(openai::message_json).collect();
    serde_json::to_string_pretty(&json).expect("JSON values always serialize")
}

/// JSON text, such as tool call arguments, as pretty JSON. Text that is not
/// JSON stays as it is.
fn pretty_json(text: &str) -> String {
    match serde_json::from_str::<Value>(text) {
        Ok(json) => serde_json::to_string_pretty(&json).expect("JSON values always serialize"),
        Err(_) => text.to_string(),
    }
}

/// The task, followed by the input, or `None` if both are empty.
fn user_prompt(task: &str, input: &str) -> Option<String> {
    let input = input.trim();
    match (task.is_empty(), input.is_empty()) {
        (true, true) => None,
        (false, true) => Some(task.to_string()),
        (true, false) => Some(input.to_string()),
        (false, false) => Some(format!("{task}\n\nInput:\n{input}")),
    }
}

fn system_prompt(agent: AgentType) -> &'static str {
    match agent {
        AgentType::Coder => {
            "You are a coding agent that works in the current directory. Use the \
             tools to read, change and test code for the task. When you are done, \
             reply with a short summary of the changes, without tool calls."
        }
        AgentType::Summarizer => {
            "You are a summarizing agent. The user message has an instruction, an \
             expertise, a length limit for the summary, and the input text. Analyse the \
             input and find its key information. Then rewrite the input as a summary \
             that keeps all its key points. Keep the summary informative, concise and \
             to the point. If the input names numbers, quantities, dates or times, \
             names or concepts that are important to its key matter, keep them in the \
             summary. Do not add facts that are not in the input. Use the terms and the \
             point of view of the expertise. Stay within the length limit. Reply with \
             the summary only."
        }
    }
}
