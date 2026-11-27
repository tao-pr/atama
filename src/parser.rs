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
//! - Top-level `- begin_if:`, `- skip_if:`, `- actions:`,
//!   `- actions-parallel:`, `- if_fail:` and `- if_succeed:` bullets in a
//!   task are fields.
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
//!   - `agent:<type>` is [`Actor::AgentTool`]. The type is `coder`,
//!     `summarizer` or `autonomous`. The options of the agent are not on
//!     the line of the action. They are a list under it, one
//!     `- <key>=<value>` bullet for each option, indented more than the
//!     action. A value can be in double quotes. An indented line without a
//!     bullet continues the value of the option above it, after a space:
//!
//!     ```markdown
//!     - actions:
//!       - agent:coder
//!         - model=local-qwen
//!         - max_steps=15
//!         - task=Fix the failing test.
//!     ```
//!
//!     `model=<profile>` is necessary, and the profile must be in the
//!     ```` ```config ```` block. If `output_schema=` has a name, not a
//!     path, the job must have a schema block with that name. A
//!     `foreach agent:<type>` action has the list in the same way.
//! - Lines inside ``` code fences are always treated as description text.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::str::FromStr;

use crate::actor::{Actor, AgentType, ApiVerb, Auth, CLAUDE_AGENT, list_files};
use crate::agent_loop_claude::definition::EnvironmentDefinition;
use crate::agent_loop_claude::env_block;
use crate::agent_loop_claude::options::ClaudeAgentConfig;
use crate::agent_loop_openai_compat::agent_options::AgentConfig;
use crate::budget_block::{self, BudgetBlocks};
use crate::job::{Event, Hook, Job, SkipIf, Task, Trigger};
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
    /// ```` ```budget:<name> ````: a budget block.
    Budget(String),
    /// ```` ```claude-env:<name> ````: a cloud environment of a
    /// `claude-agent` action.
    ClaudeEnv(String),
}

/// The blocks before the `# <job name>` heading.
#[derive(Default)]
struct Blocks {
    profiles: Profiles,
    schemas: Schemas,
    budgets: BudgetBlocks,
    /// The ```` ```claude-env:<name> ```` blocks, by name.
    envs: BTreeMap<String, EnvironmentDefinition>,
}

/// Where an action is in the job, for the default name of a
/// `claude-agent` action.
struct Place<'a> {
    job: &'a str,
    task: &'a str,
    /// The number of the action in its task, from 1.
    number: usize,
}

impl Place<'_> {
    /// The default name of the agent of a `claude-agent` action:
    /// `atama-<job>-<task>-<number>`, in lower case, with `-` for each run
    /// of other characters than letters and digits.
    fn agent_name(&self) -> String {
        let text = format!("atama-{}-{}-{}", self.job, self.task, self.number);
        let mut name = String::new();
        for c in text.chars() {
            if c.is_ascii_alphanumeric() {
                name.push(c.to_ascii_lowercase());
            } else if !name.ends_with('-') {
                name.push('-');
            }
        }
        name.trim_matches('-').to_string()
    }
}

