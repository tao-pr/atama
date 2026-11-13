//! Parser for job config markdown files.
//!
//! The format is line based:
//!
//! - A ```` ```config ```` block before the `# <name>` heading defines model
//!   profiles, one per line (see [`crate::profile`]).
//! - A ```` ```json:<name> ```` block before the `# <name>` heading defines
//!   an output schema for agent actions (see [`crate::schema`]).
//! - `# <name>` names the job; text after it is the job description.
//! - `## <id>` starts a task; its heading text is the task id.
//! - Top-level `- begin_if:`, `- actions:`, `- actions-parallel:`,
//!   `- if_fail:` and `- if_succeed:` bullets in a task are fields.
//!   `- actions-parallel:max=<n>` limits how many actions run at the same
//!   time. Indented
//!   bullets under `- actions:` or `- actions-parallel:` are the actions. Any other text is part of the task description.
//! - An action is whitespace-separated tokens: the action name, then its
//!   parameters. The name selects the [`Actor`]:
//!   - `bash <cmd> [params...]` is [`Actor::BashScript`].
//!   - `bash-sandbox <cmd> [params...]` is [`Actor::BashSandbox`].
//!   - `api_call <verb> <url> [payload=<text>] [input_file=<path>]
//!     [auth=bearer:<token>]` is [`Actor::ApiCall`].
//!   - `download <url> [mime=<type>] [output_dir=<dir>] [auth=bearer:<token>]`
//!     is [`Actor::DownloadFromUrl`].
//!   - `tavily_search [options...] [query...]` is [`Actor::TavilySearch`].
//!   - `websearch topic=<text> [max_results=<n>] [fetch_content=<true|false>]
//!     [base_url=<url>]` is [`Actor::WebSearch`].
//!   - `read_file [path]` is [`Actor::ReadFile`]. A path with spaces is in
//!     quotes. Without a path, the action reads the path from its input.
//!   - `list_files [dir=<path>] [pattern=<glob>]` is [`Actor::ListFiles`].
//!     Without `dir=`, the action reads the directory from its input. The
//!     default pattern is `*`.
//!   - `write_file <path>` is [`Actor::WriteFile`]. A path with spaces is in
//!     quotes.
//!   - `foreach <action>` is [`Actor::ForEach`]. The action after `foreach`
//!     is any other action, as in `foreach bash wc -c`.
//!   - `mock fail` is [`Actor::MockFail`]. `mock` with any other parameters
//!     is [`Actor::MockPlaceholder`].
//!   - `agent:<type> model=<profile> [params...]` is [`Actor::AgentTool`].
//!     The type is `coder` or `summarizer`. The profile must be in the
//!     ```` ```config ```` block. If `output_schema=` has a name, not a
//!     path, the job must have a schema block with that name.
//! - Lines inside ``` code fences are always treated as description text.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::str::FromStr;

use crate::actor::{Actor, AgentType, ApiVerb, Auth, list_files};
use crate::agent_loop_openai_compat::agent_options::AgentConfig;
use crate::job::{Event, Hook, Job, Task, Trigger};
use crate::loop_block::{self, LoopBlocks};
use crate::passer::Passer;
use crate::profile::{self, Profiles};
use crate::schema::{self, Schemas};
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
/// The kind of an open fenced block before the `# <job name>` heading.
enum Block {
    /// ```` ```config ````: the model profiles.
    Config,
    /// ```` ```json:<name> ````: an output schema.
    Schema(String),
    /// ```` ```loop:<name> ````: a loop block.
    Loop(String),
}

struct ParsedTask {
    task: Task,
    description: Vec<String>,
    line: usize,
    begin_if_line: Option<usize>,
    /// Whether the task has an `actions-parallel` field, if it has an
    /// `actions` or `actions-parallel` field.
    parallel_field: Option<bool>,
}

