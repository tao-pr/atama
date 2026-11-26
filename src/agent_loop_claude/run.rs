//! [`ClaudeAgentRun`]: one run of a `claude-agent` action.
//!
//! A run gets the agent and the environment (see [`super::lock`]), starts
//! a session, opens its event stream, and sends the task. Anthropic runs the
//! agent loop. Atama reads the events: it runs each local tool call, answers
//! each native tool call that waits for approval, and returns the last
//! message of the agent when the agent ends its turn.

use std::cell::RefCell;
use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::api::{AgentsApi, EventStream, HttpAgentsApi};
use super::custom_tools::LocalTools;
use super::definition::{EnvironmentDefinition, check_existing_agent};
use super::events::{IdleStop, Permission, SessionEvent};
use super::lock::{AgentLock, LOCK_PATH};
use super::options::{AgentSource, ClaudeAgentConfig, EnvironmentSource, OnAsk};
use crate::actor::CLAUDE_AGENT;
use crate::agent_loop_openai_compat::agent_options::format_duration;
use crate::profile::ModelProfile;
use crate::protect::Protect;
use crate::schema::{self, OutputSchema};
use crate::tools::{Approver, DenyApprover, PromptApprover};

/// How often the run checks the deadline and the signals while it waits for
/// an event.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// The most times that a run opens the stream again after it ends too
/// early.
pub const MAX_RECONNECTS: u32 = 3;

/// The message of a denied native tool call, if the approver gives no
/// reason.
const DENIED: &str = "the call was denied";

/// What the thread of the stream sends to the run.
enum StreamMessage {
    Event(SessionEvent),
    /// The stream ended, with the error if it failed.
    End(Option<String>),
}

/// One run of a `claude-agent` action.
///
/// The builder methods replace the parts of a run, for tests: the API
/// client, the lock file, the approver, the local tools and the clock.
pub struct ClaudeAgentRun<'a> {
    default_name: String,
    api: Option<Box<dyn AgentsApi + 'a>>,
    lock_path: PathBuf,
    approver: Option<Rc<RefCell<dyn Approver>>>,
    tools: Option<LocalTools>,
    schema_block: Option<OutputSchema>,
    environment_block: Option<EnvironmentDefinition>,
    debug: Option<Box<dyn Write + 'a>>,
    clock: Box<dyn Fn() -> Instant + 'a>,
    poll_interval: Duration,
    /// `agent:claude-agent (<profile>)`, for the errors and the debug
    /// output.
    label: String,
}

impl<'a> ClaudeAgentRun<'a> {
    /// A run whose agent has the name `default_name` without `name=`.
    pub fn new(default_name: &str) -> Self {
        ClaudeAgentRun {
            default_name: default_name.to_string(),
            api: None,
            lock_path: PathBuf::from(LOCK_PATH),
            approver: None,
            tools: None,
            schema_block: None,
            environment_block: None,
            debug: None,
            clock: Box::new(Instant::now),
            poll_interval: DEFAULT_POLL_INTERVAL,
            label: CLAUDE_AGENT.to_string(),
        }
    }

    /// Uses `api` instead of an [`HttpAgentsApi`] for the profile.
    pub fn api(mut self, api: impl AgentsApi + 'a) -> Self {
        self.api = Some(Box::new(api));
        self
    }

