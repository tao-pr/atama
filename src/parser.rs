//! Parser for job config markdown files.
//!
//! The format is line based:
//!
//! - `# <name>` names the job; text after it is the job description.
//! - `## <id>` starts a task; its heading text is the task id.
//! - Top-level `- begin_if:`, `- actions:`, `- if_fail:` and `- if_succeed:`
//!   bullets in a task are fields. Indented bullets under `- actions:` are the
//!   actions. Any other text is part of the task description.
//! - An action is whitespace-separated tokens: the action name, then its
//!   parameters. The name selects the [`Actor`]:
//!   - `bash <cmd> [params...]` is [`Actor::BashScript`].
//!   - `api_call <verb> <url> [payload=<text>] [input_file=<path>]
//!     [auth=bearer:<token>]` is [`Actor::ApiCall`].
//!   - `download <url> [mime=<type>] [output_dir=<dir>] [auth=bearer:<token>]`
//!     is [`Actor::DownloadFromUrl`].
//!   - `tavily_search [options...] [query...]` is [`Actor::TavilySearch`].
//!   - `websearch topic=<text> [max_results=<n>] [fetch_content=<true|false>]
//!     [base_url=<url>]` is [`Actor::WebSearch`].
//!   - `mock fail` is [`Actor::MockFail`]. `mock` with any other parameters
//!     is [`Actor::MockPlaceholder`].
//!   - `agent:<type> [params...]` is [`Actor::AgentTool`]. The type is
//!     `coder` or `summarizer`.
//! - Lines inside ``` code fences are always treated as description text.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::str::FromStr;

use crate::actor::{Actor, AgentType, ApiVerb, Auth};
use crate::job::{Event, Hook, Job, Task, Trigger};
use crate::passer::Passer;
use crate::websearch::{self, WebSearchOptions};

/// An error found while parsing a job config file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// 1-based line number the error refers to.
    pub line: usize,
    pub message: String,
}

