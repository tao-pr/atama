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
//!
//! `PLAN-claude-agent.md` describes the plan of the agent type.

pub mod api;
pub mod events;
pub mod sse;
