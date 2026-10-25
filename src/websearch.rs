//! The `websearch` action ([`Actor::WebSearch`](crate::Actor::WebSearch)):
//! one Tavily search for a topic.
//!
//! The action asks no model. Its output is the response body from Tavily,
//! exactly as Tavily sends it. The search uses the client in
//! [`crate::tavily`], so the API key comes only from
//! [`tavily::API_KEY_ENV`].

use std::io::Write;

use serde_json::Value;

use crate::tavily::{self, SearchConfig};

/// Search results that a `websearch` action asks for if it sets no
/// `max_results`.
pub const DEFAULT_MAX_RESULTS: u32 = 5;
/// Most search results that a `websearch` action can ask for. Tavily returns
/// at most 20.
pub const MAX_SEARCH_RESULTS: u32 = 20;
/// Options of a `websearch` action.
pub const OPTION_KEYS: [&str; 4] = ["topic", "max_results", "fetch_content", "base_url"];

/// The options of a `websearch` action: what it searches for, how many
/// results it asks for, whether it also gets the page of each result, and
/// the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSearchOptions {
    /// What to search for, from `topic=<text>`.
    pub topic: String,
    /// Most search results to ask for, from `max_results=<n>`. The default
    /// is [`DEFAULT_MAX_RESULTS`].
    pub max_results: u32,
    /// Whether the search also gets the content of each result page, as
    /// markdown, from `fetch_content=<true|false>`. The default is `false`.
    pub fetch_content: bool,
    /// Base URL of the Tavily API, from `base_url=<url>`. The default is
    /// [`tavily::DEFAULT_BASE_URL`].
    pub base_url: String,
}

impl WebSearchOptions {
    /// Reads the options from the values of `topic=`, `max_results=`,
    /// `fetch_content=` and `base_url=`, if the action sets them.
    ///
    /// Fails if `max_results` is not a number from 1 to
    /// [`MAX_SEARCH_RESULTS`], `fetch_content` is not `true` or `false`, or
    /// there is no topic.
    pub fn from_values(
        topic: Option<String>,
        max_results: Option<String>,
        fetch_content: Option<String>,
        base_url: Option<String>,
    ) -> Result<Self, String> {
        let max_results = match max_results {
            None => DEFAULT_MAX_RESULTS,
            Some(value) => value
                .parse()
                .ok()
                .filter(|n| (1..=MAX_SEARCH_RESULTS).contains(n))
                .ok_or_else(|| {
                    format!("`max_results={value}` is not a number from 1 to {MAX_SEARCH_RESULTS}")
                })?,
        };
        let fetch_content = match fetch_content.as_deref() {
            None | Some("false") => false,
            Some("true") => true,
            Some(value) => {
                return Err(format!("`fetch_content={value}` is not `true` or `false`"));
            }
        };
        let topic =
            topic.ok_or("`websearch` needs a topic, as in `topic=\"latest Rust release\"`")?;
        Ok(WebSearchOptions {
            topic,
            max_results,
            fetch_content,
            base_url: base_url.unwrap_or_else(|| tavily::DEFAULT_BASE_URL.to_string()),
        })
    }

    /// The options for debug output, one `name: value` line each.
    pub fn summary(&self) -> String {
        format!(
            "topic: {}\nmax_results: {}\nfetch_content: {}\nbase_url: {}",
            self.topic, self.max_results, self.fetch_content, self.base_url
        )
    }

    /// Sends one Tavily search for the topic, and returns the response body
    /// exactly as Tavily sends it. `env` returns the value of an environment
    /// variable, or `None` if it is not set.
    ///
    /// The search asks for `max_results` results. If `fetch_content` is
    /// `true`, it also asks for the page of each result as markdown
    /// (`include_raw_content`). If `debug` is set, the search writes the
    /// options and the response body, as pretty JSON, to it. The API key is
    /// never written.
    ///
    /// Fails if `env` has no Tavily API key, or the search fails.
    pub fn search(
        &self,
        env: &dyn Fn(&str) -> Option<String>,
        mut debug: Option<&mut dyn Write>,
    ) -> Result<String, String> {
        if let Some(out) = debug.as_mut() {
            // A failed write does not stop the search.
            let _ = writeln!(out, "=== websearch options ===\n{}", self.summary());
        }
        let search = SearchConfig {
            base_url: self.base_url.clone(),
            api_key: tavily::api_key(env)?,
            query: self.topic.clone(),
            max_results: Some(self.max_results),
            search_depth: None,
            topic: None,
            include_answer: None,
            include_raw_content: self.fetch_content.then(|| "markdown".to_string()),
        };
        let body = search.search()?;
        if let Some(out) = debug {
            let _ = writeln!(out, "=== websearch response ===\n{}", pretty_json(&body));
        }
        Ok(body)
    }
}

/// JSON text as pretty JSON. Text that is not JSON stays as it is.
fn pretty_json(text: &str) -> String {
    match serde_json::from_str::<Value>(text) {
        Ok(json) => serde_json::to_string_pretty(&json).expect("JSON values always serialize"),
        Err(_) => text.to_string(),
    }
}
