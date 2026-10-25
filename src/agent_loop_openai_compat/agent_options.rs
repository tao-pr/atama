//! The options of an `agent:<type>` action, and the configuration of one
//! run of the agent loop that the options make.
//!
//! [`AgentConfig::from_params`] reads the options of an agent. It reads the
//! options of a `summarizer` agent into [`SummarizerOptions`].

use crate::actor::AgentType;

/// Base URL of the chat API if the action and the environment set none.
pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
/// Most model replies that one run of the loop asks for.
pub const DEFAULT_MAX_STEPS: u32 = 10;
/// Environment variable with the base URL of the chat API.
pub const BASE_URL_ENV: &str = "ATAMA_AGENT_BASE_URL";
/// Environment variable with the model identifier.
pub const MODEL_ENV: &str = "ATAMA_AGENT_MODEL";
/// Environment variable with the API key. Local servers often need none.
pub const API_KEY_ENV: &str = "ATAMA_AGENT_API_KEY";
/// Expertise of a `summarizer` agent if the action has no `expertise=`.
pub const DEFAULT_EXPERTISE: &str = "general";
/// Instruction of a `summarizer` agent if the action has no `task=`.
pub const DEFAULT_SUMMARIZER_PROMPT: &str = "Summarize the input.";

/// Options of an agent action.
const OPTION_KEYS: [&str; 6] = [
    "base_url",
    "model",
    "max_steps",
    "task",
    "expertise",
    "max_output",
];

/// The configuration of one run, read from the action's params and the
/// environment.
///
/// Each param is one option. A param `task=<text>`, `model=<id>`,
/// `base_url=<url>` or `max_steps=<n>` sets that option for every agent
/// type. A `summarizer` agent also takes
/// `expertise=<text>` and `max_output=<size>` (see [`SummarizerOptions`]).
/// In the job file, a value can be in double quotes, so that it can have
/// spaces, as in `task="Fix the tests"`. The parser removes the quotes. If
/// the params do not set `model` or `base_url`, the environment variables
/// [`MODEL_ENV`] and [`BASE_URL_ENV`] set them. The API key comes
/// only from [`API_KEY_ENV`], so it is never in the job file or the log.
pub struct AgentConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub max_steps: u32,
    /// The options of a `summarizer` agent. `None` for other agent types.
    pub summarizer: Option<SummarizerOptions>,
    /// The task, the instruction for the model.
    pub task: String,
}

/// The options of a `summarizer` agent: the field of the reader, and how
/// long the summary can be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummarizerOptions {
    /// The field whose terms and point of view the summary uses, from
    /// `expertise=<text>`. The default is [`DEFAULT_EXPERTISE`].
    // TAOTODO: load domain expertise as skills, for example a file with the
    // terms, rules and examples of a field, instead of only naming the field.
    pub expertise: String,
    /// The most that the summary can be, from `max_output=<size>`. `None`
    /// lets the model choose.
    pub max_output: Option<MaxOutput>,
}

/// How long the summary of a `summarizer` agent can be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaxOutput {
    /// A part of the input, in percent from 1 to 100, as in `60%`.
    Percent(u32),
    /// A number of sentences, as in `3 sentences`.
    Sentences(u32),
    /// A number of words, as in `100 words`.
    Words(u32),
    /// A number of paragraphs, as in `2 paragraphs`.
    Paragraphs(u32),
}

impl std::str::FromStr for MaxOutput {
    type Err = String;

    /// Reads `<n>%`, or `<n> sentences`, `<n> words` or `<n> paragraphs`.
    /// The unit can also be singular, as in `1 sentence`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = || {
            format!(
                "`max_output={s}` is not a size, expected a percent from 1 to 100 as in \
                 `60%`, or a number of sentences, words or paragraphs as in `3 sentences`"
            )
        };
        let positive = |n: &str| n.parse().ok().filter(|n| *n > 0);
        if let Some(percent) = s.strip_suffix('%') {
            return positive(percent)
                .filter(|n| *n <= 100)
                .map(MaxOutput::Percent)
                .ok_or_else(invalid);
        }
        let (count, unit) = s.split_once(' ').ok_or_else(invalid)?;
        let count = positive(count).ok_or_else(invalid)?;
        match unit.trim() {
            "sentence" | "sentences" => Ok(MaxOutput::Sentences(count)),
            "word" | "words" => Ok(MaxOutput::Words(count)),
            "paragraph" | "paragraphs" => Ok(MaxOutput::Paragraphs(count)),
            _ => Err(invalid()),
        }
    }
}

/// Writes the size as in the action, for example `60%`, `3 sentences` or
/// `1 sentence`.
impl std::fmt::Display for MaxOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (n, unit) = match *self {
            MaxOutput::Percent(n) => return write!(f, "{n}%"),
            MaxOutput::Sentences(n) => (n, "sentence"),
            MaxOutput::Words(n) => (n, "word"),
            MaxOutput::Paragraphs(n) => (n, "paragraph"),
        };
        let plural = if n == 1 { "" } else { "s" };
        write!(f, "{n} {unit}{plural}")
    }
}

impl SummarizerOptions {
    /// Reads the options from the values of `expertise=` and `max_output=`,
    /// if the params set them.
    ///
    /// Fails if `max_output` is not a size (see [`MaxOutput`]).
    fn from_values(expertise: Option<String>, max_output: Option<String>) -> Result<Self, String> {
        Ok(SummarizerOptions {
            expertise: expertise.unwrap_or_else(|| DEFAULT_EXPERTISE.to_string()),
            max_output: max_output.map(|value| value.parse()).transpose()?,
        })
    }

