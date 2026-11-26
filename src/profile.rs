//! Model profiles: named model settings that agent actions use.
//!
//! A job file can start with a ```` ```config ```` block, before the
//! `# <job name>` heading. Each line of the block defines one profile:
//!
//! ```text
//! local-qwen: model=qwen3.5:0.8b, base_url=http://localhost:11434/v1
//! openai-gpt5: model=gpt-5, base_url=https://api.openai.com/v1, api_key=OPENAI_API_KEY
//! small-budget: model=qwen3.5:0.8b, base_url=http://localhost:11434/v1, token_budget=50000
//! openai-luna: model=gpt-6-luna, base_url=https://api.openai.com/v1, api_key=OPENAI_API_KEY, thinking=false
//! ```
//!
//! An agent action then selects a profile by its name, with the option
//! `- model=local-qwen` in its list. The job file never holds an API key:
//! `api_key=` is the name of the environment variable that holds the key.

use std::collections::BTreeMap;
use std::str::FromStr;

use crate::actor::AgentType;

/// The settings of one model, from one line of the ```` ```config ```` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelProfile {
    /// The name that agent actions use, as in `model=<name>`.
    pub name: String,
    /// The model identifier that the server knows, from `model=<id>`.
    pub model: String,
    /// The base URL of the chat API, from `base_url=<url>`.
    pub base_url: String,
    /// The name of the environment variable with the API key, from
    /// `api_key=<ENV_VAR>`. `None` if the server needs no key.
    pub api_key_env: Option<String>,
    /// The most tokens that one run of an agent loop can use, from
    /// `token_budget=<n>`. The `token_budget=` option of an agent action
    /// replaces it. `None` sets no limit.
    pub token_budget: Option<u64>,
    /// From `thinking=<true|false>`. The `thinking=` option of an agent
    /// action replaces it. `Some(false)` sends `"reasoning_effort": "none"`,
    /// for example for a model that takes tools only without reasoning.
    /// `None` sends nothing, and the server uses its default.
    pub thinking: Option<bool>,
    /// The effort level of a Claude model, from `effort=<level>`, as in
    /// `high` (see [`EFFORT_LEVELS`]). Only a `claude-agent` action uses it.
    /// The parser warns if another agent uses a profile with `effort=`.
    /// `None` uses the default of the model.
    pub effort: Option<String>,
}

/// The values of `effort=`.
pub const EFFORT_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// Reads the value of `effort=`, one of [`EFFORT_LEVELS`].
///
/// Fails on any other value.
pub fn parse_effort(value: &str) -> Result<String, String> {
    if EFFORT_LEVELS.contains(&value) {
        Ok(value.to_string())
    } else {
        Err(format!(
            "`effort={value}` is not `low`, `medium`, `high`, `xhigh` or `max`"
        ))
    }
}

/// Reads the value of `thinking=`, `true` or `false`.
///
/// Fails on any other value.
pub fn parse_thinking(value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("`thinking={value}` is not `true` or `false`")),
    }
}

/// Reads the value of `token_budget=`, a positive number of tokens. An
/// `_` between two digits separates groups of digits, as in `100_000`.
///
/// Fails if `value` is not a positive whole number, or an `_` is not
/// between two digits.
pub fn parse_token_budget(value: &str) -> Result<u64, String> {
    let error = || format!("`token_budget={value}` is not a positive number");
    let bytes = value.as_bytes();
    let separators_ok = bytes.iter().enumerate().all(|(index, byte)| {
        *byte != b'_'
            || (index > 0
                && bytes[index - 1].is_ascii_digit()
                && bytes.get(index + 1).is_some_and(u8::is_ascii_digit))
    });
    if !separators_ok {
        return Err(error());
    }
    value
        .replace('_', "")
        .parse()
        .ok()
        .filter(|tokens| *tokens > 0)
        .ok_or_else(error)
}

/// Reads one profile line:
/// `<name>: model=<id>, base_url=<url>[, api_key=<ENV_VAR>][, token_budget=<n>][, thinking=<true|false>][, effort=<level>]`.
///
/// Fails if the line has no `:`, the name is empty or has whitespace, a
/// setting is not `key=value`, a key is unknown, empty or set twice, or
/// `model` or `base_url` is missing, `token_budget` is not a positive
/// number, `thinking` is not `true` or `false`, or `effort` is not one of
/// [`EFFORT_LEVELS`].
impl FromStr for ModelProfile {
    type Err = String;

