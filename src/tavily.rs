//! Web search with the Tavily search API
//! (<https://docs.tavily.com/documentation/api-reference/endpoint/search>).
//!
//! A `tavily_search` action ([`Actor::TavilySearch`](crate::Actor::TavilySearch))
//! and a `websearch` action ([`crate::websearch`]) both send a `POST <base_url>/search` request, and
//! both return the response body unchanged. The API key comes only from
//! [`API_KEY_ENV`], so it is never in the job file or the log.

use serde_json::{Map, Value, json};
use ureq::Agent;

/// Base URL of the Tavily API if the action sets none.
pub const DEFAULT_BASE_URL: &str = "https://api.tavily.com";
/// Environment variable with the Tavily API key.
pub const API_KEY_ENV: &str = "TAVILY_API_KEY";

/// One search request, read from the params of a `tavily_search` action or
/// from the arguments of a tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchConfig {
    pub base_url: String,
    pub api_key: String,
    pub query: String,
    /// Most results to return. Tavily uses its default if not set.
    pub max_results: Option<u32>,
    /// `basic` or `advanced`, for example. Tavily uses its default if not set.
    pub search_depth: Option<String>,
    /// `general`, `news` or `finance`, for example. Tavily uses its default
    /// if not set.
    pub topic: Option<String>,
    /// `true`, `false`, `basic` or `advanced`. Tavily uses its default if not set.
    pub include_answer: Option<String>,
    /// `true`, `false`, `markdown` or `text`: whether each result also has
    /// the content of its page, and in which format. Tavily uses its
    /// default if not set.
    pub include_raw_content: Option<String>,
}

impl SearchConfig {
    /// Reads the configuration of a `tavily_search` action.
    ///
    /// A param `base_url=<url>`, `max_results=<n>`, `search_depth=<depth>`,
    /// `topic=<topic>`, `include_answer=<value>` or
    /// `include_raw_content=<value>` sets that option. The
    /// other params, joined with spaces, are the query. If there are none,
    /// `input` without surrounding whitespace is the query. `env` returns
    /// the value of an environment variable, or `None` if it is not set.
    ///
    /// Fails if an option is empty or set twice, `max_results` is not a
    /// number, the query is empty, or [`API_KEY_ENV`] is not set.
    pub fn from_params(
        params: &[String],
        input: &str,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        let mut base_url = None;
        let mut max_results = None;
        let mut search_depth = None;
        let mut topic = None;
        let mut include_answer = None;
        let mut include_raw_content = None;
        let mut query = Vec::new();
        for param in params {
            // A word without `=`, such as `topic`, is part of the query.
            let (key, value) = param.split_once('=').unwrap_or_default();
            let option = match key {
                "base_url" => &mut base_url,
                "max_results" => &mut max_results,
                "search_depth" => &mut search_depth,
                "topic" => &mut topic,
                "include_answer" => &mut include_answer,
                "include_raw_content" => &mut include_raw_content,
                _ => {
                    query.push(param.as_str());
                    continue;
                }
            };
            if value.is_empty() {
                return Err(format!("option `{key}` has no value"));
            }
            if option.replace(value.to_string()).is_some() {
                return Err(format!("option `{key}` is set twice"));
            }
        }

        let max_results = max_results
            .map(|value| {
                value
                    .parse()
                    .map_err(|_| format!("`max_results={value}` is not a number"))
            })
            .transpose()?;
        let query = match query.join(" ") {
            query if query.is_empty() => input.trim().to_string(),
            query => query,
        };
        if query.is_empty() {
            return Err("`tavily_search` needs a query in its params or an input".to_string());
        }
        Ok(SearchConfig {
            base_url: base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            api_key: api_key(env)?,
            query,
            max_results,
            search_depth,
            topic,
            include_answer,
            include_raw_content,
        })
    }

    /// The JSON body of the search request. It has only the options that
    /// are set, so Tavily uses its defaults for the others.
    pub fn request_body(&self) -> Value {
        let mut body = Map::new();
        body.insert("query".into(), json!(self.query));
        if let Some(max_results) = self.max_results {
            body.insert("max_results".into(), json!(max_results));
        }
        if let Some(search_depth) = &self.search_depth {
            body.insert("search_depth".into(), json!(search_depth));
        }
        if let Some(topic) = &self.topic {
            body.insert("topic".into(), json!(topic));
        }
        // Tavily takes a boolean, or the name of the answer depth or the
        // format of the page content.
        for (key, value) in [
            ("include_answer", &self.include_answer),
            ("include_raw_content", &self.include_raw_content),
        ] {
            if let Some(value) = value {
                body.insert(key.into(), flag_value(value));
            }
        }
        Value::Object(body)
    }

    /// Sends the search request and returns the response body, the search
    /// results as JSON.
    ///
    /// Fails if the request fails, or the server responds with a non-2xx status.
    pub fn search(&self) -> Result<String, String> {
        let url = format!("{}/search", self.base_url.trim_end_matches('/'));
        let agent =
            Agent::new_with_config(Agent::config_builder().http_status_as_error(false).build());
        let mut response = agent
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .content_type("application/json")
            .send(self.request_body().to_string())
            .map_err(|err| format!("request to `{url}` failed: {err}"))?;
        let status = response.status();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|err| format!("cannot read response from `{url}`: {err}"))?;
        if !status.is_success() {
            return Err(format!("`{url}` responded with {status}: {}", body.trim()));
        }
        Ok(body)
    }
}

/// `true` and `false` as JSON booleans, and other values as JSON strings.
fn flag_value(value: &str) -> Value {
    match value {
        "true" => json!(true),
        "false" => json!(false),
        other => json!(other),
    }
}

/// Reads the API key from [`API_KEY_ENV`] with `env`. Fails if it is not
/// set or empty.
pub fn api_key(env: impl Fn(&str) -> Option<String>) -> Result<String, String> {
    env(API_KEY_ENV)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| format!("no Tavily API key: set `{API_KEY_ENV}`"))
}