    /// The task for the model: the instruction, the expertise and the length
    /// limit. The loop adds `input` after it. The instruction is `prompt`, or
    /// [`DEFAULT_SUMMARIZER_PROMPT`] if it is empty. A limit in percent
    /// also gives the number of words that it allows, because a model
    /// cannot measure a percent of a text well.
    ///
    /// Fails if `input` is empty, because there is nothing to summarize.
    pub fn task(&self, prompt: &str, input: &str) -> Result<String, String> {
        let words = input.split_whitespace().count();
        if words == 0 {
            return Err("`agent:summarizer` needs an input to summarize".to_string());
        }
        let prompt = if prompt.is_empty() {
            DEFAULT_SUMMARIZER_PROMPT
        } else {
            prompt
        };
        let length = match self.max_output {
            None => "as short as it can be without losing key information".to_string(),
            Some(MaxOutput::Percent(percent)) => {
                let limit = (words * percent as usize).div_ceil(100);
                format!("at most {percent}% of the input, about {limit} words of {words}")
            }
            Some(size) => format!("at most {size}"),
        };
        Ok(format!(
            "{prompt}\n\nExpertise: {}\nLength of the summary: {length}",
            self.expertise
        ))
    }
}

/// Fails if the params set an option that only works with the `owner`
/// agent type. `values` has the key and the value of each such option.
fn only_for(owner: AgentType, values: &[(&str, &Option<String>)]) -> Result<(), String> {
    match values.iter().find(|(_, value)| value.is_some()) {
        Some((key, _)) => Err(format!("option `{key}` works only with `agent:{owner}`")),
        None => Ok(()),
    }
}

/// The value of each option of an agent action, if the params set it.
#[derive(Default)]
struct Values {
    base_url: Option<String>,
    model: Option<String>,
    max_steps: Option<String>,
    task: Option<String>,
    expertise: Option<String>,
    max_output: Option<String>,
}

impl Values {
    /// Reads each param as one `key=value` option.
    ///
    /// Fails if a param has no `=`, its key is not in [`OPTION_KEYS`], its
    /// value is empty, or the params set the key twice.
    fn read(params: &[String]) -> Result<Self, String> {
        let mut values = Values::default();
        for param in params {
            let Some((key, value)) = param.split_once('=') else {
                return Err(format!(
                    "unexpected `{param}`, put the task in `task=\"...\"`"
                ));
            };
            let option = match key {
                "base_url" => &mut values.base_url,
                "model" => &mut values.model,
                "max_steps" => &mut values.max_steps,
                "task" => &mut values.task,
                "expertise" => &mut values.expertise,
                "max_output" => &mut values.max_output,
                _ => {
                    return Err(format!(
                        "unknown option `{key}`, expected {}",
                        expected_options()
                    ));
                }
            };
            if value.is_empty() {
                return Err(format!("option `{key}` has no value"));
            }
            if option.replace(value.to_string()).is_some() {
                return Err(format!("option `{key}` is set twice"));
            }
        }
        Ok(values)
    }
}

impl AgentConfig {
    /// Reads the configuration of an `agent` action. `env` returns the value
    /// of an environment variable, or `None` if it is not set.
    ///
    /// Fails if a param is not a known option, an option is empty, set twice
    /// or invalid, there is no model, or the params have an option of another
    /// agent type.
    pub fn from_params(
        agent: AgentType,
        params: &[String],
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        let Values {
            base_url,
            model,
            max_steps,
            task,
            expertise,
            max_output,
        } = Values::read(params)?;

        // The options of one agent type only control that agent, so another
        // agent fails on them instead of ignoring them.
        if agent != AgentType::Summarizer {
            only_for(
                AgentType::Summarizer,
                &[("expertise", &expertise), ("max_output", &max_output)],
            )?;
        }
        let summarizer = (agent == AgentType::Summarizer)
            .then(|| SummarizerOptions::from_values(expertise, max_output))
            .transpose()?;
        let max_steps = match max_steps {
            None => DEFAULT_MAX_STEPS,
            Some(value) => value
                .parse()
                .ok()
                .filter(|steps| *steps > 0)
                .ok_or_else(|| format!("`max_steps={value}` is not a positive number"))?,
        };
        let model = model
            .or_else(|| env(MODEL_ENV))
            .filter(|model| !model.is_empty())
            .ok_or_else(|| format!("no model: add `model=<id>` or set `{MODEL_ENV}`"))?;
        Ok(AgentConfig {
            base_url: base_url
                .or_else(|| env(BASE_URL_ENV))
                .filter(|url| !url.is_empty())
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            model,
            api_key: env(API_KEY_ENV).filter(|key| !key.is_empty()),
            max_steps,
            summarizer,
            task: task.unwrap_or_default(),
        })
    }
}

impl AgentConfig {
    /// The configuration for debug output, one `name: value` line each. It
    /// shows only whether the API key is set, never the key itself. For a
    /// `summarizer` agent, it also shows `expertise` and `max_output`.
    pub fn summary(&self) -> String {
        let key = if self.api_key.is_some() {
            "set"
        } else {
            "not set"
        };
        let mut summary = format!(
            "model: {}\nbase_url: {}\nmax_steps: {}\napi key: {key}",
            self.model, self.base_url, self.max_steps
        );
        if let Some(options) = &self.summarizer {
            let max_output = options
                .max_output
                .map_or("not set".to_string(), |size| size.to_string());
            summary.push_str(&format!(
                "\nexpertise: {}\nmax_output: {max_output}",
                options.expertise
            ));
        }
        summary
    }
}

/// The options of an agent action, as in "`base_url=`, `model=`".
fn expected_options() -> String {
    let options: Vec<String> = OPTION_KEYS.iter().map(|key| format!("`{key}=`")).collect();
    options.join(", ")
}
