//! The native tools of a Managed Agent, from `native_tools=`.
//!
//! The native tools run on the side of Anthropic: `bash` and the file tools
//! in the cloud container of the session, and the web tools on the servers
//! of Anthropic. They cannot see the files of the machine that runs Atama.
//! [`NativeToolset::to_json`] writes the `agent_toolset_20260401` entry of
//! the agent.

use serde_json::{Value, json};

use crate::tools::Autonomy;

/// The type of the toolset of the native tools.
pub const TOOLSET_TYPE: &str = "agent_toolset_20260401";

/// The most domains of one list, `allowed_domains=` or `blocked_domains=`.
pub const MAX_DOMAINS: usize = 64;

/// One native tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeTool {
    Bash,
    Read,
    Write,
    Edit,
    Glob,
    Grep,
    WebFetch,
    WebSearch,
}

impl NativeTool {
    /// All native tools, in the order of the API documentation.
    pub const ALL: [NativeTool; 8] = [
        NativeTool::Bash,
        NativeTool::Read,
        NativeTool::Write,
        NativeTool::Edit,
        NativeTool::Glob,
        NativeTool::Grep,
        NativeTool::WebFetch,
        NativeTool::WebSearch,
    ];

    /// The name of the tool in the API.
    pub fn name(self) -> &'static str {
        match self {
            NativeTool::Bash => "bash",
            NativeTool::Read => "read",
            NativeTool::Write => "write",
            NativeTool::Edit => "edit",
            NativeTool::Glob => "glob",
            NativeTool::Grep => "grep",
            NativeTool::WebFetch => "web_fetch",
            NativeTool::WebSearch => "web_search",
        }
    }

    /// The tool with the name `name`, or `None`.
    pub fn from_name(name: &str) -> Option<NativeTool> {
        NativeTool::ALL.into_iter().find(|tool| tool.name() == name)
    }

    /// Whether the tool uses the web, so that the domain lists apply to it.
    pub fn is_web(self) -> bool {
        matches!(self, NativeTool::WebFetch | NativeTool::WebSearch)
    }

    /// Reads the value of `native_tools=`: `all`, or a list of tool names
    /// that commas separate.
    ///
    /// Fails on an empty list, an unknown name, or a name that is in the
    /// list twice.
    pub fn parse_list(value: &str) -> Result<Vec<NativeTool>, String> {
        if value.trim() == "all" {
            return Ok(NativeTool::ALL.to_vec());
        }
        let mut tools = Vec::new();
        for name in split_list(value) {
            let tool = NativeTool::from_name(name).ok_or_else(|| {
                let names: Vec<String> = NativeTool::ALL
                    .iter()
                    .map(|tool| format!("`{}`", tool.name()))
                    .collect();
                format!(
                    "`{name}` is not a native tool, expected `all` or {}",
                    names.join(", ")
                )
            })?;
            if tools.contains(&tool) {
                return Err(format!("native tool `{name}` is in `native_tools=` twice"));
            }
            tools.push(tool);
        }
        if tools.is_empty() {
            return Err("`native_tools=` has no tool".to_string());
        }
        Ok(tools)
    }
}

/// The permission policy of the native tools in the API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionPolicy {
    /// The server reads each call, and runs, denies or holds it.
    Auto,
    /// Each call runs.
    AlwaysAllow,
    /// Each call waits for a `user.tool_confirmation` event.
    AlwaysAsk,
}

impl PermissionPolicy {
    /// The value of `permission_policy.type`.
    pub fn name(self) -> &'static str {
        match self {
            PermissionPolicy::Auto => "auto",
            PermissionPolicy::AlwaysAllow => "always_allow",
            PermissionPolicy::AlwaysAsk => "always_ask",
        }
    }
}

impl From<Autonomy> for PermissionPolicy {
    /// The policy that a level of `autonomy=` sends.
    fn from(autonomy: Autonomy) -> Self {
        match autonomy {
            Autonomy::NoAsk => PermissionPolicy::AlwaysAllow,
            Autonomy::AskIfRisky => PermissionPolicy::Auto,
            Autonomy::AlwaysAsk => PermissionPolicy::AlwaysAsk,
        }
    }
}

