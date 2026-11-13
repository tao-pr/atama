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
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::actor::AgentType;
use crate::profile::ModelProfile;
use crate::schema::{self, OutputSchema};

pub mod agent_options;
pub mod budget;
pub mod checks;
pub mod openai;
pub mod tools;

use agent_options::{AgentConfig, DEFAULT_MAX_STEPS, format_duration};
use budget::{RunBudget, StopReason};
use checks::{CommandCheck, FinalCheck, Rejection, SchemaCheck};

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
    /// The tokens of the request and the reply together, from
    /// `usage.total_tokens` of the response. `None` if the server does not
    /// send them.
    pub tokens: Option<u64>,
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
    /// Whether the model can call the tool now. A request offers only the
    /// available tools. A tool at its call limit is not available (see
    /// [`tools::Limited`]).
    fn available(&self) -> bool {
        true
    }
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
    /// The most iterations of one run. 1 for `coder` and `summarizer`.
    max_iterations: u32,
    /// The most wall clock time of one run, if set.
    max_time: Option<Duration>,
    /// The start of the action, for the time limit. `None` makes
    /// [`AgentToolLoop::run_with`] start the time when it starts.
    started: Option<Instant>,
    /// Gives the current time. Tests replace it.
    clock: Box<dyn Fn() -> Instant + 'a>,
    /// The bash command of the success check, if set (see
    /// [`checks::CommandCheck`]).
    success_check: Option<String>,
    /// The guardrails for the system prompt, if set.
    guardrails: Option<String>,
    /// The most tokens that one run can use, if set.
    token_budget: Option<u64>,
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
            max_iterations: 1,
            max_time: None,
            started: None,
            clock: Box::new(Instant::now),
            success_check: None,
            guardrails: None,
            token_budget: None,
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

    /// Sets the most iterations of one run. After a failed success check,
    /// the next iteration starts.
    pub fn max_iterations(mut self, max_iterations: u32) -> Self {
        self.max_iterations = max_iterations;
        self
    }

    /// Sets the most wall clock time of one run. `None` sets no limit.
    /// The loop examines the time before each request and before each tool
    /// call.
    pub fn max_time(mut self, max_time: Option<Duration>) -> Self {
        self.max_time = max_time;
        self
    }

    /// Replaces the clock of the time limit. Tests use it so that they do
    /// not need to wait.
    pub fn clock(mut self, clock: impl Fn() -> Instant + 'a) -> Self {
        self.clock = Box::new(clock);
        self
    }

    /// Sets the bash command of the success check. The check runs on each
    /// reply without tool calls, and a failed check starts the next
    /// iteration (see [`checks::CommandCheck`]).
    pub fn success_check(mut self, command: Option<String>) -> Self {
        self.success_check = command;
        self
    }

    /// Sets the guardrails, rules in natural language that the loop puts
    /// into the system prompt.
    pub fn guardrails(mut self, guardrails: Option<String>) -> Self {
        self.guardrails = guardrails;
        self
    }

    /// Sets the most tokens that one run can use. The loop adds up the
    /// tokens of each reply (see [`Reply::tokens`]). It does not send a new
    /// request after the run used `token_budget` tokens or more, so one run
    /// can go over the budget by the tokens of its last step. `None` sets no
    /// limit. [`AgentToolLoop::run_loop`] sets it from `token_budget=`.
    pub fn token_budget(mut self, token_budget: Option<u64>) -> Self {
        self.token_budget = token_budget;
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

    /// Names of the tools that the model can call now (see
    /// [`Tool::available`]).
    pub fn tool_names(&self) -> Vec<&str> {
        self.tools
            .iter()
            .filter(|tool| tool.available())
            .map(|tool| tool.name())
            .collect()
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
        self.started = Some((self.clock)());
        let config = AgentConfig::from_params(self.agent, params, profile, env)?;
        self.profile = Some(config.profile.clone());
        if let Some(options) = &config.autonomous {
            self.max_iterations = options.max_iterations;
            self.max_time = Some(options.max_time);
            self.success_check = Some(options.success_check.clone());
            self.guardrails = options.guardrails.clone();
        }
        self.max_steps = config.max_steps;
        self.token_budget = config.token_budget;
        self.reasoning_effort = openai::reasoning_effort(config.thinking);
        self.output_schema = config
            .output_schema
            .as_deref()
            .map(|value| schema::load(value, self.schema_block.as_ref()))
            .transpose()?;
        let summary = config.summary();
        self.write_debug(format!("=== {} configuration ===\n{summary}", self.label()));
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
    /// With a token budget (see [`AgentToolLoop::token_budget`]), the loop
    /// counts the tokens of each reply, and stops before a request if the
    /// run used the budget.
    ///
    /// With a success check (see [`AgentToolLoop::success_check`]), a reply
    /// without tool calls ends an iteration. If the check fails, the model
    /// gets the reason, and the next iteration starts. The guardrails (see
    /// [`AgentToolLoop::guardrails`]) are in the system prompt.
    ///
    /// Fails if the task and the input are both empty, the client fails, or
    /// the model has no final reply, or no reply that passes the checks,
    /// after `max_steps` replies or `max_iterations` iterations. With a
    /// token budget, it also fails if a reply has no token count, or the run
    /// used the budget before the final reply. With a time limit, it fails
    /// if the run used the time before a request or a tool call.
    pub fn run_with(
        &mut self,
        client: &mut dyn ChatClient,
        task: &str,
        input: &str,
    ) -> Result<String, String> {
        let prompt = user_prompt(task, input)
            .ok_or_else(|| format!("`{}` needs `task=` or an input", self.label()))?;
        let mut system = system_prompt(self.agent).to_string();
        if let Some(command) = &self.success_check {
            system = format!(
                "{system}\n\nWhen you reply without tool calls, the success check \
                 `{command}` runs. If it exits with status 0, the task is done. Else you get \
                 its output, and you continue the work."
            );
        }
        if let Some(guardrails) = &self.guardrails {
            system = format!(
                "{system}\n\nGuardrails. You must obey these rules in all your work:\n{guardrails}"
            );
        }
        if let Some(schema) = &self.output_schema {
            system = format!("{system}\n\n{}", schema.instruction());
        }
        let mut messages = vec![Message::System(system), Message::User(prompt)];
        let mut checks: Vec<Box<dyn FinalCheck>> = Vec::new();
        if let Some(schema) = &self.output_schema {
            checks.push(Box::new(SchemaCheck::new(schema.clone())));
        }
        if let Some(command) = &self.success_check {
            checks.push(Box::new(CommandCheck::new(command.clone())));
        }
        // The index of the check that failed last, and its error.
        let mut last_failure: Option<(usize, String)> = None;
        let mut budget = RunBudget::new(self.max_iterations, self.max_steps, self.token_budget);
        if let Some(max) = self.max_time {
            let start = self.started.unwrap_or_else(|| (self.clock)());
            budget = budget.with_time_limit(start, max);
        }

        // Number of messages that earlier requests already sent.
        let mut sent = 0;
        loop {
            let now = (self.clock)();
            let step = match budget.check_time(now).and_then(|()| budget.start_step()) {
                Ok(step) => step,
                Err(reason) => return Err(self.stop_error(&reason, &checks, last_failure)),
            };
            let at = self.position(&budget, step);
            if self.debug.is_some() {
                let tools = self.tool_names().join(", ");
                let new = pretty(&messages[sent..]);
                let effort = self.reasoning_effort.map_or(String::new(), |effort| {
                    format!(", reasoning_effort: {effort}")
                });
                let format = self.output_schema.as_ref().map_or(String::new(), |schema| {
                    format!(", output_schema: {}", schema.name)
                });
                let time = budget.time_usage(now).map_or(String::new(), |(used, max)| {
                    format!(", {}s of {}", used.as_secs(), format_duration(max))
                });
                self.write_debug(format!(
                    "=== {} {at}: request ({} messages, {} new, tools: {tools}{effort}{format}{time}) ===\n{new}",
                    self.label(),
                    messages.len(),
                    messages.len() - sent,
                ));
            }
            sent = messages.len();
            let reply = client.chat(&messages, &self.tools)?;
            budget
                .add_tokens(reply.tokens)
                .map_err(|reason| reason.message(&self.label()))?;
            if self.debug.is_some() {
                let json = pretty(std::slice::from_ref(&Message::Assistant(reply.clone())));
                let tokens = reply
                    .tokens
                    .map_or(String::new(), |tokens| format!(", {tokens} tokens"));
                let used = budget.token_usage().map_or(String::new(), |(used, limit)| {
                    format!(", {used} of {limit} tokens used")
                });
                self.write_debug(format!(
                    "=== {} {at}: response ({} tool calls{tokens}{used}) ===\n{json}",
                    self.label(),
                    reply.tool_calls.len()
                ));
            }
            if reply.tool_calls.is_empty() {
                let text = reply.content.clone().unwrap_or_default();
                let Rejection {
                    index,
                    message,
                    error,
                } = match checks::run_checks(&mut checks, &text) {
                    Ok(output) => return Ok(output),
                    Err(rejection) => rejection,
                };
                self.write_debug(format!(
                    "=== {} {at}: {} ===\n{error}",
                    self.label(),
                    checks[index].rejection()
                ));
                let mut message = message;
                if checks[index].ends_iteration() {
                    match budget.next_iteration() {
                        Ok(iteration) => {
                            message = format!(
                                "{message}\nThis is iteration {iteration} of {}.",
                                self.max_iterations
                            );
                        }
                        Err(reason) => {
                            return Err(self.stop_error(&reason, &checks, Some((index, error))));
                        }
                    }
                }
                messages.push(Message::Assistant(reply));
                messages.push(Message::User(message));
                last_failure = Some((index, error));
                continue;
            }
            let calls = reply.tool_calls.clone();
            messages.push(Message::Assistant(reply));
            let count = calls.len();
            for (index, call) in calls.into_iter().enumerate() {
                if let Err(reason) = budget.check_time((self.clock)()) {
                    return Err(self.stop_error(&reason, &checks, last_failure));
                }
                let result = self.call_tool(&call);
                if self.debug.is_some() {
                    let outcome = match &result {
                        Ok(content) => format!("ok, {} characters", content.chars().count()),
                        Err(err) => format!("error: {err}"),
                    };
                    self.write_debug(format!(
                        "=== {} {at}: tool call {} of {count}: {} (id {}) ===\narguments:\n{}\nresult: {outcome}",
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
    }

    /// The error of a run that stops for `reason`. If the run used all its
    /// steps or iterations after a failed final check, the error names that
    /// check and its last error. If the run used its tokens or its time
    /// after a failed success check, the error adds the last error of the
    /// check.
    fn stop_error(
        &self,
        reason: &StopReason,
        checks: &[Box<dyn FinalCheck>],
        last_failure: Option<(usize, String)>,
    ) -> String {
        let label = self.label();
        match (reason, last_failure) {
            (StopReason::Steps { max }, Some((index, error))) => format!(
                "`{label}` {} in {max} steps, the last error: {error}",
                checks[index].failure()
            ),
            (StopReason::Iterations { max }, Some((index, error))) => format!(
                "`{label}` {} in {max} iterations, the last error: {error}",
                checks[index].failure()
            ),
            (StopReason::Tokens { .. } | StopReason::Time { .. }, Some((index, error)))
                if checks[index].ends_iteration() =>
            {
                format!("{}, the last error: {error}", reason.message(&label))
            }
            (reason, _) => reason.message(&label),
        }
    }

    /// The position of a step in debug headers: `step 3`, or
    /// `iteration 2 step 3` if the run can have more than one iteration.
    fn position(&self, budget: &RunBudget, step: u32) -> String {
        if self.max_iterations > 1 {
            format!("iteration {} step {step}", budget.iteration())
        } else {
            format!("step {step}")
        }
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
        AgentType::Autonomous => {
            "You are an autonomous agent that works in the current directory. Nobody \
             watches you while you work. Use the tools to do the task step by step. Read \
             before you change, and test your changes. When you think that the task is \
             done, reply with a short summary of the work, without tool calls."
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
