//! Model profiles: named model settings that agent actions use.
//!
//! A job file can start with a ```` ```config ```` block, before the
//! `# <job name>` heading. Each line of the block defines one profile:
//!
//! ```text
//! local-qwen: model=qwen3.5:0.8b, base_url=http://localhost:11434/v1
//! openai-gpt5: model=gpt-5, base_url=https://api.openai.com/v1, api_key=OPENAI_API_KEY
//! ```
//!
//! An agent action then selects a profile by its name, as in
//! `agent:coder model=local-qwen`. The job file never holds an API key:
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
}

/// Reads one profile line: `<name>: model=<id>, base_url=<url>[, api_key=<ENV_VAR>]`.
///
/// Fails if the line has no `:`, the name is empty or has whitespace, a
/// setting is not `key=value`, a key is unknown, empty or set twice, or
/// `model` or `base_url` is missing.
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
                other => {
                    return Err(format!(
                        "unknown setting `{other}` in profile `{name}`, expected `model=`, \
                         `base_url=` or `api_key=`"
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
        Ok(ModelProfile {
            name: name.to_string(),
            model: model.ok_or_else(|| missing("model"))?,
            base_url: base_url.ok_or_else(|| missing("base_url"))?,
            api_key_env,
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