/// Parses the contents of a job config markdown file.
pub fn parse_job(input: &str) -> Result<Job, ParseError> {
    let mut name: Option<String> = None;
    let mut job_description: Vec<String> = Vec::new();
    let mut tasks: Vec<ParsedTask> = Vec::new();
    let mut in_actions = false;
    let mut in_code_block = false;
    let mut profiles = Profiles::new();
    let mut schemas = Schemas::new();
    let mut loops = LoopBlocks::new();
    // The open ```config, ```json:<name> or ```loop:<name> block: the line
    // number of its fence, its kind, and the lines after the fence.
    let mut open_block: Option<(usize, Block, Vec<&str>)> = None;
    let mut has_config = false;

    for (index, raw_line) in input.lines().enumerate() {
        let line_no = index + 1;
        let line = raw_line.trim_end();

        if let Some((start, kind, lines)) = open_block.as_mut() {
            if !line.trim_start().starts_with("```") {
                lines.push(line);
                continue;
            }
            let start = *start;
            let at = |(index, message)| ParseError::new(start + 1 + index, message);
            match kind {
                Block::Config => {
                    profiles = profile::parse_profiles(lines.iter().copied()).map_err(at)?
                }
                Block::Schema(name) => {
                    let parsed = schema::parse_block(name, &lines.join("\n")).map_err(at)?;
                    schemas.insert(name.to_string(), parsed);
                }
                Block::Loop(name) => {
                    let parsed = loop_block::parse_block(name, lines).map_err(at)?;
                    loops.insert(name.to_string(), parsed);
                }
            }
            open_block = None;
            continue;
        }
        if let Some(loop_name) = loop_block::block_name(line).filter(|_| !in_code_block) {
            if name.is_some() {
                return Err(ParseError::new(
                    line_no,
                    "the ```loop:<name> block must come before the `# <job name>` heading",
                ));
            }
            loop_block::check_name(loop_name)
                .map_err(|message| ParseError::new(line_no, message))?;
            if loops.contains_key(loop_name) {
                return Err(ParseError::new(
                    line_no,
                    format!("loop block `{loop_name}` is defined twice"),
                ));
            }
            open_block = Some((line_no, Block::Loop(loop_name.to_string()), Vec::new()));
            continue;
        }
        if let Some(schema_name) = schema::block_name(line).filter(|_| !in_code_block) {
            if name.is_some() {
                return Err(ParseError::new(
                    line_no,
                    "the ```json:<name> schema block must come before the `# <job name>` heading",
                ));
            }
            schema::check_name(schema_name).map_err(|message| ParseError::new(line_no, message))?;
            if schemas.contains_key(schema_name) {
                return Err(ParseError::new(
                    line_no,
                    format!("schema `{schema_name}` is defined twice"),
                ));
            }
            open_block = Some((line_no, Block::Schema(schema_name.to_string()), Vec::new()));
            continue;
        }
        if line.trim() == "```config" && !in_code_block {
            if name.is_some() {
                return Err(ParseError::new(
                    line_no,
                    "the ```config block must come before the `# <job name>` heading",
                ));
            }
            if has_config {
                return Err(ParseError::new(line_no, "more than one ```config block"));
            }
            has_config = true;
            open_block = Some((line_no, Block::Config, Vec::new()));
            continue;
        }

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
                        parallel: false,
                        max_parallel: None,
                        if_fail: None,
                        if_succeed: None,
                    },
                    description: Vec::new(),
                    line: line_no,
                    begin_if_line: None,
                    parallel_field: None,
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
                current.task.actions.push(parse_action(
                    action.trim(),
                    line_no,
                    &profiles,
                    &schemas,
                    &loops,
                )?);
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
                "actions" | "actions-parallel" => {
                    let parallel = key == "actions-parallel";
                    match current.parallel_field {
                        Some(earlier) if earlier == parallel => {
                            return Err(ParseError::new(
                                line_no,
                                format!("duplicate `{key}` field"),
                            ));
                        }
                        Some(_) => {
                            return Err(ParseError::new(
                                line_no,
                                "a task has `actions` or `actions-parallel`, not both",
                            ));
                        }
                        None => {}
                    }
                    current.parallel_field = Some(parallel);
                    current.task.parallel = parallel;
                    let (max, value) = parallel_limit(value)
                        .map_err(|message| ParseError::new(line_no, message))?;
                    if max.is_some() && !parallel {
                        return Err(ParseError::new(
                            line_no,
                            "`max=` works only with `actions-parallel`, as in \
                             `- actions-parallel:max=2`",
                        ));
                    }
                    current.task.max_parallel = max;
                    if !value.is_empty() {
                        current
                            .task
                            .actions
                            .push(parse_action(value, line_no, &profiles, &schemas, &loops)?);
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

    if let Some((start, kind, _)) = open_block {
        let block = match kind {
            Block::Config => "```config".to_string(),
            Block::Schema(name) => format!("```json:{name}"),
            Block::Loop(name) => format!("```loop:{name}"),
        };
        return Err(ParseError::new(
            start,
            format!("the {block} block has no closing ```"),
        ));
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

/// Parses an action. An agent action also gets the model profile that its
/// `model=` names, the schema block that its `output_schema=` names, the
/// loop block that its `loop=` names, and the model profile that its
/// `judge_model=` names. Then the parser makes sure that the
/// options of the agent action, with the options of its loop block, are
/// valid (see [`AgentConfig::check`]).
fn parse_action(
    value: &str,
    line: usize,
    profiles: &Profiles,
    schemas: &Schemas,
    loops: &LoopBlocks,
) -> Result<Actor, ParseError> {
    let mut actor: Actor = parse_value(value, line, "action")?;
    // The agent of a `foreach` action needs its profile too.
    let agent_actor = match &mut actor {
        Actor::ForEach(inner) => inner.as_mut(),
        other => other,
    };
    if let Actor::AgentTool {
        agent,
        params,
        profile,
        output_schema,
        loop_block,
        judge_profile,
    } = agent_actor
    {
        let error = |message| ParseError::new(line, message);
        *profile = Some(profile::resolve(*agent, params, profiles).map_err(error)?);
        *output_schema = schema::resolve(params, schemas).map_err(error)?;
        // Only an `autonomous` agent takes `loop=`. For another agent,
        // `AgentConfig::check` names the option.
        if *agent == AgentType::Autonomous {
            *loop_block = loop_block::resolve(params, loops).map_err(error)?;
        }
        let merged = match loop_block {
            Some(block) => block.merge(params),
            None => params.clone(),
        };
        // `judge_model=` can come from the loop block. For another agent,
        // `AgentConfig::check` names the option.
        if *agent == AgentType::Autonomous
            && let Some(name) = merged.iter().find_map(|p| p.strip_prefix("judge_model="))
        {
            *judge_profile = Some(Box::new(profile::find(name, profiles).map_err(error)?));
        }
        AgentConfig::check(*agent, &merged, profile.as_ref()).map_err(error)?;
    }
    Ok(actor)
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
    matches!(
        key,
        "begin_if" | "actions" | "actions-parallel" | "if_fail" | "if_succeed"
    )
    .then_some((key, value))
}

/// Splits the value of an `actions` or `actions-parallel` field into the
/// `max=<n>` limit at its start, if it has one, and the rest, which is an
/// inline action or empty. `max=2 mock` gives `(Some(2), "mock")`.
///
/// Fails if `max` is not a positive number.
fn parallel_limit(value: &str) -> Result<(Option<usize>, &str), String> {
    let Some(rest) = value.strip_prefix("max=") else {
        return Ok((None, value));
    };
    let (number, rest) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let max = number
        .parse()
        .ok()
        .filter(|max| *max > 0)
        .ok_or_else(|| format!("`max={number}` is not a positive number"))?;
    Ok((Some(max), rest.trim()))
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
            "bash" | "bash-sandbox" => {
                let (cmd, params) = params
                    .split_first()
                    .ok_or_else(|| format!("`{name}` needs a command, as in `{name} npm ci`"))?;
                let cmd = cmd.to_string();
                let params = params.iter().map(|param| param.to_string()).collect();
                Ok(if name == "bash" {
                    Actor::BashScript { cmd, params }
                } else {
                    Actor::BashSandbox { cmd, params }
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
            "foreach" => {
                if rest.trim().is_empty() {
                    return Err("`foreach` needs an action, as in `foreach bash wc -c`".to_string());
                }
                let action: Actor = rest.parse()?;
                if matches!(action, Actor::ForEach(_)) {
                    return Err("`foreach` cannot run another `foreach`".to_string());
                }
                Ok(Actor::ForEach(Box::new(action)))
            }
            "read_file" => match split_quoted(rest)?.as_slice() {
                [] => Ok(Actor::ReadFile { path: None }),
                [path] => Ok(Actor::ReadFile {
                    path: Some(path.clone()),
                }),
                _ => Err(
                    "`read_file` takes one path, as in `read_file ./notes.txt`. \
                          Put a path with spaces in quotes"
                        .to_string(),
                ),
            },
            "list_files" => {
                let params = split_quoted(rest)?;
                let params: Vec<&str> = params.iter().map(String::as_str).collect();
                let mut options = parse_options(&params, &["dir", "pattern"])?;
                let pattern = options
                    .remove("pattern")
                    .unwrap_or_else(|| list_files::DEFAULT_PATTERN.to_string());
                list_files::check_pattern(&pattern)?;
                Ok(Actor::ListFiles {
                    dir: options.remove("dir"),
                    pattern,
                })
            }
            "write_file" => match split_quoted(rest)?.as_slice() {
                [path] => Ok(Actor::WriteFile { path: path.clone() }),
                [] => Err(
                    "`write_file` needs a path, as in `write_file ./output/result.txt`".to_string(),
                ),
                _ => Err(
                    "`write_file` takes one path, as in `write_file ./output/result.txt`. \
                     Put a path with spaces in quotes"
                        .to_string(),
                ),
            },
            "mock" => Ok(match params.first() {
                Some(&"fail") => Actor::MockFail,
                _ => Actor::MockPlaceholder,
            }),
            // Prefixed action, e.g. "agent:{}"
            other => match other.strip_prefix("agent:") {
                Some(agent) => Ok(Actor::AgentTool {
                    agent: agent.parse()?,
                    params: split_quoted(rest)?,
                    profile: None,
                    output_schema: None,
                    loop_block: None,
                    judge_profile: None,
                }),
                None => Err(format!(
                    "unknown action `{other}`, expected `bash`, `bash-sandbox`, `api_call`, `download`, \
                     `tavily_search`, `websearch`, `read_file`, `list_files`, `write_file`, \
                     `foreach`, `mock` or `agent:<type>`"
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
            "autonomous" => Ok(AgentType::Autonomous),
            "websearch" => Err(
                "`agent:websearch` is now the `websearch` action, because it asks no model"
                    .to_string(),
            ),
            _ => Err(format!(
                "unknown agent type `{s}`, expected `agent:coder`, `agent:summarizer` or \
                 `agent:autonomous`"
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
