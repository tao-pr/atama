//! The options of a `claude-agent` action.
//!
//! [`ClaudeAgentConfig::from_params`] reads the `key=value` options of the
//! action, and makes sure of the rules between them. [`ClaudeAgentConfig::check`]
//! reads them without the API key, so that the job parser can stop an
//! invalid job before it starts.

use std::str::FromStr;
use std::time::Duration;

use super::custom_tools;
use super::definition::{AgentDefinition, EnvironmentDefinition};
use super::native_tools::{DomainFilter, NativeTool, NativeToolset, PermissionPolicy};
use crate::agent_loop_openai_compat::agent_options::{
    DEFAULT_MAX_TIME, named_profile, parse_duration, read_api_key,
};
use crate::profile::{ModelProfile, parse_token_budget};
use crate::protect::parse_patterns;
use crate::tools::Autonomy;

/// The default most `user.message` events of a run: the task, and one for
/// each failed check of the reply.
pub const DEFAULT_MAX_TURNS: u32 = 3;

/// The default level of `autonomy=`.
pub const DEFAULT_AUTONOMY: Autonomy = Autonomy::AskIfRisky;

/// The keys of the options of a `claude-agent` action.
pub const OPTION_KEYS: [&str; 19] = [
    "model",
    "task",
    "system",
    "name",
    "agent_id",
    "native_tools",
    "autonomy",
    "on_ask",
    "allowed_domains",
    "blocked_domains",
    "local_tools",
    "protect",
    "environment",
    "environment_id",
    "max_time",
    "max_cost_usd",
    "token_budget",
    "max_turns",
    "output_schema",
];

/// The options that define an agent. An action with `agent_id=` cannot
/// have them, because the agent exists.
const DEFINITION_KEYS: [&str; 5] = [
    "name",
    "system",
    "native_tools",
    "allowed_domains",
    "blocked_domains",
];

/// Who answers a tool call that waits for approval, from `on_ask=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnAsk {
    /// `prompt`: ask on the terminal. Without a terminal, deny.
    Prompt,
    /// `deny`: deny each call.
    Deny,
}

impl FromStr for OnAsk {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, String> {
        match value {
            "prompt" => Ok(OnAsk::Prompt),
            "deny" => Ok(OnAsk::Deny),
            "judge" => Err("`on_ask=judge` is not available yet, use `prompt` or `deny`".into()),
            _ => Err(format!("`on_ask={value}` is not `prompt` or `deny`")),
        }
    }
}

/// The agent of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSource {
    /// Atama creates the agent, or updates it if its definition changed.
    Managed(AgentDefinition),
    /// The agent with this ID exists, from `agent_id=`. Atama does not
    /// change it.
    Existing(String),
}

/// The environment of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentSource {
    /// Atama creates the environment if the lock file has no equal one.
    Managed(EnvironmentDefinition),
    /// The environment with this ID exists, from `environment_id=`.
    Existing(String),
}

/// The configuration of one `claude-agent` action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeAgentConfig {
    /// The name of the model profile.
    pub profile: String,
    /// The base URL of the API, from the profile.
    pub base_url: String,
    pub api_key: String,
    pub task: String,
    pub agent: AgentSource,
    pub environment: EnvironmentSource,
    /// The names of the local tools in the registry, as in `bash`.
    pub local_tools: Vec<String>,
    pub autonomy: Autonomy,
    pub on_ask: OnAsk,
    /// The patterns of `protect=`.
    pub protect: Vec<String>,
    pub max_time: Duration,
    /// From `max_cost_usd=`, in cents.
    pub max_cost_cents: Option<u64>,
    /// From `token_budget=`, else from the profile.
    pub token_budget: Option<u64>,
    pub max_turns: u32,
    /// The raw value of `output_schema=`.
    pub output_schema: Option<String>,
}

