//! Loop blocks: the configuration of an `autonomous` agent in a fenced
//! block of the job file, so that criteria and guardrails can have more
//! than one line, and the job can select tools and their limits.
//!
//! A ```` ```loop:<name> ```` block comes before the `# <name>` heading. An
//! `agent:autonomous` action selects it with `loop=<name>`. Each line is
//! `<key>: <value>`. A key with no value starts a list, and each list item
//! is a `- <item>` line after it:
//!
//! ````markdown
//! ```loop:fix-tests
//! max_time: 20m
//! success_check: cargo test --quiet
//! guardrails:
//!   - Do not change files in tests/.
//!   - Do not run git push.
//! tools:
//!   - read_file
//!   - bash max_calls=40 max_failures=5
//! ```
//! ````
//!
//! The keys are options of the action (see [`BLOCK_KEYS`]), and `tools`. A
//! list of text, such as `guardrails:`, becomes one value with one
//! `- <item>` line for each item. An option of the action replaces the same
//! option of the block (see [`LoopBlock::merge`]).

use std::collections::BTreeMap;

use crate::agent_loop_openai_compat::Tool;
use crate::agent_loop_openai_compat::tools::{
    DEFAULT_MAX_CALLS, DEFAULT_MAX_FAILURES, Limited, TOOL_NAMES, tool_by_name,
};
use crate::schema::{MAX_NAME_CHARS, is_name};

/// The options of an action that a loop block can set, besides `tools`.
pub const BLOCK_KEYS: [&str; 13] = [
    "task",
    "max_steps",
    "token_budget",
    "max_context",
    "thinking",
    "max_iterations",
    "max_time",
    "success_check",
    "success",
    "guardrails",
    "guard",
    "max_violations",
    "judge_model",
];

/// The loop blocks of a job, by name.
pub type LoopBlocks = BTreeMap<String, LoopBlock>;

/// One ```` ```loop:<name> ```` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopBlock {
    pub name: String,
    /// The options of the block as `key=value` params, in order.
    pub params: Vec<String>,
    /// The tools from `tools:`, or `None` for the default tools.
    pub tools: Option<Vec<ToolLimits>>,
}

/// One item of `tools:`: a tool name and its limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolLimits {
    pub name: String,
    /// From `max_calls=<n>`. The default is [`DEFAULT_MAX_CALLS`].
    pub max_calls: u32,
    /// From `max_failures=<n>`. The default is [`DEFAULT_MAX_FAILURES`].
    pub max_failures: u32,
}

/// The name of a loop block from its opening fence, as in `fix-tests` from
/// ```` ```loop:fix-tests ````. Returns `None` if the line is not such a
/// fence.
pub fn block_name(line: &str) -> Option<&str> {
    line.trim().strip_prefix("```loop:").map(str::trim)
}

/// Makes sure that `name`, from a ```` ```loop:<name> ```` fence, can name
/// a loop block.
pub fn check_name(name: &str) -> Result<(), String> {
    if is_name(name) {
        Ok(())
    } else {
        Err(format!(
            "`{name}` is not a loop block name, use 1 to {MAX_NAME_CHARS} letters, digits, \
             `_` or `-`, as in ```loop:fix-tests"
        ))
    }
}

