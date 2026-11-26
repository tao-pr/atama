//! The `claude-agent` agent type: runs a Claude Managed Agent.
//!
//! The OpenAI compatible loop (see [`crate::agent_loop_openai_compat`])
//! runs the agent loop in Atama. A Managed Agent is different: Anthropic
//! runs the loop. Atama stores an agent and an environment, starts a
//! session, sends the task as an event, and reads the events of the session
//! from a stream. The native tools of Claude run in a cloud container. The
//! custom tools run in Atama.
//!
//! The API is in beta. Each request sends the beta header [`api::BETA`].
//! The modules are:
//!
//! - [`api`] has the [`api::AgentsApi`] trait and its HTTP client.
//! - [`sse`] reads the server-sent events of the event stream.
//! - [`events`] reads one event into a [`events::SessionEvent`].
//! - [`options`] reads the options of the action into
//!   [`options::ClaudeAgentConfig`].
//! - [`native_tools`] writes the toolset of the native tools.
//! - [`custom_tools`] has the local tools, which the agent gets as custom
//!   tools named `local_<name>`.
//! - [`definition`] writes the JSON of an agent and of an environment.
//! - [`env_block`] reads the ```` ```claude-env:<name> ```` blocks.
//! - [`lock`] keeps the IDs of the agents and environments in
//!   `.atama/claude-lock.json`.
//! - [`run`] has [`ClaudeAgentRun`], one run of an action: the session,
//!   its events, the tool calls and the reply.
//!
//! `PLAN-claude-agent.md` describes the plan of the agent type.

pub mod api;
pub mod custom_tools;
pub mod definition;
pub mod env_block;
pub mod events;
pub mod lock;
pub mod native_tools;
pub mod options;
pub mod run;
pub mod sse;

pub use run::ClaudeAgentRun;
