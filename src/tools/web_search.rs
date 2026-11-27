//! The `web_search` tool: one Tavily search.
//!
//! The tool runs the same code as the `websearch` action
//! ([`WebSearchOptions::search`]). The API key comes only from
//! [`tavily::API_KEY_ENV`]. A `claude-agent` sees the tool as
//! `local_web_search`, so it does not clash with the native `web_search`
//! of Claude.

use serde_json::{Value, json};

use super::{Tool, string_argument, truncate};
use crate::tavily;
use crate::websearch::{DEFAULT_MAX_RESULTS, MAX_SEARCH_RESULTS, WebSearchOptions};

/// Returns the value of an environment variable, or `None` if it is not set.
type Env = Box<dyn Fn(&str) -> Option<String>>;

/// `web_search`: searches the web with Tavily and returns the response JSON
/// unchanged. It does not get the content of the result pages, so that the
/// result stays short.
pub struct WebSearch {
    base_url: String,
    env: Env,
}

impl Default for WebSearch {
    /// The tool with the real environment and [`tavily::DEFAULT_BASE_URL`].
    fn default() -> Self {
        WebSearch::new(tavily::DEFAULT_BASE_URL, |name| std::env::var(name).ok())
    }
}

impl WebSearch {
    /// The tool that sends its searches to `base_url`, and reads the API key
    /// with `env`. Tests use it with a local mock server and a fake
    /// environment.
    pub fn new(
        base_url: impl Into<String>,
        env: impl Fn(&str) -> Option<String> + 'static,
    ) -> Self {
        WebSearch {
            base_url: base_url.into(),
            env: Box::new(env),
        }
    }
}

impl Tool for WebSearch {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Searches the web and returns the results as JSON: the title, the URL and a short \
         extract of each result page. Use it to find facts that are not in the files."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "What to search for."},
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_SEARCH_RESULTS,
                    "description": format!(
                        "The most results to return. The default is {DEFAULT_MAX_RESULTS}."
                    ),
                },
            },
            "required": ["query"],
        })
    }

    /// Fails if the query is empty, `max_results` is not a number from 1 to
    /// [`MAX_SEARCH_RESULTS`], the environment has no API key, or the search
    /// fails.
    fn call(&mut self, arguments: &Value) -> Result<String, String> {
        let query = string_argument(arguments, "query")?;
        if query.trim().is_empty() {
            return Err("the argument `query` is empty".to_string());
        }
        let max_results = match arguments.get("max_results") {
            None | Some(Value::Null) => DEFAULT_MAX_RESULTS,
            Some(value) => value
                .as_u64()
                .filter(|n| (1..=u64::from(MAX_SEARCH_RESULTS)).contains(n))
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| {
                    format!(
                        "the argument `max_results` is {value}, not a number from 1 to \
                         {MAX_SEARCH_RESULTS}"
                    )
                })?,
        };
        let options = WebSearchOptions {
            topic: query,
            max_results,
            fetch_content: false,
            base_url: self.base_url.clone(),
        };
        let body = options.search(&*self.env, None)?;
        Ok(truncate(body))
    }

    fn read_only(&self) -> bool {
        true
    }
}