/// Reads the lines of the block `name`.
///
/// Fails with the index of the line and a message if a key is unknown or
/// set twice, a value is invalid, an item does not follow a key with no
/// value, a key has no value and no items, or a tool is unknown.
pub fn parse_block(name: &str, lines: &[&str]) -> Result<LoopBlock, (usize, String)> {
    // Each key, its index of line, its value, and the items of its list.
    let mut entries: Vec<(&str, usize, &str, Vec<&str>)> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(item) = line.strip_prefix("- ") {
            match entries.last_mut() {
                Some((_, _, "", items)) => items.push(item.trim()),
                _ => {
                    return Err((
                        index,
                        format!(
                            "the item `{line}` needs a key with no value before it, as in \
                             `guardrails:`"
                        ),
                    ));
                }
            }
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            return Err((index, format!("expected `<key>: <value>`, found `{line}`")));
        };
        let key = key.trim();
        if key != "tools" && !BLOCK_KEYS.contains(&key) {
            let keys: Vec<String> = BLOCK_KEYS.iter().map(|key| format!("`{key}`")).collect();
            return Err((
                index,
                format!(
                    "unknown key `{key}` in loop block `{name}`, expected {} or `tools`",
                    keys.join(", ")
                ),
            ));
        }
        if entries.iter().any(|(other, ..)| *other == key) {
            return Err((index, format!("key `{key}` is set twice")));
        }
        entries.push((key, index, value.trim(), Vec::new()));
    }

    let mut block = LoopBlock {
        name: name.to_string(),
        params: Vec::new(),
        tools: None,
    };
    for (key, index, value, items) in entries {
        if value.is_empty() && items.is_empty() {
            return Err((index, format!("key `{key}` has no value and no items")));
        }
        if key == "tools" {
            let tools: Result<Vec<_>, String> = items.iter().map(|item| parse_tool(item)).collect();
            block.tools = Some(tools.map_err(|message| (index, message))?);
            continue;
        }
        let value = if items.is_empty() {
            value.to_string()
        } else {
            let lines: Vec<String> = items.iter().map(|item| format!("- {item}")).collect();
            lines.join("\n")
        };
        block.params.push(format!("{key}={value}"));
    }
    Ok(block)
}

/// Reads one item of `tools:`: `<name> [max_calls=<n>] [max_failures=<n>]`.
fn parse_tool(item: &str) -> Result<ToolLimits, String> {
    let mut words = item.split_whitespace();
    let name = words.next().unwrap_or_default();
    if !TOOL_NAMES.contains(&name) {
        let names: Vec<String> = TOOL_NAMES.iter().map(|name| format!("`{name}`")).collect();
        return Err(format!(
            "unknown tool `{name}`, expected {}",
            names.join(", ")
        ));
    }
    let mut tool = ToolLimits {
        name: name.to_string(),
        max_calls: DEFAULT_MAX_CALLS,
        max_failures: DEFAULT_MAX_FAILURES,
    };
    for word in words {
        let (key, value) = word.split_once('=').unwrap_or((word, ""));
        let limit = match key {
            "max_calls" => &mut tool.max_calls,
            "max_failures" => &mut tool.max_failures,
            _ => {
                return Err(format!(
                    "unknown limit `{word}` of tool `{name}`, expected `max_calls=<n>` or \
                     `max_failures=<n>`"
                ));
            }
        };
        *limit = value
            .parse()
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| format!("`{word}` of tool `{name}` is not a positive number"))?;
    }
    Ok(tool)
}

impl LoopBlock {
    /// The params of the block and of the action, for one action. An option
    /// of the action replaces the same option of the block. The options of
    /// the block come first.
    pub fn merge(&self, action: &[String]) -> Vec<String> {
        let key = |param: &str| param.split_once('=').map(|(key, _)| key.to_string());
        let action_keys: Vec<Option<String>> = action.iter().map(|param| key(param)).collect();
        self.params
            .iter()
            .filter(|param| !action_keys.contains(&key(param)))
            .chain(action)
            .cloned()
            .collect()
    }

    /// The tools of `tools:`, each one in a [`Limited`] with its limits, or
    /// `None` if the block has no `tools:`.
    pub fn make_tools(&self) -> Option<Vec<Box<dyn Tool>>> {
        let tools = self.tools.as_ref()?;
        let tools = tools.iter().map(|limits| -> Box<dyn Tool> {
            let tool = tool_by_name(&limits.name).expect("parse_tool accepts only known tools");
            Box::new(Limited::new(
                tool,
                Some(limits.max_calls),
                Some(limits.max_failures),
            ))
        });
        Some(tools.collect())
    }
}

/// The name in the `loop=<name>` param of an action, if it has one.
pub fn loop_name(params: &[String]) -> Option<&str> {
    params.iter().find_map(|param| param.strip_prefix("loop="))
}

/// Finds the block that the `loop=<name>` param of an action names.
/// Returns `None` if the action has no `loop=`.
///
/// Fails if `blocks` has no block with that name.
pub fn resolve(params: &[String], blocks: &LoopBlocks) -> Result<Option<LoopBlock>, String> {
    let Some(name) = loop_name(params) else {
        return Ok(None);
    };
    blocks
        .get(name)
        .cloned()
        .map(Some)
        .ok_or_else(|| format!("loop block `{name}` is not defined in the job"))
}
