//! Output schemas: JSON schemas that the final reply of an agent must match.
//!
//! An agent action takes `output_schema=<value>`. The value is the name of a
//! schema block of the job, or the path of a JSON schema file:
//!
//! - A value that has only ASCII letters, digits, `_` and `-` is a name, as
//!   in `output_schema=article`. The job must define a schema block with that
//!   name. The parser makes sure of this (see [`resolve`]).
//! - Any other value is a path, as in `output_schema=schemas/article.json`
//!   or `output_schema=./article`. The agent reads the file when it runs, so
//!   that an earlier task can write it (see [`load`]).
//!
//! A schema block comes before the `# <job name>` heading, as the
//! ```` ```config ```` block does. Its fence names the schema:
//!
//! ````text
//! ```json:article
//! {"type": "object", "properties": {"title": {"type": "string"}}}
//! ```
//! ````
//!
//! Each schema must be a JSON object and a valid JSON schema (see
//! [`check_schema`]).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{Value, json};

/// Most characters in the name of a schema. OpenAI has the same limit for
/// the name in `response_format`.
pub const MAX_NAME_CHARS: usize = 64;

/// Most validation errors of one reply that the model gets.
const MAX_REPLY_ERRORS: usize = 5;

/// A JSON schema for the final reply of an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputSchema {
    /// The name of the schema block, or a name made from the file name.
    /// It is a valid [`is_name`] value.
    pub name: String,
    /// The JSON schema.
    pub schema: Value,
}

/// The schema blocks of a job, by name.
pub type Schemas = BTreeMap<String, OutputSchema>;

/// Returns `true` if `value` is the name of a schema block, and `false` if
/// it is a path. A name has 1 to [`MAX_NAME_CHARS`] characters, and each
/// one is an ASCII letter, a digit, `_` or `-`.
pub fn is_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_NAME_CHARS
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The name of a schema block from its opening fence, as in `article` from
/// ```` ```json:article ````. Returns `None` if the line is not such a fence.
/// The name is trimmed, and it can be empty or invalid.
pub fn block_name(line: &str) -> Option<&str> {
    line.trim().strip_prefix("```json:").map(str::trim)
}

/// Makes sure that `name`, from a ```` ```json:<name> ```` fence, can name a
/// schema block.
pub fn check_name(name: &str) -> Result<(), String> {
    if is_name(name) {
        return Ok(());
    }
    if name.is_empty() {
        return Err("the ```json:<name> block has no name, as in ```json:article".to_string());
    }
    Err(format!(
        "`{name}` is not a schema name, use 1 to {MAX_NAME_CHARS} letters, digits, `_` or `-`"
    ))
}

/// Makes sure that `schema` is a JSON object and a valid JSON schema. The
/// `$schema` keyword of the schema selects the draft, and the default is
/// draft 2020-12. `what` names the schema in the error.
pub fn check_schema(schema: &Value, what: &str) -> Result<(), String> {
    if !schema.is_object() {
        return Err(format!(
            "{what} is not a JSON object, as in `{{\"type\": \"object\"}}`"
        ));
    }
    jsonschema::validator_for(schema)
        .map(|_| ())
        .map_err(|err| {
            let path = err.instance_path().to_string();
            let at = if path.is_empty() {
                String::new()
            } else {
                format!(" at `{path}`")
            };
            format!("{what} is not a valid JSON schema{at}: {err}")
        })
}

/// Reads the text of the schema block `name`.
///
/// Fails if the text is not JSON, or not a valid schema (see
/// [`check_schema`]). The error has the 0-based index of the line in the
/// text where the JSON error is, or 0 for an invalid schema.
pub fn parse_block(name: &str, text: &str) -> Result<OutputSchema, (usize, String)> {
    let what = format!("schema `{name}`");
    let schema: Value = serde_json::from_str(text).map_err(|err| {
        (
            err.line().saturating_sub(1),
            format!("{what} is not valid JSON: {}", json_error(&err)),
        )
    })?;
    check_schema(&schema, &what).map_err(|message| (0, message))?;
    Ok(OutputSchema {
        name: name.to_string(),
        schema,
    })
}

/// The text of a JSON error without its position, because the caller
/// gives the line of the job file.
fn json_error(err: &serde_json::Error) -> String {
    let text = err.to_string();
    let position = format!(" at line {} column {}", err.line(), err.column());
    let message = text.strip_suffix(&position).unwrap_or(&text);
    format!("{message} (column {})", err.column())
}

/// The value of the `output_schema=` param, if the params have one.
fn param_value(params: &[String]) -> Option<&str> {
    params
        .iter()
        .find_map(|param| param.strip_prefix("output_schema="))
}