impl ParseError {
    fn new(line: usize, message: impl Into<String>) -> Self {
        ParseError {
            line,
            message: message.into(),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

/// A task plus the line numbers needed to report validation errors.
struct ParsedTask {
    task: Task,
    description: Vec<String>,
    line: usize,
    begin_if_line: Option<usize>,
}

/// Parses the contents of a job config markdown file.
pub fn parse_job(input: &str) -> Result<Job, ParseError> {
    let mut name: Option<String> = None;
    let mut job_description: Vec<String> = Vec::new();
    let mut tasks: Vec<ParsedTask> = Vec::new();
    let mut in_actions = false;
    let mut in_code_block = false;

    for (index, raw_line) in input.lines().enumerate() {
        let line_no = index + 1;
        let line = raw_line.trim_end();

        if line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
        }

        if !in_code_block {
            if let Some(id) = heading(line, "##") {
                if name.is_none() {
                    return Err(ParseError::new(
                        line_no,
                        "task heading found before the `# <job name>` heading",
                    ));
                }
                if id.is_empty() {
                    return Err(ParseError::new(line_no, "task heading is empty"));
                }
                tasks.push(ParsedTask {
                    task: Task {
                        id: id.to_string(),
                        description: String::new(),
                        begin_if: Trigger::default(),
                        actions: Vec::new(),
                        if_fail: None,
                        if_succeed: None,
                    },
                    description: Vec::new(),
                    line: line_no,
                    begin_if_line: None,
                });
                in_actions = false;
                continue;
            }

            if let Some(job_name) = heading(line, "#") {
                if name.is_some() {
                    return Err(ParseError::new(
                        line_no,
                        "more than one `# <job name>` heading",
                    ));
                }
                if job_name.is_empty() {
                    return Err(ParseError::new(line_no, "job name heading is empty"));
                }
                name = Some(job_name.to_string());
                continue;
            }
        }

        if name.is_none() {
            if line.trim().is_empty() {
                continue;
            }
            return Err(ParseError::new(
                line_no,
                "expected the `# <job name>` heading before any other text",
            ));
        }

        let Some(current) = tasks.last_mut() else {
            job_description.push(line.to_string());
            continue;
        };

        if in_code_block || line.trim_start().starts_with("```") {
            in_actions = false;
            current.description.push(line.to_string());
            continue;
        }

        if in_actions {
            let is_indented = line.starts_with(' ') || line.starts_with('\t');
            if let Some(action) = bullet_text(line.trim_start()).filter(|_| is_indented) {
                current
                    .task
                    .actions
                    .push(parse_value(action.trim(), line_no, "action")?);
                continue;
            }
            if line.trim().is_empty() {
                continue;
            }
            in_actions = false;
        }

        if let Some((key, value)) = bullet_text(line).and_then(field) {
            let value = value.trim();
            match key {
                "actions" => {
                    if !current.task.actions.is_empty() {
                        return Err(ParseError::new(line_no, "duplicate `actions` field"));
                    }
                    if !value.is_empty() {
                        current
                            .task
                            .actions
                            .push(parse_value(value, line_no, "action")?);
                    }
                    in_actions = true;
                }
                "begin_if" => {
                    if current.begin_if_line.is_some() {
                        return Err(ParseError::new(line_no, "duplicate `begin_if` field"));
                    }
                    current.task.begin_if = parse_value(value, line_no, key)?;
                    current.begin_if_line = Some(line_no);
                }
                "if_fail" | "if_succeed" => {
                    let slot = if key == "if_fail" {
                        &mut current.task.if_fail
                    } else {
                        &mut current.task.if_succeed
                    };
                    if slot.is_some() {
                        return Err(ParseError::new(line_no, format!("duplicate `{key}` field")));
                    }
                    *slot = Some(parse_value(value, line_no, key)?);
                }
                _ => unreachable!("field() only returns known keys"),
            }
            continue;
        }

        current.description.push(line.to_string());
    }

    let Some(name) = name else {
        return Err(ParseError::new(1, "missing `# <job name>` heading"));
    };
    if tasks.is_empty() {
        return Err(ParseError::new(1, "job has no `## <task>` sections"));
    }

    validate(&tasks)?;

    Ok(Job {
        name,
        description: join_text(&job_description),
        tasks: tasks
            .into_iter()
            .map(|parsed| Task {
                description: join_text(&parsed.description),
                ..parsed.task
            })
            .collect(),
    })
}

/// Checks rules that need the whole job: unique ids and valid trigger references.
fn validate(tasks: &[ParsedTask]) -> Result<(), ParseError> {
    let mut ids = HashSet::new();
    for parsed in tasks {
        if !ids.insert(parsed.task.id.as_str()) {
            return Err(ParseError::new(
                parsed.line,
                format!("duplicate task id `{}`", parsed.task.id),
            ));
        }
    }

    for (index, parsed) in tasks.iter().enumerate() {
        let Some(line) = parsed.begin_if_line else {
            continue;
        };
        match &parsed.task.begin_if {
            Trigger::PrevTask(_) if index == 0 => {
                return Err(ParseError::new(
                    line,
                    "the first task has no previous task to depend on",
                ));
            }
            Trigger::Task { id, .. } if *id == parsed.task.id => {
                return Err(ParseError::new(
                    line,
                    format!("task `{id}` depends on itself"),
                ));
            }
            Trigger::Task { id, .. } if !ids.contains(id.as_str()) => {
                return Err(ParseError::new(line, format!("unknown task id `{id}`")));
            }
            _ => {}
        }
    }

    Ok(())
}

/// Returns the text after a `- ` or `* ` bullet marker.
fn bullet_text(line: &str) -> Option<&str> {
    line.strip_prefix("- ").or_else(|| line.strip_prefix("* "))
}

/// Splits a bullet into a known field key and its value.
fn field(item: &str) -> Option<(&str, &str)> {
    let (key, value) = item.split_once(':').unwrap_or((item, ""));
    let key = key.trim();
    matches!(key, "begin_if" | "actions" | "if_fail" | "if_succeed").then_some((key, value))
}

fn parse_value<T: FromStr<Err = String>>(
    value: &str,
    line: usize,
    key: &str,
) -> Result<T, ParseError> {
    if value.is_empty() {
        return Err(ParseError::new(line, format!("`{key}` has no value")));
    }
    value
        .parse()
        .map_err(|message| ParseError::new(line, message))
}

/// The trimmed text of a heading that starts with `marker`, such as `##`.
/// The marker alone is an empty heading. Returns `None` if the line is not
/// such a heading: `##` is not a `#` heading.
fn heading<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(marker)?;
    (rest.is_empty() || rest.starts_with([' ', '\t'])).then(|| rest.trim())
}

/// Joins description lines and trims surrounding blank lines.
fn join_text(lines: &[String]) -> String {
    lines.join("\n").trim().to_string()
}

impl FromStr for Actor {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let (name, rest) = s.split_once(char::is_whitespace).unwrap_or((s, ""));
        if name.is_empty() {
            return Err("action is empty".to_string());
        }
        let params: Vec<&str> = rest.split_whitespace().collect();
        match name {
            "bash" => {
                let (cmd, params) = params
                    .split_first()
                    .ok_or("`bash` needs a command, as in `bash npm ci`")?;
                Ok(Actor::BashScript {
                    cmd: cmd.to_string(),
                    params: params.iter().map(|param| param.to_string()).collect(),
                })
            }
            "api_call" => {
                let [verb, url, options @ ..] = params.as_slice() else {
                    return Err("`api_call` needs a verb and a URL, \
                                as in `api_call GET https://example.com`"
                        .to_string());
                };
                let mut options = parse_options(options, &["payload", "input_file", "auth"])?;
                Ok(Actor::ApiCall {
                    verb: verb.parse()?,
                    url: url.to_string(),
                    input_file: options.remove("input_file"),
                    payload: options.remove("payload"),
                    auth: options
                        .remove("auth")
                        .map(|auth| parse_auth(&auth))
                        .transpose()?,
                })
            }
            "download" => {
                let [url, options @ ..] = params.as_slice() else {
                    return Err(
                        "`download` needs a URL, as in `download https://example.com/a.zip`"
                            .to_string(),
                    );
                };
                let mut options = parse_options(options, &["mime", "output_dir", "auth"])?;
                Ok(Actor::DownloadFromUrl {
                    url: url.to_string(),
                    auth: options
                        .remove("auth")
                        .map(|auth| parse_auth(&auth))
                        .transpose()?,
                    mime: options.remove("mime"),
                    output_dir: options.remove("output_dir"),
                })
            }
            "tavily_search" => Ok(Actor::TavilySearch {
                params: params.iter().map(|param| param.to_string()).collect(),
            }),
            "websearch" => {
                let params = split_quoted(rest)?;
                let params: Vec<&str> = params.iter().map(String::as_str).collect();
                let mut options = parse_options(&params, &websearch::OPTION_KEYS)?;
                Ok(Actor::WebSearch(WebSearchOptions::from_values(
                    options.remove("topic"),
                    options.remove("max_results"),
                    options.remove("fetch_content"),
                    options.remove("base_url"),
                )?))
            }
            "mock" => Ok(match params.first() {
                Some(&"fail") => Actor::MockFail,
                _ => Actor::MockPlaceholder,
            }),
            // Prefixed action, e.g. "agent:{}"
            other => match other.strip_prefix("agent:") {
                Some(agent) => Ok(Actor::AgentTool {
                    agent: agent.parse()?,
                    params: split_quoted(rest)?,
                }),
                None => Err(format!(
                    "unknown action `{other}`, expected `bash`, `api_call`, `download`, \
                     `tavily_search`, `websearch`, `mock` or `agent:<type>`"
                )),
            },
        }
    }
}

