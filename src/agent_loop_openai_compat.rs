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
//! Set the `base_url=` param (or `ATAMA_AGENT_BASE_URL`) to the base URL, and
//! `model=` to a model identifier of that server. These URLs are examples
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
use crate::tavily::{self, SearchConfig};

pub mod openai;
pub mod tools;

/// Base URL of the chat API if the action and the environment set none.
pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
/// Most model replies that one run of the loop asks for.
pub const DEFAULT_MAX_STEPS: u32 = 10;
/// Environment variable with the base URL of the chat API.
pub const BASE_URL_ENV: &str = "ATAMA_AGENT_BASE_URL";
/// Environment variable with the model identifier.
pub const MODEL_ENV: &str = "ATAMA_AGENT_MODEL";
/// Environment variable with the API key. Local servers often need none.
pub const API_KEY_ENV: &str = "ATAMA_AGENT_API_KEY";
/// Search results that a `websearch` agent analyses if the action sets no
/// `max_results`.
pub const DEFAULT_MAX_RESULTS: u32 = 5;
/// Most search results that a `websearch` agent can ask for. Tavily returns
/// at most 20.
pub const MAX_SEARCH_RESULTS: u32 = 20;
/// Instruction of a `websearch` agent if the action has no prompt words.
pub const DEFAULT_WEBSEARCH_PROMPT: &str = "Summarize the main facts in the search results \
     about the topic. Name the URL of the source of each fact.";

/// Options of an agent action. Only these options can have a quoted value.
const OPTION_KEYS: [&str; 5] = ["base_url", "model", "max_steps", "topic", "max_results"];

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

/// The configuration of one run, read from the action's params and the
/// environment.
///
/// A param `model=<id>`, `base_url=<url>` or `max_steps=<n>` sets that
/// option for every agent type. A `websearch` agent also takes
/// `topic=<text>` and `max_results=<n>` (see [`WebSearchOptions`]). A value
/// can be in double quotes, so that it can have spaces, as in
/// `topic="latest Rust release"`. The other params, joined with spaces, are
/// the task. If the params do not set `model` or `base_url`, the environment
/// variables [`MODEL_ENV`] and [`BASE_URL_ENV`] set them. The API key comes
/// only from [`API_KEY_ENV`], so it is never in the job file or the log.
pub struct AgentConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub max_steps: u32,
    /// The search options of a `websearch` agent. `None` for other agent
    /// types.
    pub websearch: Option<WebSearchOptions>,
    /// The task. For a `websearch` agent, it is the instruction that tells
    /// the model how to analyse the search results.
    pub task: String,
}

/// The options of a `websearch` agent: what it searches for, and how many
/// results it analyses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSearchOptions {
    /// What to search for, from `topic=<text>`.
    pub topic: String,
    /// Most search results to analyse, from `max_results=<n>`. The default
    /// is [`DEFAULT_MAX_RESULTS`].
    pub max_results: u32,
}

impl WebSearchOptions {
    /// Reads the options from the values of `topic=` and `max_results=`, if
    /// the params set them.
    ///
    /// Fails if `max_results` is not a number from 1 to
    /// [`MAX_SEARCH_RESULTS`], or there is no topic.
    fn from_values(topic: Option<String>, max_results: Option<String>) -> Result<Self, String> {
        let max_results = match max_results {
            None => DEFAULT_MAX_RESULTS,
            Some(value) => value
                .parse()
                .ok()
                .filter(|n| (1..=MAX_SEARCH_RESULTS).contains(n))
                .ok_or_else(|| {
                    format!("`max_results={value}` is not a number from 1 to {MAX_SEARCH_RESULTS}")
                })?,
        };
        let topic = topic
            .ok_or("`agent:websearch` needs a topic, as in `topic=\"latest Rust release\"`")?;
        Ok(WebSearchOptions { topic, max_results })
    }
}