    /// Uses the lock file at `path` instead of [`LOCK_PATH`].
    pub fn lock_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.lock_path = path.into();
        self
    }

    /// Uses `approver` instead of the approver of `on_ask=`.
    pub fn approver(mut self, approver: Rc<RefCell<dyn Approver>>) -> Self {
        self.approver = Some(approver);
        self
    }

    /// Uses `tools` instead of the tools of `local_tools=`.
    pub fn with_tools(mut self, tools: LocalTools) -> Self {
        self.tools = Some(tools);
        self
    }

    /// The schema block that `output_schema=<name>` names.
    pub fn schema_block(mut self, block: Option<OutputSchema>) -> Self {
        self.schema_block = block;
        self
    }

    /// The ```` ```claude-env:<name> ```` block that `environment=<name>`
    /// names.
    pub fn environment_block(mut self, block: Option<EnvironmentDefinition>) -> Self {
        self.environment_block = block;
        self
    }

    /// Writes the session, each event and each answer of the run to `out`.
    pub fn debug(mut self, out: impl Write + 'a) -> Self {
        self.debug = Some(Box::new(out));
        self
    }

    /// Uses `clock` instead of [`Instant::now`] for the time limit.
    pub fn clock(mut self, clock: impl Fn() -> Instant + 'a) -> Self {
        self.clock = Box::new(clock);
        self
    }

    /// Checks the deadline and the signals every `interval` while the run
    /// waits for an event.
    pub fn poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Runs the action with the options `params`, the model profile
    /// `profile` and the input `input`. It reads the environment of the
    /// process.
    pub fn run(
        self,
        params: &[String],
        profile: Option<&ModelProfile>,
        input: &str,
    ) -> Result<String, String> {
        self.run_with_env(params, profile, input, &|name| std::env::var(name).ok())
    }

    /// Runs the action as [`ClaudeAgentRun::run`] does, with the
    /// environment `env`. Tests use it.
    pub fn run_with_env(
        mut self,
        params: &[String],
        profile: Option<&ModelProfile>,
        input: &str,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<String, String> {
        if let Some(profile) = profile {
            self.label = format!("{CLAUDE_AGENT} ({})", profile.name);
        }
        let label = self.label.clone();
        let start = (self.clock)();
        let result = ClaudeAgentConfig::from_params(
            params,
            profile,
            self.environment_block.as_ref(),
            &self.default_name,
            env,
        )
        .and_then(|config| self.run_with(&config, input, start));
        result.map_err(|error| format!("{label}: {error}"))
    }

    fn run_with(
        &mut self,
        config: &ClaudeAgentConfig,
        input: &str,
        start: Instant,
    ) -> Result<String, String> {
        let output_schema = config
            .output_schema
            .as_deref()
            .map(|value| schema::load(value, self.schema_block.as_ref()))
            .transpose()?;
        let protect = match config.protect.is_empty() {
            true => None,
            false => {
                let cwd = crate::protect::working_dir()?;
                Some(Arc::new(Protect::new(&config.protect, &cwd)?))
            }
        };
        #[cfg(not(target_os = "macos"))]
        if protect.is_some() && config.local_tools.iter().any(|tool| tool == "bash") {
            return Err(
                "`protect=` needs `sandbox-exec` for the local `bash` tool, which only macOS \
                 has. Remove `bash` from `local_tools=`"
                    .to_string(),
            );
        }
        let mut message = first_message(config, input, output_schema.as_ref(), protect.as_deref())?;
        let deadline = start + config.max_time;

        let approver = self
            .approver
            .clone()
            .unwrap_or_else(|| match config.on_ask {
                OnAsk::Prompt => Rc::new(RefCell::new(PromptApprover::new())),
                OnAsk::Deny => Rc::new(RefCell::new(DenyApprover)),
            });
        let mut tools = match self.tools.take() {
            Some(tools) => tools,
            None => LocalTools::from_names(&config.local_tools, config.autonomy, approver.clone())?,
        };
        tools.set_deadline_time(Some(deadline));
        tools.set_protect(protect);

        let mut api = match self.api.take() {
            Some(api) => api,
            None => Box::new(HttpAgentsApi::new(&config.base_url, &config.api_key)),
        };
        let api = api.as_mut();
        let session = self.start_session(api, config)?;
        // The stream must be open before the first message, because it
        // gives only the events after it opens.
        let receiver = open_stream_for(api, &session)?;
        let mut run = Session {
            id: session,
            receiver,
            seen: HashSet::new(),
            last_reply: None,
            last_error: None,
            turns: 1,
            reconnects: 0,
        };
        api.send_events(&run.id, &[user_message(&message)])?;

        loop {
            let now = (self.clock)();
            if now >= deadline {
                interrupt(api, &run.id);
                return Err(format!(
                    "the session `{}` stopped after {}, because the time limit of the run is \
                     used",
                    run.id,
                    format_duration(config.max_time)
                ));
            }
            if crate::signals::stopping() {
                interrupt(api, &run.id);
                return Err(format!("the session `{}` stopped by a signal", run.id));
            }
            let events = match run.receiver.recv_timeout(self.poll_interval) {
                Ok(StreamMessage::Event(event)) => vec![event],
                Err(RecvTimeoutError::Timeout) => continue,
                Ok(StreamMessage::End(error)) => run.reconnect(api, error)?,
                Err(RecvTimeoutError::Disconnected) => run.reconnect(api, None)?,
            };
            for event in events {
                if !event.id().is_empty() && !run.seen.insert(event.id().to_string()) {
                    continue;
                }
                let elapsed = (self.clock)().saturating_duration_since(start);
                self.write_debug(&format!(
                    "{} of {}: {}",
                    format_duration(Duration::from_secs(elapsed.as_secs())),
                    format_duration(config.max_time),
                    describe(&event)
                ));
                match event {
                    SessionEvent::AgentMessage { text, .. } => run.last_reply = Some(text),
                    SessionEvent::CustomToolUse { id, name, input } => {
                        let (content, is_error) = match tools.call(&name, &input) {
                            Ok(result) => (result, false),
                            Err(error) => (error, true),
                        };
                        self.write_debug(&format!(
                            "{name} {}: {content}",
                            if is_error { "failed" } else { "returned" }
                        ));
                        api.send_events(
                            &run.id,
                            &[json!({
                                "type": "user.custom_tool_result",
                                "custom_tool_use_id": id,
                                "content": [{"type": "text", "text": content}],
                                "is_error": is_error,
                            })],
                        )?;
                    }
                    SessionEvent::AgentToolUse {
                        id,
                        name,
                        input,
                        permission: Permission::Ask,
                    } => {
                        let answer = approver.borrow_mut().approve(&name, &input);
                        let confirmation = match answer {
                            Ok(()) => json!({
                                "type": "user.tool_confirmation",
                                "tool_use_id": id,
                                "result": "allow",
                            }),
                            Err(reason) => json!({
                                "type": "user.tool_confirmation",
                                "tool_use_id": id,
                                "result": "deny",
                                "deny_message": if reason.is_empty() { DENIED.to_string() } else { reason },
                            }),
                        };
                        self.write_debug(&format!(
                            "{name}: {}",
                            confirmation["result"].as_str().unwrap_or_default()
                        ));
                        api.send_events(&run.id, &[confirmation])?;
                    }
                    SessionEvent::Usage { usage, .. } => {
                        if let Some(budget) = config.token_budget
                            && usage.total_tokens() > budget
                        {
                            interrupt(api, &run.id);
                            return Err(format!(
                                "the session `{}` used {} tokens, more than its budget of \
                                 {budget}",
                                run.id,
                                usage.total_tokens()
                            ));
                        }
                    }
                    SessionEvent::Error { message, .. } => run.last_error = Some(message),
                    SessionEvent::StatusIdle { stop, .. } => match stop {
                        IdleStop::RequiresAction { .. } => {}
                        IdleStop::EndTurn => {
                            let reply = run.last_reply.take().ok_or_else(|| {
                                format!("the session `{}` ended its turn without a message", run.id)
                            })?;
                            let Some(schema) = &output_schema else {
                                return Ok(reply);
                            };
                            match schema.check_reply(&reply) {
                                Ok(json) => return Ok(json),
                                Err(error) if run.turns >= config.max_turns => {
                                    return Err(format!(
                                        "the reply does not match the output schema after \
                                         {} turns: {error}",
                                        run.turns
                                    ));
                                }
                                Err(error) => {
                                    message = format!(
                                        "Your reply is not valid: {error}\nReply again with \
                                         only the JSON value."
                                    );
                                    api.send_events(&run.id, &[user_message(&message)])?;
                                    run.turns += 1;
                                }
                            }
                        }
                        IdleStop::RetriesExhausted => {
                            return Err(run.failure("failed after its retries"));
                        }
                        IdleStop::BudgetReached => {
                            let cost = config
                                .max_cost_cents
                                .map(|cents| format!(" of ${}.{:02}", cents / 100, cents % 100))
                                .unwrap_or_default();
                            return Err(format!("the session `{}` used its budget{cost}", run.id));
                        }
                        IdleStop::Other(reason) => {
                            return Err(run.failure(&format!("stopped with `{reason}`")));
                        }
                    },
                    SessionEvent::StatusTerminated { .. } => {
                        return Err(run.failure("ended before the agent finished its turn"));
                    }
                    _ => {}
                }
            }
        }
    }

    /// Gets the agent and the environment, and creates the session.
    /// Returns the ID of the session.
    fn start_session(
        &mut self,
        api: &mut dyn AgentsApi,
        config: &ClaudeAgentConfig,
    ) -> Result<String, String> {
        let needs_lock = matches!(config.agent, AgentSource::Managed(_))
            || matches!(config.environment, EnvironmentSource::Managed(_));
        let mut lock = match needs_lock {
            true => Some(AgentLock::load(&self.lock_path)?),
            false => None,
        };
        let agent = match &config.agent {
            AgentSource::Managed(definition) => {
                let lock = lock
                    .as_mut()
                    .expect("the lock is loaded for a managed agent");
                let agent = lock.ensure_agent(api, &definition.name, &definition.to_json())?;
                json!({"type": "agent", "id": agent.id, "version": agent.version})
            }
            AgentSource::Existing(id) => {
                let agent = api.get_agent(id)?;
                check_existing_agent(&agent, &config.local_tools)?;
                match agent["version"].as_u64() {
                    Some(version) => json!({"type": "agent", "id": id, "version": version}),
                    None => json!(id),
                }
            }
        };
        let environment = match &config.environment {
            EnvironmentSource::Managed(definition) => {
                let lock = lock
                    .as_mut()
                    .expect("the lock is loaded for a managed environment");
                lock.ensure_environment(api, &definition.name, &definition.to_json())?
            }
            EnvironmentSource::Existing(id) => id.clone(),
        };
        let mut body = json!({
            "agent": agent,
            "environment_id": environment,
            "title": format!("atama: {}", self.default_name),
        });
        if let Some(cents) = config.max_cost_cents {
            body["budget"] = json!({
                "type": "limit",
                "max_list_cost": {"amount": cents.to_string(), "currency": "USD"},
            });
        }
        let session = api.create_session(&body)?;
        self.write_debug(&format!(
            "agent {} in environment {environment}, session {session}",
            agent["id"].as_str().or(agent.as_str()).unwrap_or_default()
        ));
        self.write_debug(&format!(
            "console https://platform.claude.com/workspaces/default/sessions/{session}"
        ));
        Ok(session)
    }

    fn write_debug(&mut self, text: &str) {
        let label = &self.label;
        if let Some(out) = self.debug.as_mut() {
            let _ = writeln!(out, "=== {label} {text}");
        }
    }
}

