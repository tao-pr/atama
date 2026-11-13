//! Writes an [`Actor`] back as the action statement of a job file.
//!
//! The parser reads action statements into actors (see the `FromStr` impl in
//! [`crate::parser`]). This module does the reverse, for the printed job and
//! the executor's log.

use std::fmt;

use super::{Actor, AgentType, Auth, list_files};
use crate::{tavily, websearch};

/// Writes the action statement that parses back into this actor, for
/// example `bash npm ci` or `api_call POST https://example.com payload={}`.
///
/// The executor logs this text, so it hides secrets: `auth` is written as
/// `auth=<kind>:***`, for example `auth=bearer:***`.
impl fmt::Display for Actor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Actor::BashScript { cmd, params } | Actor::BashSandbox { cmd, params } => {
                let name = match self {
                    Actor::BashScript { .. } => "bash",
                    _ => "bash-sandbox",
                };
                write!(f, "{name} {cmd}")?;
                for param in params {
                    write!(f, " {param}")?;
                }
                Ok(())
            }
            Actor::ApiCall {
                verb,
                url,
                input_file,
                payload,
                auth,
            } => {
                write!(f, "api_call {verb:?} {url}")?;
                write_option(f, "payload", payload)?;
                write_option(f, "input_file", input_file)?;
                write_auth(f, auth)
            }
            Actor::DownloadFromUrl {
                url,
                auth,
                mime,
                output_dir,
            } => {
                write!(f, "download {url}")?;
                write_option(f, "mime", mime)?;
                write_option(f, "output_dir", output_dir)?;
                write_auth(f, auth)
            }
            Actor::AgentTool { agent, params, .. } => {
                write!(f, "agent:{agent}")?;
                for param in params {
                    write!(f, " {}", quote_param(param))?;
                }
                Ok(())
            }
            Actor::TavilySearch { params } => {
                write!(f, "tavily_search")?;
                for param in params {
                    write!(f, " {param}")?;
                }
                Ok(())
            }
            Actor::WebSearch(options) => {
                let topic = quote_param(&format!("topic={}", options.topic));
                write!(f, "websearch {topic}")?;
                if options.max_results != websearch::DEFAULT_MAX_RESULTS {
                    write!(f, " max_results={}", options.max_results)?;
                }
                if options.fetch_content {
                    write!(f, " fetch_content=true")?;
                }
                if options.base_url != tavily::DEFAULT_BASE_URL {
                    write!(f, " base_url={}", options.base_url)?;
                }
                Ok(())
            }
            Actor::ForEach(action) => write!(f, "foreach {action}"),
            Actor::ReadFile { path } => {
                write!(f, "read_file")?;
                match path {
                    Some(path) => write!(f, " {}", quote_param(path)),
                    None => Ok(()),
                }
            }
            Actor::ListFiles { dir, pattern } => {
                write!(f, "list_files")?;
                if let Some(dir) = dir {
                    write!(f, " {}", quote_param(&format!("dir={dir}")))?;
                }
                if pattern != list_files::DEFAULT_PATTERN {
                    write!(f, " {}", quote_param(&format!("pattern={pattern}")))?;
                }
                Ok(())
            }
            Actor::WriteFile { path } => write!(f, "write_file {}", quote_param(path)),
            Actor::MockPlaceholder => write!(f, "mock"),
            Actor::MockFail => write!(f, "mock fail"),
        }
    }
}

/// Writes the agent type as it appears after `agent:`, for example `websearch`.
impl fmt::Display for AgentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            AgentType::Coder => "coder",
            AgentType::Summarizer => "summarizer",
            AgentType::Autonomous => "autonomous",
        })
    }
}

/// Puts the value of an agent param in double quotes if it has whitespace
/// or a quote, so that the parser reads it back as one param:
/// `task=Fix the tests` becomes `task="Fix the tests"`. A param without a
/// key is quoted whole.
fn quote_param(param: &str) -> String {
    if !param.contains(|c: char| c.is_whitespace() || c == '"') && !param.is_empty() {
        return param.to_string();
    }
    let escape = |text: &str| text.replace('\\', "\\\\").replace('"', "\\\"");
    match param.split_once('=') {
        Some((key, value)) if !key.contains(|c: char| c.is_whitespace() || c == '"') => {
            format!("{key}=\"{}\"", escape(value))
        }
        _ => format!("\"{}\"", escape(param)),
    }
}

/// Writes ` key=value` if the option is set.
fn write_option(f: &mut fmt::Formatter<'_>, key: &str, value: &Option<String>) -> fmt::Result {
    match value {
        Some(value) => write!(f, " {key}={value}"),
        None => Ok(()),
    }
}

/// Writes ` auth=<kind>:***` if `auth` is set, without the secret.
fn write_auth(f: &mut fmt::Formatter<'_>, auth: &Option<Auth>) -> fmt::Result {
    let kind = match auth {
        None => return Ok(()),
        Some(Auth::Bearer { .. }) => "bearer",
        Some(Auth::Jwt { .. }) => "jwt",
        Some(Auth::OAuth2 { .. }) => "oauth2",
        Some(Auth::ApiKey { .. }) => "api_key",
    };
    write!(f, " auth={kind}:***")
}