impl AgentConfig {
    /// Reads the configuration of an `agent` action. `env` returns the value
    /// of an environment variable, or `None` if it is not set.
    ///
    /// Fails if an option is empty, set twice or invalid, there is no model,
    /// a `websearch` agent has no topic, or another agent type gets
    /// `topic=` or `max_results=`.
    pub fn from_params(
        agent: AgentType,
        params: &[String],
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        let mut base_url = None;
        let mut model = None;
        let mut max_steps = None;
        let mut topic = None;
        let mut max_results = None;
        let mut task = Vec::new();
        let params = join_quoted(params)?;
        for param in &params {
            // A word without `=`, such as `model`, is part of the task.
            let (key, value) = param.split_once('=').unwrap_or_default();
            let option = match key {
                "base_url" => &mut base_url,
                "model" => &mut model,
                "max_steps" => &mut max_steps,
                "topic" => &mut topic,
                "max_results" => &mut max_results,
                _ => {
                    task.push(param.as_str());
                    continue;
                }
            };
            if value.is_empty() {
                return Err(format!("option `{key}` has no value"));
            }
            if option.replace(value.to_string()).is_some() {
                return Err(format!("option `{key}` is set twice"));
            }
        }

        let websearch = match agent {
            AgentType::WebSearch => Some(WebSearchOptions::from_values(topic, max_results)?),
            AgentType::Coder => {
                // The search options only control the search of a `websearch`
                // agent, so another agent fails on them instead of ignoring them.
                for (key, value) in [("topic", &topic), ("max_results", &max_results)] {
                    if value.is_some() {
                        return Err(format!("option `{key}` works only with `agent:websearch`"));
                    }
                }
                None
            }
        };
        let max_steps = match max_steps {
            None => DEFAULT_MAX_STEPS,
            Some(value) => value
                .parse()
                .ok()
                .filter(|steps| *steps > 0)
                .ok_or_else(|| format!("`max_steps={value}` is not a positive number"))?,
        };
        let model = model
            .or_else(|| env(MODEL_ENV))
            .filter(|model| !model.is_empty())
            .ok_or_else(|| format!("no model: add `model=<id>` or set `{MODEL_ENV}`"))?;
        Ok(AgentConfig {
            base_url: base_url
                .or_else(|| env(BASE_URL_ENV))
                .filter(|url| !url.is_empty())
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            model,
            api_key: env(API_KEY_ENV).filter(|key| !key.is_empty()),
            max_steps,
            websearch,
            task: task.join(" "),
        })
    }
}

impl AgentConfig {
    /// The configuration for debug output, one `name: value` line each. It
    /// shows only whether the API key is set, never the key itself. For a
    /// `websearch` agent, it also shows `topic` and `max_results`.
    pub fn summary(&self) -> String {
        let key = if self.api_key.is_some() {
            "set"
        } else {
            "not set"
        };
        let mut summary = format!(
            "model: {}\nbase_url: {}\nmax_steps: {}\napi key: {key}",
            self.model, self.base_url, self.max_steps
        );
        if let Some(options) = &self.websearch {
            summary.push_str(&format!(
                "\ntopic: {}\nmax_results: {}",
                options.topic, options.max_results
            ));
        }
        summary
    }
}

/// Runs one agent of a given type until it finishes its work.
pub struct AgentToolLoop<'a> {
    agent: AgentType,
    tools: Vec<Box<dyn Tool>>,
    max_steps: u32,
    /// Base URL of the Tavily API for the search of a `websearch` agent.
    search_base_url: String,
    /// Gets the debug output, if set.
    debug: Option<Box<dyn Write + 'a>>,
}

impl<'a> AgentToolLoop<'a> {
    /// Makes a loop with the built-in tools for the agent type (see
    /// [`tools::default_tools`]).
    pub fn new(agent: AgentType) -> Self {
        AgentToolLoop {
            agent,
            tools: tools::default_tools(agent),
            max_steps: DEFAULT_MAX_STEPS,
            search_base_url: tavily::DEFAULT_BASE_URL.to_string(),
            debug: None,
        }
    }

    /// Writes debug output to `out`: the configuration, for a `websearch`
    /// agent the Tavily response body as pretty JSON and the results that
    /// the model gets, and for each step the new messages of the request,
    /// the model's reply, and each tool call with its arguments and result.
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

    /// Sets the most model replies that one run asks for.
    pub fn max_steps(mut self, max_steps: u32) -> Self {
        self.max_steps = max_steps;
        self
    }

    /// Sets the base URL of the Tavily API for the search of a `websearch`
    /// agent. The default is [`tavily::DEFAULT_BASE_URL`].
    pub fn search_base_url(mut self, url: &str) -> Self {
        self.search_base_url = url.to_string();
        self
    }

    /// The type of agent that this loop runs.
    pub fn agent(&self) -> AgentType {
        self.agent
    }

    /// Names of the tools that the model can call.
    pub fn tool_names(&self) -> Vec<&str> {
        self.tools.iter().map(|tool| tool.name()).collect()
    }

    /// Runs the agent with `params` from the action statement and `input`,
    /// the output of the previous action or task, with the environment of
    /// the process (see [`AgentToolLoop::run_loop_with_env`]).
    pub fn run_loop(&mut self, params: &[String], input: &str) -> Result<String, String> {
        self.run_loop_with_env(params, input, &|name| std::env::var(name).ok())
    }

