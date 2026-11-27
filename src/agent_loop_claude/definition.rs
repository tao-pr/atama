//! The definitions of an agent and of an environment, and their JSON for
//! the API.
//!
//! [`AgentDefinition::to_json`] writes the body of `POST /agents`, and
//! [`EnvironmentDefinition::to_json`] the body of `POST /environments`. The
//! order of the keys is fixed, so that two equal definitions give the same
//! text. The lock file (see [`super::lock`]) compares these bodies.

use serde_json::{Value, json};

use super::custom_tools::{LOCAL_PREFIX, custom_tool_json};
use super::native_tools::NativeToolset;
use crate::tools::tool_by_name;

/// The name of the environment that an action without `environment=` and
/// `environment_id=` uses.
pub const DEFAULT_ENVIRONMENT: &str = "atama-default";

/// An agent that Atama creates, or updates when its definition changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDefinition {
    /// The name of the agent, and its key in the lock file.
    pub name: String,
    /// The model identifier.
    pub model: String,
    /// The effort level, as in `high`. `None` uses the default of the model.
    pub effort: Option<String>,
    pub system: Option<String>,
    pub native: NativeToolset,
    /// The names of the local tools in the registry, as in `bash`.
    pub local_tools: Vec<String>,
}

impl AgentDefinition {
    /// The body of `POST /agents`.
    pub fn to_json(&self) -> Value {
        let model = match &self.effort {
            None => json!(self.model),
            Some(effort) => json!({"id": self.model, "effort": effort}),
        };
        let mut tools: Vec<Value> = self.native.to_json().into_iter().collect();
        tools.extend(
            self.local_tools
                .iter()
                .filter_map(|name| tool_by_name(name))
                .map(|tool| custom_tool_json(tool.as_ref())),
        );
        let mut body = json!({"name": self.name, "model": model});
        if let Some(system) = &self.system {
            body["system"] = json!(system);
        }
        body["tools"] = json!(tools);
        body
    }
}

/// Makes sure that the local tools `local_tools` can answer each custom
/// tool of `agent`, the JSON of an agent that exists (`agent_id=`).
///
/// Fails if a custom tool of the agent is not `local_<name>` for a name of
/// `local_tools`.
pub fn check_existing_agent(agent: &Value, local_tools: &[String]) -> Result<(), String> {
    let id = agent["id"].as_str().unwrap_or("the agent");
    for tool in agent["tools"].as_array().into_iter().flatten() {
        if tool["type"] != "custom" {
            continue;
        }
        let name = tool["name"].as_str().unwrap_or_default();
        let known = name
            .strip_prefix(LOCAL_PREFIX)
            .is_some_and(|local| local_tools.iter().any(|tool| tool == local));
        if !known {
            return Err(format!(
                "agent `{id}` has the custom tool `{name}`, but no tool of `local_tools=` \
                 answers it"
            ));
        }
    }
    Ok(())
}

/// The network access of a cloud environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Networking {
    /// All hosts, except a safety block list of the API.
    Unrestricted,
    /// No host, except the hosts that the fields allow.
    Limited {
        allowed_hosts: Vec<String>,
        allow_package_managers: bool,
        allow_mcp_servers: bool,
    },
}

/// A cloud environment that Atama creates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentDefinition {
    /// The name of the environment, and its key in the lock file.
    pub name: String,
    pub networking: Networking,
}

impl EnvironmentDefinition {
    /// The environment [`DEFAULT_ENVIRONMENT`]: `limited` networking with
    /// no host.
    pub fn default_environment() -> Self {
        EnvironmentDefinition {
            name: DEFAULT_ENVIRONMENT.to_string(),
            networking: Networking::Limited {
                allowed_hosts: Vec::new(),
                allow_package_managers: false,
                allow_mcp_servers: false,
            },
        }
    }

    /// The body of `POST /environments`.
    pub fn to_json(&self) -> Value {
        let networking = match &self.networking {
            Networking::Unrestricted => json!({"type": "unrestricted"}),
            Networking::Limited {
                allowed_hosts,
                allow_package_managers,
                allow_mcp_servers,
            } => json!({
                "type": "limited",
                "allowed_hosts": allowed_hosts,
                "allow_package_managers": allow_package_managers,
                "allow_mcp_servers": allow_mcp_servers,
            }),
        };
        json!({
            "name": self.name,
            "config": {"type": "cloud", "networking": networking},
        })
    }

    /// Whether the environment allows no host to the web tools: `limited`
    /// networking with no `allowed_hosts`.
    pub fn blocks_the_web(&self) -> bool {
        matches!(&self.networking, Networking::Limited { allowed_hosts, .. } if allowed_hosts.is_empty())
    }
}