/// Splits the params of an agent action at whitespace, but not inside
/// double quotes, and removes the quotes. `task="Fix the tests"` becomes
/// `task=Fix the tests`. Inside quotes, `\"` is a quote and `\\` is a
/// backslash. Other text, and the spaces inside quotes, do not change.
///
/// Fails if a quote has no closing quote.
fn split_quoted(text: &str) -> Result<Vec<String>, String> {
    let mut params = Vec::new();
    let mut param = String::new();
    // A param can be empty only if it has quotes, as in `""`.
    let mut started = false;
    let mut quoted = false;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if quoted => match chars.next() {
                Some(next @ ('"' | '\\')) => param.push(next),
                Some(next) => {
                    param.push('\\');
                    param.push(next);
                }
                None => param.push('\\'),
            },
            c if c.is_whitespace() && !quoted => {
                if started {
                    params.push(std::mem::take(&mut param));
                    started = false;
                }
            }
            c => {
                param.push(c);
                started = true;
            }
        }
    }
    if quoted {
        return Err(format!("a quote in `{}` has no closing quote", text.trim()));
    }
    if started {
        params.push(param);
    }
    Ok(params)
}

/// Parses `key=value` tokens. Each key must be one of `keys` and can occur
/// only once.
fn parse_options(tokens: &[&str], keys: &[&str]) -> Result<HashMap<String, String>, String> {
    let expected = || {
        keys.iter()
            .map(|key| format!("`{key}=`"))
            .collect::<Vec<_>>()
            .join(" or ")
    };
    let mut options = HashMap::new();
    for token in tokens {
        let Some((key, value)) = token.split_once('=') else {
            return Err(format!("unexpected `{token}`, expected {}", expected()));
        };
        if !keys.contains(&key) {
            return Err(format!("unknown option `{key}`, expected {}", expected()));
        }
        if value.is_empty() {
            return Err(format!("option `{key}` has no value"));
        }
        if options.insert(key.to_string(), value.to_string()).is_some() {
            return Err(format!("duplicate option `{key}`"));
        }
    }
    Ok(options)
}