    fn from_str(line: &str) -> Result<Self, Self::Err> {
        let (name, settings) = line.split_once(':').ok_or_else(|| {
            format!("`{line}` is not a profile, expected `<name>: model=<id>, base_url=<url>`")
        })?;
        let name = name.trim();
        if name.is_empty() || name.contains(char::is_whitespace) {
            return Err(format!(
                "`{name}` is not a profile name, use one word as in `local-qwen`"
            ));
        }
        let mut model = None;
        let mut base_url = None;
        let mut api_key_env = None;
        let mut token_budget = None;
        let mut thinking = None;
        let mut effort = None;
        for setting in settings.split(',') {
            let setting = setting.trim();
            let Some((key, value)) = setting.split_once('=') else {
                return Err(format!(
                    "unexpected `{setting}` in profile `{name}`, expected `key=value`"
                ));
            };
            let slot = match key.trim() {
                "model" => &mut model,
                "base_url" => &mut base_url,
                "api_key" => &mut api_key_env,
                "token_budget" => &mut token_budget,
                "thinking" => &mut thinking,
                "effort" => &mut effort,
                other => {
                    return Err(format!(
                        "unknown setting `{other}` in profile `{name}`, expected `model=`, \
                         `base_url=`, `api_key=`, `token_budget=`, `thinking=` or `effort=`"
                    ));
                }
            };
            let value = value.trim();
            if value.is_empty() {
                return Err(format!(
                    "setting `{}` in profile `{name}` has no value",
                    key.trim()
                ));
            }
            if slot.replace(value.to_string()).is_some() {
                return Err(format!(
                    "setting `{}` in profile `{name}` is set twice",
                    key.trim()
                ));
            }
        }
        let missing = |key: &str| format!("profile `{name}` needs `{key}=`");
        let token_budget = token_budget
            .as_deref()
            .map(parse_token_budget)
            .transpose()
            .map_err(|err| format!("{err} in profile `{name}`"))?;
        let thinking = thinking
            .as_deref()
            .map(parse_thinking)
            .transpose()
            .map_err(|err| format!("{err} in profile `{name}`"))?;
        let effort = effort
            .as_deref()
            .map(parse_effort)
            .transpose()
            .map_err(|err| format!("{err} in profile `{name}`"))?;
        Ok(ModelProfile {
            name: name.to_string(),
            model: model.ok_or_else(|| missing("model"))?,
            base_url: base_url.ok_or_else(|| missing("base_url"))?,
            api_key_env,
            token_budget,
            thinking,
            effort,
        })
    }
}

/// The profiles of a job, by name.
pub type Profiles = BTreeMap<String, ModelProfile>;

/// Reads the lines of a ```` ```config ```` block into profiles. Blank lines
/// are skipped. Each error has the index of its line in `lines`.
///
/// Fails if a line is not a valid profile, or two lines define the same name.
pub fn parse_profiles<'a>(
    lines: impl IntoIterator<Item = &'a str>,
) -> Result<Profiles, (usize, String)> {
    let mut profiles = Profiles::new();
    for (index, line) in lines.into_iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let profile: ModelProfile = line.trim().parse().map_err(|err| (index, err))?;
        if profiles.contains_key(&profile.name) {
            return Err((
                index,
                format!("profile `{}` is defined twice", profile.name),
            ));
        }
        profiles.insert(profile.name.clone(), profile);
    }
    Ok(profiles)
}

/// The error for an agent action without `model=`.
pub fn missing_model(agent: AgentType) -> String {
    format!(
        "`agent:{agent}` needs `model=<profile>`, the name of a model profile in the \
         ```config block at the start of the job"
    )
}

/// Finds the profile that the `model=<profile>` param of an `agent` action
/// names.
///
/// Fails if the params have no `model=`, or `profiles` has no profile with
/// that name.
pub fn resolve(
    agent: AgentType,
    params: &[String],
    profiles: &Profiles,
) -> Result<ModelProfile, String> {
    let name = params
        .iter()
        .find_map(|param| param.strip_prefix("model="))
        .ok_or_else(|| missing_model(agent))?;
    find(name, profiles)
}

/// Finds the profile `name` in `profiles`, for `model=<name>` or
/// `judge_model=<name>`.
///
/// Fails if `profiles` has no profile with that name.
pub fn find(name: &str, profiles: &Profiles) -> Result<ModelProfile, String> {
    if let Some(profile) = profiles.get(name) {
        return Ok(profile.clone());
    }
    if profiles.is_empty() {
        return Err(format!(
            "unknown model profile `{name}`: the job has no ```config block with model profiles"
        ));
    }
    let names: Vec<String> = profiles.keys().map(|name| format!("`{name}`")).collect();
    Err(format!(
        "unknown model profile `{name}`, expected {}",
        names.join(", ")
    ))
}