    /// Runs the agent like [`AgentToolLoop::run_loop`]. `env` returns the
    /// value of an environment variable, or `None` if it is not set.
    ///
    /// Reads the configuration with [`AgentConfig::from_params`], talks to
    /// the model with an [`openai::OpenAiClient`], and returns the model's
    /// final reply. A `websearch` agent first searches its topic with Tavily
    /// (see [`AgentToolLoop::search_task`]).
    ///
    /// Fails if the configuration is invalid, or the search of a
    /// `websearch` agent fails.
    pub fn run_loop_with_env(
        &mut self,
        params: &[String],
        input: &str,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<String, String> {
        let config = AgentConfig::from_params(self.agent, params, env)?;
        self.max_steps = config.max_steps;
        let summary = config.summary();
        self.write_debug(format!(
            "=== agent:{} configuration ===\n{summary}",
            self.agent
        ));
        let task = match &config.websearch {
            Some(options) => self.search_task(options, &config.task, env)?,
            None => config.task.clone(),
        };
        let mut client =
            openai::OpenAiClient::new(&config.base_url, &config.model, config.api_key.clone());
        self.run_with(&mut client, &task, input)
    }

    /// Searches the topic of a `websearch` agent with Tavily, and returns
    /// the task for the model: the instruction, the topic and the results.
    ///
    /// The search asks for `options.max_results` results. The instruction is
    /// `prompt`, or [`DEFAULT_WEBSEARCH_PROMPT`] if it is empty.
    ///
    /// Fails if `env` has no Tavily API key, or the search fails.
    pub fn search_task(
        &mut self,
        options: &WebSearchOptions,
        prompt: &str,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<String, String> {
        let WebSearchOptions { topic, max_results } = options;
        let search = SearchConfig {
            base_url: self.search_base_url.clone(),
            api_key: tavily::api_key(env)?,
            query: topic.clone(),
            max_results: Some(*max_results),
            search_depth: None,
            topic: None,
            include_answer: None,
        };
        let body = search.search()?;
        if self.debug.is_some() {
            self.write_debug(format!(
                "=== agent:{} search response ===\n{}",
                self.agent,
                pretty_json(&body)
            ));
        }
        let results = tools::truncate(tavily::format_results(&body)?);
        self.write_debug(format!(
            "=== agent:{} search: {topic} (max_results {max_results}) ===\n{results}",
            self.agent
        ));
        let prompt = if prompt.is_empty() {
            DEFAULT_WEBSEARCH_PROMPT
        } else {
            prompt
        };
        Ok(format!(
            "{prompt}\n\nTopic: {topic}\n\nSearch results:\n{results}"
        ))
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
    /// Fails if the task and the input are both empty, the client fails, or
    /// the model still calls tools after `max_steps` replies.
    pub fn run_with(
        &mut self,
        client: &mut dyn ChatClient,
        task: &str,
        input: &str,
    ) -> Result<String, String> {
        let prompt = user_prompt(task, input).ok_or_else(|| {
            format!(
                "`agent:{}` needs a task in its params or an input",
                self.agent
            )
        })?;
        let mut messages = vec![
            Message::System(system_prompt(self.agent).to_string()),
            Message::User(prompt),
        ];

        // Number of messages that earlier requests already sent.
        let mut sent = 0;
        for step in 1..=self.max_steps {
            if self.debug.is_some() {
                let tools = self.tool_names().join(", ");
                let new = pretty(&messages[sent..]);
                self.write_debug(format!(
                    "=== agent:{} step {step}: request ({} messages, {} new, tools: {tools}) ===\n{new}",
                    self.agent,
                    messages.len(),
                    messages.len() - sent,
                ));
            }
            sent = messages.len();
            let reply = client.chat(&messages, &self.tools)?;
            if self.debug.is_some() {
                let json = pretty(std::slice::from_ref(&Message::Assistant(reply.clone())));
                self.write_debug(format!(
                    "=== agent:{} step {step}: response ({} tool calls) ===\n{json}",
                    self.agent,
                    reply.tool_calls.len()
                ));
            }
            if reply.tool_calls.is_empty() {
                return Ok(reply.content.unwrap_or_default());
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
                        "=== agent:{} step {step}: tool call {} of {count}: {} (id {}) ===\narguments:\n{}\nresult: {outcome}",
                        self.agent,
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
        Err(format!(
            "`agent:{}` did not finish in {} steps",
            self.agent, self.max_steps
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
        AgentType::WebSearch => {
            "You are a web research agent. The user message has an instruction, a \
             topic, and web search results for the topic. Analyse the results as the \
             instruction says. Use the tools only if you need more facts, for example \
             to read a result in full. When you are done, reply with the answer only, \
             without tool calls."
        }
        AgentType::Coder => {
            "You are a coding agent that works in the current directory. Use the \
             tools to read, change and test code for the task. When you are done, \
             reply with a short summary of the changes, without tool calls."
        }
    }
}

/// Joins a quoted option value that the parser split at its spaces, so that
/// `topic="latest`, `Rust`, `release"` becomes `topic=latest Rust release`.
/// Only the keys in [`OPTION_KEYS`] can have a quoted value. Other params do
/// not change.
///
/// Fails if a quoted value has no closing quote.
fn join_quoted(params: &[String]) -> Result<Vec<String>, String> {
    let mut joined = Vec::new();
    let mut params = params.iter();
    while let Some(param) = params.next() {
        let quoted = param
            .split_once("=\"")
            .filter(|(key, _)| OPTION_KEYS.contains(key));
        let Some((key, rest)) = quoted else {
            joined.push(param.clone());
            continue;
        };
        let mut value = rest.to_string();
        while !value.ends_with('"') {
            let next = params
                .next()
                .ok_or_else(|| format!("option `{key}` has no closing quote"))?;
            value.push(' ');
            value.push_str(next);
        }
        value.pop();
        joined.push(format!("{key}={value}"));
    }
    Ok(joined)
}