/// The value of each option, if the params set it.
#[derive(Default)]
struct Values {
    values: Vec<(&'static str, String)>,
}

impl Values {
    /// Reads each param as one `key=value` option.
    ///
    /// Fails if a param has no `=`, its key is not in [`OPTION_KEYS`], its
    /// value is empty, or the params set the key twice.
    fn read(params: &[String]) -> Result<Self, String> {
        let mut values = Values::default();
        for param in params {
            let Some((key, value)) = param.split_once('=') else {
                return Err(format!(
                    "unexpected `{param}`, put the task in `task=\"...\"`"
                ));
            };
            let Some(key) = OPTION_KEYS.iter().find(|known| **known == key) else {
                let keys: Vec<String> = OPTION_KEYS.iter().map(|key| format!("`{key}`")).collect();
                return Err(format!(
                    "unknown option `{key}` of `agent:claude-agent`, expected {}",
                    keys.join(", ")
                ));
            };
            if value.is_empty() {
                return Err(format!("option `{key}` has no value"));
            }
            if values.get(key).is_some() {
                return Err(format!("option `{key}` is set twice"));
            }
            values.values.push((key, value.to_string()));
        }
        Ok(values)
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(known, _)| *known == key)
            .map(|(_, value)| value.as_str())
    }
}

/// Reads `max_cost_usd=`: an amount of US dollars, more than 0, with two
/// decimal places at most, as in `15`, `2.5` or `2.00`. Returns the cents.
/// It reads the digits as text, so that no rounding changes the amount.
pub fn parse_usd_cents(value: &str) -> Result<u64, String> {
    let invalid = || {
        format!(
            "`max_cost_usd={value}` is not an amount of dollars, use a number more than 0 \
             with two decimal places at most, as in `15`, `2.5` or `2.00`"
        )
    };
    let (dollars, cents) = value.split_once('.').unwrap_or((value, ""));
    let digits = |text: &str| text.chars().all(|c| c.is_ascii_digit());
    if dollars.is_empty() || !digits(dollars) || !digits(cents) || cents.len() > 2 {
        return Err(invalid());
    }
    if value.contains('.') && cents.is_empty() {
        return Err(invalid());
    }
    let dollars: u64 = dollars.parse().map_err(|_| invalid())?;
    let cents: u64 = format!("{cents:0<2}").parse().map_err(|_| invalid())?;
    let total = dollars
        .checked_mul(100)
        .and_then(|amount| amount.checked_add(cents))
        .filter(|total| *total > 0)
        .ok_or_else(invalid)?;
    Ok(total)
}

fn positive(key: &str, value: &str) -> Result<u32, String> {
    value
        .parse()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| format!("`{key}={value}` is not a positive number"))
}