/// An action of a task that the parser read, but did not parse yet,
/// because the list of its options can follow on the next lines.
struct PendingAction {
    /// The text of the action bullet, without the bullet marker.
    text: String,
    /// The line number of the action bullet.
    line: usize,
    /// The indentation of the action bullet. A bullet with more
    /// indentation is an option of the action.
    indent: usize,
    /// The line number and the text of each option bullet.
    options: Vec<(usize, String)>,
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

/// A problem in a job file that does not stop the job, for example an
/// option that an agent ignores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseWarning {
    /// 1-based line number the warning refers to.
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

/// Parses the contents of a job config markdown file. It drops the
/// warnings (see [`parse_job_with_warnings`]).
pub fn parse_job(input: &str) -> Result<Job, ParseError> {
    parse_job_with_warnings(input).map(|(job, _)| job)
}

/// Parses the contents of a job config markdown file, and returns the job
/// and its warnings, in the order of their lines.
pub fn parse_job_with_warnings(input: &str) -> Result<(Job, Vec<ParseWarning>), ParseError> {
    let mut name: Option<String> = None;
    let mut job_description: Vec<String> = Vec::new();
    let mut tasks: Vec<ParsedTask> = Vec::new();
    let mut in_actions = false;
    let mut pending: Option<PendingAction> = None;
    let mut in_code_block = false;
    let mut blocks = Blocks::default();
    let mut warnings: Vec<ParseWarning> = Vec::new();
    // The open ```config, ```json:<name> or ```budget:<name> block: the line
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
                    blocks.profiles = profile::parse_profiles(lines.iter().copied()).map_err(at)?
                }
                Block::Schema(name) => {
                    let parsed = schema::parse_block(name, &lines.join("\n")).map_err(at)?;
                    blocks.schemas.insert(name.to_string(), parsed);
                }
                Block::Budget(name) => {
                    let parsed = budget_block::parse_block(name, lines).map_err(at)?;
                    blocks.budgets.insert(name.to_string(), parsed);
                }
                Block::ClaudeEnv(name) => {
                    let parsed = env_block::parse_block(name, lines).map_err(at)?;
                    blocks.envs.insert(name.to_string(), parsed);
                }
            }
            open_block = None;
            continue;
        }
        if let Some(env_name) = env_block::block_name(line).filter(|_| !in_code_block) {
            if name.is_some() {
                return Err(ParseError::new(
                    line_no,
                    "the ```claude-env:<name> block must come before the `# <job name>` heading",
                ));
            }
            env_block::check_name(env_name).map_err(|message| ParseError::new(line_no, message))?;
            if blocks.envs.contains_key(env_name) {
                return Err(ParseError::new(
                    line_no,
                    format!("claude-env block `{env_name}` is defined twice"),
                ));
            }
            open_block = Some((line_no, Block::ClaudeEnv(env_name.to_string()), Vec::new()));
            continue;
        }
        if let Some(budget_name) = budget_block::block_name(line).filter(|_| !in_code_block) {
            if name.is_some() {
                return Err(ParseError::new(
                    line_no,
                    "the ```budget:<name> block must come before the `# <job name>` heading",
                ));
            }
            budget_block::check_name(budget_name)
                .map_err(|message| ParseError::new(line_no, message))?;
            if blocks.budgets.contains_key(budget_name) {
                return Err(ParseError::new(
                    line_no,
                    format!("budget block `{budget_name}` is defined twice"),
                ));
            }
            open_block = Some((line_no, Block::Budget(budget_name.to_string()), Vec::new()));
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
            if blocks.schemas.contains_key(schema_name) {
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
                if let Some(current) = tasks.last_mut() {
                    flush(
                        &mut pending,
                        &mut current.task,
                        &blocks,
                        name.as_deref().unwrap_or_default(),
                        &mut warnings,
                    )?;
                }
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
                        skip_if: None,
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
            flush(
                &mut pending,
                &mut current.task,
                &blocks,
                name.as_deref().unwrap_or_default(),
                &mut warnings,
            )?;
            in_actions = false;
            current.description.push(line.to_string());
            continue;
        }

        if in_actions {
            let indent = indentation(line);
            let deeper = pending
                .as_ref()
                .is_some_and(|action| indent > action.indent);
            if let Some(item) = bullet_text(line.trim_start()).filter(|_| indent > 0) {
                if let Some(action) = pending.as_mut().filter(|_| deeper) {
                    action.options.push((line_no, item.trim().to_string()));
                    continue;
                }
                flush(
                    &mut pending,
                    &mut current.task,
                    &blocks,
                    name.as_deref().unwrap_or_default(),
                    &mut warnings,
                )?;
                pending = Some(PendingAction {
                    text: item.trim().to_string(),
                    line: line_no,
                    indent,
                    options: Vec::new(),
                });
                continue;
            }
            if line.trim().is_empty() {
                continue;
            }
            // An indented line without a bullet continues the last option.
            if let Some((_, option)) = pending
                .as_mut()
                .filter(|_| deeper)
                .and_then(|action| action.options.last_mut())
            {
                option.push(' ');
                option.push_str(line.trim());
                continue;
            }
            flush(
                &mut pending,
                &mut current.task,
                &blocks,
                name.as_deref().unwrap_or_default(),
                &mut warnings,
            )?;
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
                        let place = Place {
                            job: name.as_deref().unwrap_or_default(),
                            task: &current.task.id,
                            number: current.task.actions.len() + 1,
                        };
                        let action =
                            parse_action(value, line_no, &[], &blocks, &place, &mut warnings)?;
                        current.task.actions.push(action);
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
                "skip_if" => {
                    if current.task.skip_if.is_some() {
                        return Err(ParseError::new(line_no, "duplicate `skip_if` field"));
                    }
                    current.task.skip_if = Some(parse_value(value, line_no, key)?);
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

    if let Some(current) = tasks.last_mut() {
        flush(
            &mut pending,
            &mut current.task,
            &blocks,
            name.as_deref().unwrap_or_default(),
            &mut warnings,
        )?;
    }
    if let Some((start, kind, _)) = open_block {
        let block = match kind {
            Block::Config => "```config".to_string(),
            Block::Schema(name) => format!("```json:{name}"),
            Block::Budget(name) => format!("```budget:{name}"),
            Block::ClaudeEnv(name) => format!("```claude-env:{name}"),
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

    let job = Job {
        name,
        description: join_text(&job_description),
        tasks: tasks
            .into_iter()
            .map(|parsed| Task {
                description: join_text(&parsed.description),
                ..parsed.task
            })
            .collect(),
    };
    Ok((job, warnings))
}

/// Parses the pending action, if there is one, and adds it to the actions
/// of `task`.
fn flush(
    pending: &mut Option<PendingAction>,
    task: &mut Task,
    blocks: &Blocks,
    job: &str,
    warnings: &mut Vec<ParseWarning>,
) -> Result<(), ParseError> {
    if let Some(action) = pending.take() {
        let place = Place {
            job,
            task: &task.id,
            number: task.actions.len() + 1,
        };
        let actor = parse_action(
            &action.text,
            action.line,
            &action.options,
            blocks,
            &place,
            warnings,
        )?;
        task.actions.push(actor);
    }
    Ok(())
}

/// Parses an action and the list of its options. Only an agent action
/// takes options, and only from the list, not from the line of the action.
/// Each item of `options` is its line number and its text.
///
/// An agent action also gets the model profile that its
/// `model=` names, the schema block that its `output_schema=` names, the
/// budget block that its `budget=` names, and the model profile that its
/// `judge_model=` names. Then the parser makes sure that the
/// options of the agent action, with the options of its budget block, are
/// valid (see [`AgentConfig::check`]).
fn parse_action(
    value: &str,
    line: usize,
    options: &[(usize, String)],
    blocks: &Blocks,
    place: &Place,
    warnings: &mut Vec<ParseWarning>,
) -> Result<Actor, ParseError> {
    let mut actor: Actor = parse_value(value, line, "action")?;
    // The agent of a `foreach` action needs its profile too.
    let agent_actor = match &mut actor {
        Actor::ForEach(inner) => inner.as_mut(),
        other => other,
    };
    match agent_actor {
        Actor::AgentTool { agent, params, .. } => {
            take_options(&format!("agent:{agent}"), params, options, line)?;
        }
        Actor::ClaudeAgent { params, .. } => take_options(CLAUDE_AGENT, params, options, line)?,
        _ => {
            if let Some((option_line, text)) = options.first() {
                return Err(ParseError::new(
                    *option_line,
                    format!(
                        "`{text}` is indented under an action that takes no list of options, \
                         only `agent:<type>` takes one. To add an action, indent its bullet as \
                         much as the other actions"
                    ),
                ));
            }
        }
    }
    let error = |message| ParseError::new(line, message);
    match agent_actor {
        Actor::AgentTool {
            agent,
            params,
            profile,
            output_schema,
            budget_block,
            judge_profile,
        } => {
            let resolved = profile::resolve(*agent, params, &blocks.profiles).map_err(error)?;
            if let Some(effort) = &resolved.effort {
                warnings.push(ParseWarning {
                    line,
                    message: format!(
                        "model profile `{}` sets `effort={effort}`, but `agent:{agent}` \
                         ignores it. Only `{CLAUDE_AGENT}` uses `effort=`",
                        resolved.name
                    ),
                });
            }
            *profile = Some(resolved);
            *output_schema = schema::resolve(params, &blocks.schemas).map_err(error)?;
            // Only an `autonomous` agent takes `budget=`. For another agent,
            // `AgentConfig::check` names the option.
            if *agent == AgentType::Autonomous {
                *budget_block = budget_block::resolve(params, &blocks.budgets).map_err(error)?;
            }
            let merged = match budget_block {
                Some(block) => block.merge(params),
                None => params.clone(),
            };
            // `judge_model=` can come from the budget block. A `coder`
            // agent takes it with `on_ask=judge`. For a `summarizer`,
            // `AgentConfig::check` names the option.
            if *agent != AgentType::Summarizer
                && let Some(name) = merged.iter().find_map(|p| p.strip_prefix("judge_model="))
            {
                let judge = profile::find(name, &blocks.profiles).map_err(error)?;
                if let Some(effort) = &judge.effort {
                    warnings.push(ParseWarning {
                        line,
                        message: format!(
                            "model profile `{}` sets `effort={effort}`, but the judge of \
                             `agent:{agent}` ignores it. Only `{CLAUDE_AGENT}` uses `effort=`",
                            judge.name
                        ),
                    });
                }
                *judge_profile = Some(Box::new(judge));
            }
            AgentConfig::check(*agent, &merged, profile.as_ref()).map_err(error)?;
        }
        Actor::ClaudeAgent {
            params,
            profile,
            output_schema,
            environment,
            default_name,
            judge_profile,
        } => {
            // Without `model=`, `ClaudeAgentConfig::check` names the option.
            if let Some(name) = params.iter().find_map(|p| p.strip_prefix("model=")) {
                *profile = Some(profile::find(name, &blocks.profiles).map_err(error)?);
            }
            // The judge of `on_ask=judge` is the judge of an `autonomous`
            // agent, so its profile is an OpenAI compatible one.
            if let Some(name) = params.iter().find_map(|p| p.strip_prefix("judge_model=")) {
                let judge = profile::find(name, &blocks.profiles).map_err(error)?;
                if let Some(effort) = &judge.effort {
                    warnings.push(ParseWarning {
                        line,
                        message: format!(
                            "model profile `{}` sets `effort={effort}`, but the judge of \
                             `{CLAUDE_AGENT}` ignores it",
                            judge.name
                        ),
                    });
                }
                *judge_profile = Some(Box::new(judge));
            }
            *output_schema = schema::resolve(params, &blocks.schemas).map_err(error)?;
            // An unknown block name: `ClaudeAgentConfig::check` names it.
            *environment = params
                .iter()
                .find_map(|p| p.strip_prefix("environment="))
                .and_then(|name| blocks.envs.get(name))
                .cloned();
            *default_name = place.agent_name();
            ClaudeAgentConfig::check(params, profile.as_ref(), environment.as_ref())
                .map_err(error)?;
        }
        _ => {}
    }
    Ok(actor)
}

/// Sets `params` of an agent action from the list of its `options`. Each
/// item of `options` is its line number and its text.
///
/// Fails if the line of the action has options, because they must be in
/// the list, or an option has text after its closing quote.
fn take_options(
    action: &str,
    params: &mut Vec<String>,
    options: &[(usize, String)],
    line: usize,
) -> Result<(), ParseError> {
    if !params.is_empty() {
        return Err(ParseError::new(
            line,
            format!(
                "put the options of `{action}` in a list under the action, one \
                 `- <key>=<value>` bullet on each line, as in `- model=<profile>`"
            ),
        ));
    }
    *params = options
        .iter()
        .map(|(line, text)| unquote_option(text).map_err(|m| ParseError::new(*line, m)))
        .collect::<Result<_, _>>()?;
    Ok(())
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

/// The indentation of a line in columns. A tab is 4 columns, as in
/// Markdown, and any other whitespace character is 1 column.
fn indentation(line: &str) -> usize {
    line.chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum()
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
        "begin_if" | "skip_if" | "actions" | "actions-parallel" | "if_fail" | "if_succeed"
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
            CLAUDE_AGENT => Ok(Actor::ClaudeAgent {
                params: split_quoted(rest)?,
                profile: None,
                output_schema: None,
                environment: None,
                default_name: String::new(),
                judge_profile: None,
            }),
            other => match other.strip_prefix("agent:") {
                Some(agent) => Ok(Actor::AgentTool {
                    agent: agent.parse()?,
                    params: split_quoted(rest)?,
                    profile: None,
                    output_schema: None,
                    budget_block: None,
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

/// Reads one item of the list of options of an agent action. If the value
/// after `=` is all in double quotes, the quotes go away, with the same rules
/// as in [`split_quoted`]. `max_output="40%"` becomes `max_output=40%`. Any
/// other text does not change, so a value can have spaces without quotes.
///
/// Fails if the value has text after its closing quote.
fn unquote_option(text: &str) -> Result<String, String> {
    let text = text.trim();
    let Some((key, value)) = text.split_once('=') else {
        return Ok(text.to_string());
    };
    if !(value.len() >= 2 && value.starts_with('"') && value.ends_with('"')) {
        return Ok(text.to_string());
    }
    match split_quoted(value)?.as_slice() {
        [value] => Ok(format!("{key}={value}")),
        _ => Err(format!(
            "the value of `{key}` has text after its closing quote, in `{text}`"
        )),
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
                "unknown agent type `{s}`, expected `agent:coder`, `agent:summarizer`, \
                 `agent:autonomous` or `agent:claude-agent`"
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

impl FromStr for SkipIf {
    type Err = String;

    /// Reads `exist(<path>)` or `exists-nonempty(<path>)`. The path is trimmed and must not be empty.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let (name, rest) = s
            .split_once('(')
            .ok_or_else(|| format!("unknown `skip_if` condition `{s}`, {SKIP_IF_FORMS}"))?;
        let arg = rest
            .strip_suffix(')')
            .ok_or_else(|| format!("missing `)` in `skip_if` condition `{s}`"))?
            .trim();
        let name = name.trim();
        let condition: fn(String) -> SkipIf = match name {
            "exist" => SkipIf::Exist,
            "exists-nonempty" => SkipIf::ExistsNonEmpty,
            other => {
                return Err(format!(
                    "unknown `skip_if` condition `{other}`, {SKIP_IF_FORMS}"
                ));
            }
        };
        if arg.is_empty() {
            return Err(format!("`{name}()` needs a file name"));
        }
        Ok(condition(arg.to_string()))
    }
}

/// The end of the error message for an unknown `skip_if` condition.
const SKIP_IF_FORMS: &str = "expected `exist(<file>)` or `exists-nonempty(<file>)`";