/// The state of an open session.
struct Session {
    id: String,
    receiver: Receiver<StreamMessage>,
    /// The IDs of the events that the run handled.
    seen: HashSet<String>,
    /// The text of the last `agent.message` of the turn.
    last_reply: Option<String>,
    /// The message of the last `session.error`.
    last_error: Option<String>,
    /// The `user.message` events that the run sent.
    turns: u32,
    reconnects: u32,
}

impl Session {
    /// Opens the stream again after it ended, and returns the events of the
    /// history of the session. The run skips the events that it handled.
    ///
    /// Fails after [`MAX_RECONNECTS`] times.
    fn reconnect(
        &mut self,
        api: &mut dyn AgentsApi,
        error: Option<String>,
    ) -> Result<Vec<SessionEvent>, String> {
        if self.reconnects >= MAX_RECONNECTS {
            let reason = error.map(|error| format!(": {error}")).unwrap_or_default();
            return Err(format!(
                "the stream of the session `{}` ended {MAX_RECONNECTS} times{reason}",
                self.id
            ));
        }
        self.reconnects += 1;
        self.receiver = open_stream_for(api, &self.id)?;
        Ok(api.list_events(&self.id)?)
    }

    fn failure(&self, what: &str) -> String {
        let reason = self
            .last_error
            .as_ref()
            .map(|error| format!(": {error}"))
            .unwrap_or_default();
        format!("the session `{}` {what}{reason}", self.id)
    }
}