impl ClaudeAgentConfig {
    /// Reads the configuration of a `claude-agent` action.
    ///
    /// - `profile` is the model profile that `model=` names.
    /// - `env_block` is the ```` ```claude-env:<name> ```` block that
    ///   `environment=<name>` names, if the job has it.
    /// - `default_name` is the name of the agent without `name=`.
    /// - `env` returns the value of an environment variable.
    ///
    /// Fails if an option is unknown, empty, set twice or invalid, a rule
    /// between the options is broken (see `PLAN-claude-agent.md`, section
    /// 4.3), the profile or the block is not the one that the options name,
    /// the profile sets `thinking=` or has no `api_key=`, or the variable of
    /// the API key is not set.
    pub fn from_params(
        params: &[String],
        profile: Option<&ModelProfile>,
        env_block: Option<&EnvironmentDefinition>,
        default_name: &str,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        let values = Values::read(params)?;
        let get = |key| values.get(key);

        let name = get("model").ok_or("`agent:claude-agent` needs `model=<profile>`")?;
        let profile = named_profile(name, profile)?;
        if profile.thinking.is_some() {
            return Err(format!(
                "model profile `{}` sets `thinking=`, but a claude-agent action cannot turn \
                 off thinking, remove `thinking=` from the profile",
                profile.name
            ));
        }
        if profile.api_key_env.is_none() {
            return Err(format!(
                "model profile `{}` needs `api_key=<ENV_VAR>` for a claude-agent action",
                profile.name
            ));
        }

        let local_tools = get("local_tools")
            .map(custom_tools::parse_names)
            .transpose()?
            .unwrap_or_default();
        let autonomy = get("autonomy")
            .map(str::parse)
            .transpose()?
            .unwrap_or(DEFAULT_AUTONOMY);
        let on_ask = get("on_ask")
            .map(str::parse)
            .transpose()?
            .unwrap_or(OnAsk::Prompt);
        if autonomy == Autonomy::AlwaysAsk && on_ask == OnAsk::Deny {
            return Err(
                "`autonomy=always_ask` and `on_ask=deny` cannot be together, because Atama \
                 then denies each tool call"
                    .to_string(),
            );
        }
        let protect = match get("protect") {
            None => Vec::new(),
            Some(_) if local_tools.is_empty() => {
                return Err(
                    "`protect=` needs `local_tools=`, because it protects the files of the \
                     local tools only"
                        .to_string(),
                );
            }
            Some(value) => parse_patterns(value)?,
        };

        let environment = match (get("environment"), get("environment_id")) {
            (Some(_), Some(_)) => {
                return Err(
                    "`environment=` and `environment_id=` cannot be together, use one of them"
                        .to_string(),
                );
            }
            (None, Some(id)) => EnvironmentSource::Existing(id.to_string()),
            (Some(name), None) => {
                let expected = format!("{}{name}", super::env_block::ENV_NAME_PREFIX);
                let block = env_block
                    .filter(|block| block.name == expected)
                    .ok_or_else(|| {
                        format!("claude-env block `{name}` is not defined in the job")
                    })?;
                EnvironmentSource::Managed(block.clone())
            }
            (None, None) => {
                EnvironmentSource::Managed(EnvironmentDefinition::default_environment())
            }
        };

        let agent = match get("agent_id") {
            Some(id) => {
                let set: Vec<String> = DEFINITION_KEYS
                    .iter()
                    .filter(|key| get(key).is_some())
                    .map(|key| format!("`{key}=`"))
                    .collect();
                if !set.is_empty() {
                    return Err(format!(
                        "`agent_id=` uses an agent that exists, so it cannot be with {}",
                        set.join(", ")
                    ));
                }
                AgentSource::Existing(id.to_string())
            }
            None => {
                let native = get("native_tools")
                    .map(NativeTool::parse_list)
                    .transpose()?
                    .unwrap_or_default();
                let web_domains =
                    DomainFilter::from_values(get("allowed_domains"), get("blocked_domains"))?;
                let native = NativeToolset {
                    enabled: native,
                    permission: PermissionPolicy::from(autonomy),
                    web_domains,
                };
                if native.web_domains != DomainFilter::None && !native.has_web() {
                    return Err(
                        "`allowed_domains=` and `blocked_domains=` need `web_search` or \
                         `web_fetch` in `native_tools=`"
                            .to_string(),
                    );
                }
                if native.has_web()
                    && let EnvironmentSource::Managed(environment) = &environment
                    && environment.blocks_the_web()
                {
                    return Err(format!(
                        "the web tools return nothing in environment `{}`, because it has \
                         `limited` networking with no `allowed_hosts`. Name a \
                         ```claude-env:<name>``` block with `allowed_hosts` in \
                         `environment=`",
                        environment.name
                    ));
                }
                AgentSource::Managed(AgentDefinition {
                    name: get("name").unwrap_or(default_name).to_string(),
                    model: profile.model.clone(),
                    // `effort=` of the profile comes in phase 4 of the plan.
                    effort: None,
                    system: get("system").map(str::to_string),
                    native,
                    local_tools: local_tools.clone(),
                })
            }
        };

        let max_time = get("max_time")
            .map(parse_duration)
            .transpose()?
            .unwrap_or(DEFAULT_MAX_TIME);
        let max_cost_cents = get("max_cost_usd").map(parse_usd_cents).transpose()?;
        let token_budget = get("token_budget")
            .map(parse_token_budget)
            .transpose()?
            .or(profile.token_budget);
        let max_turns = get("max_turns")
            .map(|value| positive("max_turns", value))
            .transpose()?
            .unwrap_or(DEFAULT_MAX_TURNS);

        let api_key = read_api_key(profile, env)?.unwrap_or_default();
        Ok(ClaudeAgentConfig {
            profile: profile.name.clone(),
            base_url: profile.base_url.clone(),
            api_key,
            task: get("task").unwrap_or_default().to_string(),
            agent,
            environment,
            local_tools,
            autonomy,
            on_ask,
            protect,
            max_time,
            max_cost_cents,
            token_budget,
            max_turns,
            output_schema: get("output_schema").map(str::to_string),
        })
    }

    /// Makes sure that `params` are valid options, as
    /// [`ClaudeAgentConfig::from_params`] does, but without the environment.
    /// The API key is not read, because the job can set it later.
    pub fn check(
        params: &[String],
        profile: Option<&ModelProfile>,
        env_block: Option<&EnvironmentDefinition>,
    ) -> Result<(), String> {
        let env = |_: &str| Some("not read".to_string());
        Self::from_params(params, profile, env_block, "check", &env).map(|_| ())
    }
}