/// The hosts that the web tools can reach.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum DomainFilter {
    /// No limit.
    #[default]
    None,
    /// From `allowed_domains=`: only these hosts and their subdomains.
    Allowed(Vec<String>),
    /// From `blocked_domains=`: not these hosts and their subdomains.
    Blocked(Vec<String>),
}

impl DomainFilter {
    /// Reads `allowed_domains=` and `blocked_domains=`.
    ///
    /// Fails if both are set, or a list is not valid (see
    /// [`parse_domains`]).
    pub fn from_values(allowed: Option<&str>, blocked: Option<&str>) -> Result<Self, String> {
        match (allowed, blocked) {
            (None, None) => Ok(DomainFilter::None),
            (Some(_), Some(_)) => Err(
                "`allowed_domains=` and `blocked_domains=` cannot be together, use one of them"
                    .to_string(),
            ),
            (Some(value), None) => Ok(DomainFilter::Allowed(parse_domains(
                "allowed_domains",
                value,
            )?)),
            (None, Some(value)) => Ok(DomainFilter::Blocked(parse_domains(
                "blocked_domains",
                value,
            )?)),
        }
    }
}

/// Reads a list of domains that commas separate, for the option `key`.
///
/// Fails if the list is empty or has more than [`MAX_DOMAINS`] domains, a
/// domain is twice in it, or a domain is not a plain host name with a dot,
/// as in `docs.example.com`: no scheme, port, path or `*`.
pub fn parse_domains(key: &str, value: &str) -> Result<Vec<String>, String> {
    let mut domains: Vec<String> = Vec::new();
    for domain in split_list(value) {
        let domain = domain.to_lowercase();
        let valid = domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
            && domain.len() <= 255
            && domain
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
        if !valid {
            return Err(format!(
                "`{domain}` in `{key}=` is not a host name, use a plain name as in \
                 `docs.example.com`, without a scheme, a port, a path or `*`"
            ));
        }
        if domains.contains(&domain) {
            return Err(format!("`{domain}` is in `{key}=` twice"));
        }
        domains.push(domain);
    }
    if domains.is_empty() {
        return Err(format!("`{key}=` has no domain"));
    }
    if domains.len() > MAX_DOMAINS {
        return Err(format!(
            "`{key}=` has {} domains, the most is {MAX_DOMAINS}",
            domains.len()
        ));
    }
    Ok(domains)
}

/// The items of a list that commas or new lines separate, without spaces.
/// A leading `- ` is not part of an item.
pub(crate) fn split_list(value: &str) -> impl Iterator<Item = &str> {
    value
        .split([',', '\n'])
        .map(|item| {
            let item = item.trim();
            item.strip_prefix("- ").unwrap_or(item).trim()
        })
        .filter(|item| !item.is_empty())
}

/// The native tools of an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeToolset {
    /// The tools that are on. All others are off.
    pub enabled: Vec<NativeTool>,
    pub permission: PermissionPolicy,
    /// The domain list of the web tools.
    pub web_domains: DomainFilter,
}

impl NativeToolset {
    /// The `agent_toolset_20260401` entry of the `tools` of the agent, or
    /// `None` if no tool is on.
    ///
    /// All tools are off in `default_config`, and each tool of `enabled`
    /// is on in `configs`, so that a tool is off unless the job names it.
    /// Each web tool gets the domain list.
    pub fn to_json(&self) -> Option<Value> {
        if self.enabled.is_empty() {
            return None;
        }
        let configs: Vec<Value> = self
            .enabled
            .iter()
            .map(|tool| {
                let mut config = json!({"name": tool.name(), "enabled": true});
                if tool.is_web() {
                    match &self.web_domains {
                        DomainFilter::None => {}
                        DomainFilter::Allowed(domains) => {
                            config["allowed_domains"] = json!(domains);
                        }
                        DomainFilter::Blocked(domains) => {
                            config["blocked_domains"] = json!(domains);
                        }
                    }
                }
                config
            })
            .collect();
        Some(json!({
            "type": TOOLSET_TYPE,
            "default_config": {
                "enabled": false,
                "permission_policy": {"type": self.permission.name()},
            },
            "configs": configs,
        }))
    }

    /// Whether a web tool is on.
    pub fn has_web(&self) -> bool {
        self.enabled.iter().any(|tool| tool.is_web())
    }
}