/// Parses the value of an `auth=` option. Only `bearer:<token>` is
/// supported, and the token is used as written.
fn parse_auth(value: &str) -> Result<Auth, String> {
    match value.split_once(':') {
        Some(("bearer", token)) if !token.is_empty() => Ok(Auth::Bearer {
            token: Passer::Data(token.to_string()),
        }),
        _ => Err(format!(
            "unsupported auth `{value}`, expected `auth=bearer:<token>`"
        )),
    }
}

/// Parses a verb in any case, such as `GET` or `get`.
impl FromStr for ApiVerb {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_ascii_uppercase().as_str() {
            "GET" => ApiVerb::GET,
            "POST" => ApiVerb::POST,
            "PUT" => ApiVerb::PUT,
            "DELETE" => ApiVerb::DELETE,
            "PATCH" => ApiVerb::PATCH,
            "HEAD" => ApiVerb::HEAD,
            _ => {
                return Err(format!(
                    "unknown API verb `{s}`, expected GET, POST, PUT, DELETE, PATCH or HEAD"
                ));
            }
        })
    }
}

impl FromStr for AgentType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "coder" => Ok(AgentType::Coder),
            "summarizer" => Ok(AgentType::Summarizer),
            "websearch" => Err(
                "`agent:websearch` is now the `websearch` action, because it asks no model"
                    .to_string(),
            ),
            _ => Err(format!(
                "unknown agent type `{s}`, expected `agent:coder` or `agent:summarizer`"
            )),
        }
    }
}

impl FromStr for Trigger {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if let Some(rest) = s.strip_prefix("prev task") {
            return match parse_event(rest)? {
                Event::Begin => Err("`prev task begin` is not a supported trigger".to_string()),
                event => Ok(Trigger::PrevTask(event)),
            };
        }
        if let Some(rest) = s.strip_prefix("task(") {
            let (id, rest) = rest
                .split_once(')')
                .ok_or_else(|| format!("missing `)` in trigger `{s}`"))?;
            let id = id.trim();
            if id.is_empty() {
                return Err(format!("empty task id in trigger `{s}`"));
            }
            return Ok(Trigger::Task {
                id: id.to_string(),
                on: parse_event(rest)?,
            });
        }
        Err(format!("unknown trigger condition `{s}`"))
    }
}

fn parse_event(s: &str) -> Result<Event, String> {
    let words = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match words.as_str() {
        "succeed" => Ok(Event::Succeed),
        "fail" => Ok(Event::Fail),
        "succeed or fail" => Ok(Event::SucceedOrFail),
        "begin" => Ok(Event::Begin),
        _ => Err(format!(
            "unknown event `{words}`, expected `succeed`, `fail`, `succeed or fail` or `begin`"
        )),
    }
}

impl FromStr for Hook {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        match s {
            "die" => return Ok(Hook::Die),
            "restart" => return Ok(Hook::Restart),
            _ => {}
        }
        if let Some(count) = s.strip_prefix("retry=") {
            return count
                .trim()
                .parse()
                .map(Hook::Retry)
                .map_err(|_| format!("invalid retry count in `{s}`"));
        }
        if let Some((name, rest)) = s.split_once('(') {
            let arg = rest
                .strip_suffix(')')
                .ok_or_else(|| format!("missing `)` in hook `{s}`"))?
                .trim()
                .to_string();
            return match name.trim() {
                "call_api" => Ok(Hook::CallApi(arg)),
                "write_file" => Ok(Hook::WriteFile(arg)),
                "write_error_file" => Ok(Hook::WriteErrorFile(arg)),
                other => Err(format!("unknown hook `{other}`")),
            };
        }
        Err(format!("unknown hook `{s}`"))
    }
}