/// Opens the stream of `session`, and starts a thread that sends its events
/// to the returned receiver.
fn open_stream_for(
    api: &mut dyn AgentsApi,
    session: &str,
) -> Result<Receiver<StreamMessage>, String> {
    let mut stream: Box<dyn EventStream> = api.open_stream(session)?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        loop {
            let message = match stream.next_event() {
                Ok(Some(event)) => StreamMessage::Event(event),
                Ok(None) => StreamMessage::End(None),
                Err(error) => StreamMessage::End(Some(error)),
            };
            let end = matches!(message, StreamMessage::End(_));
            if sender.send(message).is_err() || end {
                return;
            }
        }
    });
    Ok(receiver)
}

/// Sends `user.interrupt`. It ignores an error, because the run fails
/// anyway.
fn interrupt(api: &mut dyn AgentsApi, session: &str) {
    let _ = api.send_events(session, &[json!({"type": "user.interrupt"})]);
}

fn user_message(text: &str) -> Value {
    json!({"type": "user.message", "content": [{"type": "text", "text": text}]})
}

/// The text of the first `user.message`: the task and the input, as the
/// OpenAI loop writes them, then the instructions of the output schema and
/// of the protected paths.
///
/// Fails if the action has no task and no input.
fn first_message(
    config: &ClaudeAgentConfig,
    input: &str,
    output_schema: Option<&OutputSchema>,
    protect: Option<&Protect>,
) -> Result<String, String> {
    let input = input.trim();
    let mut text = match (config.task.is_empty(), input.is_empty()) {
        (true, true) => return Err("the action has no `task=` and no input".to_string()),
        (false, true) => config.task.clone(),
        (true, false) => input.to_string(),
        (false, false) => format!("{}\n\nInput:\n{input}", config.task),
    };
    if let Some(schema) = output_schema {
        text.push_str(&format!("\n\n{}", schema.instruction()));
    }
    if let Some(protect) = protect {
        text.push_str(&format!("\n\n{}", protect.instruction()));
    }
    Ok(text)
}

