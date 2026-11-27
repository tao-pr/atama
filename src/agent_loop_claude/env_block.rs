//! The ```` ```claude-env:<name> ```` blocks of a job file.
//!
//! A block defines a cloud environment for the `claude-agent` actions that
//! name it with `environment=<name>`. Each line is `<key>: <value>`, as in a
//! budget block:
//!
//! ````markdown
//! ```claude-env:research
//! networking: limited
//! allowed_hosts: arxiv.org, docs.rs
//! allow_package_managers: true
//! ```
//! ````

use super::definition::{EnvironmentDefinition, Networking};
use super::native_tools::split_list;
use crate::schema::{MAX_NAME_CHARS, is_name};

/// The keys that a block can set.
pub const ENV_KEYS: [&str; 4] = [
    "networking",
    "allowed_hosts",
    "allow_package_managers",
    "allow_mcp_servers",
];

/// The prefix of the name of each environment that a block defines, as in
/// `atama-research`.
pub const ENV_NAME_PREFIX: &str = "atama-";

/// The name of a block from its opening fence, as in `research` from
/// ```` ```claude-env:research ````. Returns `None` if the line is not such
/// a fence.
pub fn block_name(line: &str) -> Option<&str> {
    line.trim().strip_prefix("```claude-env:").map(str::trim)
}

/// Makes sure that `name` can name a block.
pub fn check_name(name: &str) -> Result<(), String> {
    if is_name(name) {
        Ok(())
    } else {
        Err(format!(
            "`{name}` is not a claude-env block name, use 1 to {MAX_NAME_CHARS} letters, \
             digits, `_` or `-`, as in ```claude-env:research"
        ))
    }
}

/// Reads the lines of the block `name`. The environment gets the name
/// `atama-<name>`.
///
/// Fails with the index of the line and a message if a line is not
/// `<key>: <value>`, a key is unknown or set twice, a value is invalid, or
/// `unrestricted` networking has a field of `limited` networking.
pub fn parse_block(name: &str, lines: &[&str]) -> Result<EnvironmentDefinition, (usize, String)> {
    let mut networking: Option<(usize, String)> = None;
    let mut allowed_hosts: Option<(usize, Vec<String>)> = None;
    let mut package_managers: Option<(usize, bool)> = None;
    let mut mcp_servers: Option<(usize, bool)> = None;
    for (index, line) in lines.iter().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            return Err((index, format!("expected `<key>: <value>`, found `{line}`")));
        };
        let (key, value) = (key.trim(), value.trim());
        let twice = || {
            Err((
                index,
                format!("`{key}` is set twice in claude-env block `{name}`"),
            ))
        };
        match key {
            "networking" => {
                if !matches!(value, "limited" | "unrestricted") {
                    return Err((
                        index,
                        format!("`networking: {value}` is not `limited` or `unrestricted`"),
                    ));
                }
                if networking.replace((index, value.to_string())).is_some() {
                    return twice();
                }
            }
            "allowed_hosts" => {
                let hosts = parse_hosts(value).map_err(|error| (index, error))?;
                if allowed_hosts.replace((index, hosts)).is_some() {
                    return twice();
                }
            }
            "allow_package_managers" | "allow_mcp_servers" => {
                let flag = parse_bool(key, value).map_err(|error| (index, error))?;
                let field = if key == "allow_package_managers" {
                    &mut package_managers
                } else {
                    &mut mcp_servers
                };
                if field.replace((index, flag)).is_some() {
                    return twice();
                }
            }
            _ => {
                let keys: Vec<String> = ENV_KEYS.iter().map(|key| format!("`{key}`")).collect();
                return Err((
                    index,
                    format!(
                        "unknown key `{key}` in claude-env block `{name}`, expected {}",
                        keys.join(", ")
                    ),
                ));
            }
        }
    }
    let networking = match networking {
        Some((_, value)) if value == "unrestricted" => {
            let limited_field = [
                allowed_hosts
                    .as_ref()
                    .map(|(index, _)| (*index, "allowed_hosts")),
                package_managers.map(|(index, _)| (index, "allow_package_managers")),
                mcp_servers.map(|(index, _)| (index, "allow_mcp_servers")),
            ]
            .into_iter()
            .flatten()
            .min();
            if let Some((index, key)) = limited_field {
                return Err((
                    index,
                    format!("`{key}` needs `networking: limited`, not `unrestricted`"),
                ));
            }
            Networking::Unrestricted
        }
        _ => Networking::Limited {
            allowed_hosts: allowed_hosts.map(|(_, hosts)| hosts).unwrap_or_default(),
            allow_package_managers: package_managers.is_some_and(|(_, flag)| flag),
            allow_mcp_servers: mcp_servers.is_some_and(|(_, flag)| flag),
        },
    };
    Ok(EnvironmentDefinition {
        name: format!("{ENV_NAME_PREFIX}{name}"),
        networking,
    })
}

fn parse_bool(key: &str, value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("`{key}: {value}` is not `true` or `false`")),
    }
}

/// Reads `allowed_hosts`: host names that commas separate. A host can start
/// with `*.`, which matches each subdomain.
fn parse_hosts(value: &str) -> Result<Vec<String>, String> {
    let mut hosts: Vec<String> = Vec::new();
    for host in split_list(value) {
        let name = host.strip_prefix("*.").unwrap_or(host);
        let valid = name.contains('.')
            && !name.starts_with('.')
            && !name.ends_with('.')
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
        if !valid {
            return Err(format!(
                "`{host}` in `allowed_hosts` is not a host name, use a name as in \
                 `api.example.com` or `*.example.com`, without a scheme, a port or a path"
            ));
        }
        let host = host.to_lowercase();
        if hosts.contains(&host) {
            return Err(format!("`{host}` is in `allowed_hosts` twice"));
        }
        hosts.push(host);
    }
    if hosts.is_empty() {
        return Err("`allowed_hosts` has no host".to_string());
    }
    Ok(hosts)
}
