//! Parser for job config markdown files.
//!
//! The format is line based:
//!
//! - `# <name>` names the job; text after it is the job description.
//! - `## <id>` starts a task; its heading text is the task id.
//! - Top-level `- begin_if:`, `- actions:`, `- if_fail:` and `- if_succeed:`
//!   bullets in a task are fields. Indented bullets under `- actions:` are the
//!   actions. Any other text is part of the task description.
//! - Lines inside ``` code fences are always treated as description text.

use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use crate::job::{Event, Hook, Job, Task, Trigger};

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
            if let Some(id) = line.strip_prefix("## ") {
                let id = id.trim();
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

            if let Some(job_name) = line.strip_prefix("# ") {
                if name.is_some() {
                    return Err(ParseError::new(line_no, "more than one `# <job name>` heading"));
                }
                let job_name = job_name.trim();
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
                current.task.actions.push(action.trim().to_string());
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
                        current.task.actions.push(value.to_string());
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
                return Err(ParseError::new(line, format!("task `{id}` depends on itself")));
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
    value.parse().map_err(|message| ParseError::new(line, message))
}

/// Joins description lines and trims surrounding blank lines.
fn join_text(lines: &[String]) -> String {
    lines.join("\n").trim().to_string()
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