/// One line about `event`, for the debug output.
fn describe(event: &SessionEvent) -> String {
    match event {
        SessionEvent::AgentMessage { text, .. } => format!("agent.message: {text}"),
        SessionEvent::AgentToolUse {
            name,
            input,
            permission,
            ..
        } => format!("agent.tool_use {name} ({permission:?}): {input}"),
        SessionEvent::AgentToolResult { is_error, .. } => {
            format!(
                "agent.tool_result{}",
                if *is_error { " (error)" } else { "" }
            )
        }
        SessionEvent::CustomToolUse { name, input, .. } => {
            format!("agent.custom_tool_use {name}: {input}")
        }
        SessionEvent::StatusRunning { .. } => "session.status_running".to_string(),
        SessionEvent::StatusIdle { stop, .. } => format!("session.status_idle ({stop:?})"),
        SessionEvent::StatusRescheduled { .. } => "session.status_rescheduled".to_string(),
        SessionEvent::StatusTerminated { .. } => "session.status_terminated".to_string(),
        SessionEvent::Usage { usage, .. } => format!(
            "session.usage: {} tokens{}",
            usage.total_tokens(),
            usage
                .list_cost_cents
                .map(|cents| format!(", ${}.{:02}", cents / 100, cents % 100))
                .unwrap_or_default()
        ),
        SessionEvent::Error { message, .. } => format!("session.error: {message}"),
        SessionEvent::Other { kind, .. } => kind.clone(),
    }
}