/// Finds the schema block that the `output_schema=<name>` param of an agent
/// action names. Returns `None` if the params have no `output_schema=`, or
/// its value is a path.
///
/// Fails if the value is a name and `schemas` has no schema with that name.
pub fn resolve(params: &[String], schemas: &Schemas) -> Result<Option<OutputSchema>, String> {
    let Some(value) = param_value(params).filter(|value| is_name(value)) else {
        return Ok(None);
    };
    if let Some(schema) = schemas.get(value) {
        return Ok(Some(schema.clone()));
    }
    if schemas.is_empty() {
        return Err(format!(
            "unknown output schema `{value}`: the job has no ```json:<name> schema block. \
             For a file, use a path, as in `output_schema=./{value}.json`"
        ));
    }
    let names: Vec<String> = schemas.keys().map(|name| format!("`{name}`")).collect();
    Err(format!(
        "unknown output schema `{value}`, expected {} or the path of a file",
        names.join(", ")
    ))
}

/// The schema that `output_schema=<value>` selects when the agent runs.
/// `block` is the schema block that the parser found for the action (see
/// [`resolve`]).
///
/// Fails if the value is a name and `block` does not have that name, or the
/// value is a path and the file cannot be read, is not JSON, or is not a
/// valid schema.
pub fn load(value: &str, block: Option<&OutputSchema>) -> Result<OutputSchema, String> {
    if is_name(value) {
        return block
            .filter(|block| block.name == value)
            .cloned()
            .ok_or_else(|| format!("output schema `{value}` is not defined in the job"));
    }
    OutputSchema::from_file(value)
}

impl OutputSchema {
    /// Reads the schema in the file at `path`. Its name is the file name
    /// without the extension, with `_` for each character that a name
    /// cannot have.
    ///
    /// Fails if the file cannot be read, is not JSON, or is not a valid
    /// schema.
    pub fn from_file(path: &str) -> Result<Self, String> {
        let text = fs::read_to_string(path)
            .map_err(|err| format!("cannot read output schema file `{path}`: {err}"))?;
        let what = format!("output schema file `{path}`");
        let schema: Value = serde_json::from_str(&text)
            .map_err(|err| format!("{what} is not valid JSON: {err}"))?;
        check_schema(&schema, &what)?;
        Ok(OutputSchema {
            name: name_from_path(path),
            schema,
        })
    }

    /// The `response_format` of a chat completions request, which asks the
    /// server for a reply that matches the schema. It has no `strict`, so
    /// a server that only takes strict schemas does not refuse the schema.
    pub fn response_format(&self) -> Value {
        json!({
            "type": "json_schema",
            "json_schema": {"name": self.name, "schema": self.schema},
        })
    }

    /// The instruction for the system prompt. A server that ignores
    /// `response_format` still tells the model the format in this way.
    pub fn instruction(&self) -> String {
        let schema =
            serde_json::to_string_pretty(&self.schema).expect("JSON values always serialize");
        format!(
            "Your final reply, without tool calls, must be only one JSON value that \
             matches this JSON schema. Do not add other text or code fences.\n{schema}"
        )
    }

    /// Reads the final reply of the model as JSON that matches the schema,
    /// and returns the JSON text. A code fence around the JSON, as in
    /// ```` ```json ````, is removed.
    ///
    /// Fails if the reply is not JSON or does not match the schema. The
    /// error has at most [`MAX_REPLY_ERRORS`] validation errors.
    pub fn check_reply(&self, reply: &str) -> Result<String, String> {
        let text = strip_fence(reply);
        let value: Value = serde_json::from_str(text)
            .map_err(|err| format!("the reply is not valid JSON: {err}"))?;
        let validator =
            jsonschema::validator_for(&self.schema).expect("check_schema accepted the schema");
        let errors: Vec<String> = validator
            .iter_errors(&value)
            .take(MAX_REPLY_ERRORS)
            .map(|err| {
                let path = err.instance_path().to_string();
                let path = if path.is_empty() {
                    "/".to_string()
                } else {
                    path
                };
                format!("at `{path}`: {err}")
            })
            .collect();
        if errors.is_empty() {
            Ok(text.to_string())
        } else {
            Err(format!(
                "the reply does not match the schema:\n{}",
                errors.join("\n")
            ))
        }
    }
}

/// The text without a code fence around it, trimmed.
pub fn strip_fence(text: &str) -> &str {
    let text = text.trim();
    let Some(rest) = text.strip_prefix("```") else {
        return text;
    };
    // The rest of the fence line is a language, as in `json`.
    let body = rest.split_once('\n').map_or("", |(_, body)| body);
    body.trim_end().strip_suffix("```").unwrap_or(body).trim()
}

/// A schema name from the file name of `path`, as in `article` from
/// `schemas/article.json`.
fn name_from_path(path: &str) -> String {
    let stem = Path::new(path)
        .file_stem()
        .map_or("output".into(), |stem| stem.to_string_lossy());
    stem.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(MAX_NAME_CHARS)
        .collect()
}
