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

use serde_json::Value;

use crate::actor::AgentType;

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
/// option. The other params, joined with spaces, are the task. If the params
/// do not set `model` or `base_url`, the environment variables [`MODEL_ENV`]
/// and [`BASE_URL_ENV`] set them. The API key comes only from
/// [`API_KEY_ENV`], so it is never in the job file or the log.
pub struct AgentConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub max_steps: u32,
    pub task: String,
}

impl AgentConfig {
    /// Reads the configuration. `env` returns the value of an environment
    /// variable, or `None` if it is not set.
    pub fn from_params(
        params: &[String],
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        let mut base_url = None;
        let mut model = None;
        let mut max_steps = None;
        let mut task = Vec::new();
        for param in params {
            // A word without `=`, such as `model`, is part of the task.
            let (key, value) = param.split_once('=').unwrap_or_default();
            let option = match key {
                "base_url" => &mut base_url,
                "model" => &mut model,
                "max_steps" => &mut max_steps,
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
            task: task.join(" "),
        })
    }
}

/// Runs one agent of a given type until it finishes its work.
pub struct AgentToolLoop {
    agent: AgentType,
    tools: Vec<Box<dyn Tool>>,
    max_steps: u32,
}

impl AgentToolLoop {
    /// Makes a loop with the built-in tools for the agent type (see
    /// [`tools::default_tools`]).
    pub fn new(agent: AgentType) -> Self {
        AgentToolLoop {
            agent,
            tools: tools::default_tools(agent),
            max_steps: DEFAULT_MAX_STEPS,
        }
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

    /// The type of agent that this loop runs.
    pub fn agent(&self) -> AgentType {
        self.agent
    }

    /// Names of the tools that the model can call.
    pub fn tool_names(&self) -> Vec<&str> {
        self.tools.iter().map(|tool| tool.name()).collect()
    }

    /// Runs the agent with `params` from the action statement and `input`,
    /// the output of the previous action or task. Reads the configuration
    /// with [`AgentConfig::from_params`], talks to the model with an
    /// [`openai::OpenAiClient`], and returns the model's final reply.
    pub fn run_loop(&mut self, params: &[String], input: &str) -> Result<String, String> {
        let config = AgentConfig::from_params(params, |name| std::env::var(name).ok())?;
        self.max_steps = config.max_steps;
        let mut client =
            openai::OpenAiClient::new(&config.base_url, &config.model, config.api_key.clone());
        self.run_with(&mut client, &config.task, input)
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

        for _ in 0..self.max_steps {
            let reply = client.chat(&messages, &self.tools)?;
            if reply.tool_calls.is_empty() {
                return Ok(reply.content.unwrap_or_default());
            }
            let calls = reply.tool_calls.clone();
            messages.push(Message::Assistant(reply));
            for call in calls {
                let content = self
                    .call_tool(&call)
                    .unwrap_or_else(|err| format!("error: {err}"));
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
            "You are a web research agent. Use the tools to find and read web pages \
             for the task. When you are done, reply with the answer only, without \
             tool calls."
        }
        AgentType::Coder => {
            "You are a coding agent that works in the current directory. Use the \
             tools to read, change and test code for the task. When you are done, \
             reply with a short summary of the changes, without tool calls."
        }
    }
}
