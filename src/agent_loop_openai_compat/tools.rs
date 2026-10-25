//! Built-in tools for the agent loop. They run the same actors as the
//! actions of a job file.

use serde_json::{Value, json};

use super::Tool;
use crate::actor::{Actor, AgentType, ApiVerb};
use crate::tavily::{self, SearchConfig};

/// Most characters of a tool result that the model gets. A longer result is
/// cut, so that one web page or one command cannot fill the model's context.
pub const MAX_RESULT_CHARS: usize = 20_000;

/// The tools that each agent type gets, with the environment of the process
/// (see [`tools_for`]).
pub fn default_tools(agent: AgentType) -> Vec<Box<dyn Tool>> {
    tools_for(agent, |name| std::env::var(name).ok())
}

/// The tools that each agent type gets:
///
/// - `websearch` gets [`FetchUrl`], and also [`TavilySearch`] if `env` has
///   the Tavily API key ([`tavily::API_KEY_ENV`]).
/// - `coder` gets [`Bash`].
///
/// `env` returns the value of an environment variable, or `None` if it is
/// not set.
pub fn tools_for(agent: AgentType, env: impl Fn(&str) -> Option<String>) -> Vec<Box<dyn Tool>> {
    match agent {
        AgentType::WebSearch => {
            let mut tools: Vec<Box<dyn Tool>> = vec![Box::new(FetchUrl)];
            if let Ok(api_key) = tavily::api_key(env) {
                tools.push(Box::new(TavilySearch::new(
                    tavily::DEFAULT_BASE_URL,
                    &api_key,
                )));
            }
            tools
        }
        AgentType::Coder => vec![Box::new(Bash)],
    }
}

/// `fetch_url`: sends a `GET` request and returns the response body.
pub struct FetchUrl;

impl Tool for FetchUrl {
    fn name(&self) -> &str {
        "fetch_url"
    }

    fn description(&self) -> &str {
        "Sends an HTTP GET request to a URL and returns the response body."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"url": {"type": "string", "description": "The URL to get."}},
            "required": ["url"],
        })
    }

    fn call(&mut self, arguments: &Value) -> Result<String, String> {
        let url = string_argument(arguments, "url")?;
        let body = Actor::ApiCall {
            verb: ApiVerb::GET,
            url,
            input_file: None,
            payload: None,
            auth: None,
        }
        .act("")?;
        Ok(truncate(body))
    }
}

/// `bash`: runs a command in bash in the current directory and returns its
/// stdout.
pub struct Bash;

impl Tool for Bash {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Runs a command in bash in the current directory and returns its stdout. \
         A command that exits with a non-zero status returns an error with its stderr."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"command": {"type": "string", "description": "The command to run."}},
            "required": ["command"],
        })
    }

    fn call(&mut self, arguments: &Value) -> Result<String, String> {
        let output = Actor::BashScript {
            cmd: string_argument(arguments, "command")?,
            params: Vec::new(),
        }
        .act("")?;
        Ok(truncate(output))
    }
}

/// `tavily_search`: searches the web with the Tavily API and returns the
/// answer and the results as text (see [`tavily::format_results`]).
pub struct TavilySearch {
    base_url: String,
    api_key: String,
}

impl TavilySearch {
    /// Makes a tool that sends requests to `base_url` with `api_key`.
    pub fn new(base_url: &str, api_key: &str) -> Self {
        TavilySearch {
            base_url: base_url.to_string(),
            api_key: api_key.to_string(),
        }
    }
}

impl Tool for TavilySearch {
    fn name(&self) -> &str {
        "tavily_search"
    }

    fn description(&self) -> &str {
        "Searches the web and returns a short answer and the title, URL and content \
         of each result. Use fetch_url to read a result in full."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "The search query."},
                "max_results": {
                    "type": "integer",
                    "description": "Most results to return, from 1 to 20. The default is 5.",
                },
                "topic": {
                    "type": "string",
                    "enum": ["general", "news", "finance"],
                    "description": "The kind of search. The default is general.",
                },
            },
            "required": ["query"],
        })
    }

    fn call(&mut self, arguments: &Value) -> Result<String, String> {
        let max_results = match arguments.get("max_results") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or("argument `max_results` is not a positive integer")?,
            ),
        };
        let config = SearchConfig {
            base_url: self.base_url.clone(),
            api_key: self.api_key.clone(),
            query: string_argument(arguments, "query")?,
            max_results,
            search_depth: None,
            topic: arguments
                .get("topic")
                .and_then(Value::as_str)
                .map(str::to_string),
            include_answer: Some("true".to_string()),
        };
        Ok(truncate(tavily::format_results(&config.search()?)?))
    }
}

fn string_argument(arguments: &Value, name: &str) -> Result<String, String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("missing string argument `{name}`"))
}

/// Cuts `text` to [`MAX_RESULT_CHARS`] characters and says so at the end.
pub fn truncate(text: String) -> String {
    match text.char_indices().nth(MAX_RESULT_CHARS) {
        None => text,
        Some((end, _)) => format!(
            "{}\n[cut: the result has more than {MAX_RESULT_CHARS} characters]",
            &text[..end]
        ),
    }
}
