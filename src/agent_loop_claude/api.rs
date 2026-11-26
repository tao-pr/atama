//! The client of the Managed Agents API.
//!
//! [`AgentsApi`] has the requests that a run needs. [`HttpAgentsApi`] sends
//! them with `ureq`. Tests use a mock [`AgentsApi`], or an [`HttpAgentsApi`]
//! whose base URL is a local mock server. Tests must not call the real API.

use std::fmt;
use std::io::BufReader;

use serde_json::{Value, json};
use ureq::Agent;
use ureq::http::Response;
use ureq::{Body, BodyReader};

use super::events::{SessionEvent, parse_event};
use super::sse::SseReader;
use crate::http;

/// The default base URL of the API.
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1";
/// The value of the `anthropic-version` header.
pub const API_VERSION: &str = "2026-10-21";
/// The value of the `anthropic-beta` header.
pub const BETA: &str = "managed-agents-2026-04-01";
/// The most events of one page of [`AgentsApi::list_events`].
pub const EVENTS_PAGE_LIMIT: u32 = 1000;

/// An error of a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    /// The HTTP status, or `None` if the request got no response.
    pub status: Option<u16>,
    pub message: String,
}

impl ApiError {
    fn new(message: String) -> Self {
        ApiError {
            status: None,
            message,
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<ApiError> for String {
    fn from(error: ApiError) -> String {
        error.message
    }
}

/// The ID and the version of an agent, from the response of a create or an
/// update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRef {
    pub id: String,
    pub version: u64,
}

/// The requests of the Managed Agents API that a run needs.
pub trait AgentsApi {
    /// `POST /agents` with `body`.
    fn create_agent(&mut self, body: &Value) -> Result<AgentRef, ApiError>;
    /// `POST /agents/{id}` with `body`. With a `version` in `body`, the
    /// API fails with status 409 if the agent has another version.
    fn update_agent(&mut self, id: &str, body: &Value) -> Result<AgentRef, ApiError>;
    /// `GET /agents/{id}`.
    fn get_agent(&mut self, id: &str) -> Result<Value, ApiError>;
    /// `POST /environments` with `body`. Returns the ID.
    fn create_environment(&mut self, body: &Value) -> Result<String, ApiError>;
    /// `POST /sessions` with `body`. Returns the ID.
    fn create_session(&mut self, body: &Value) -> Result<String, ApiError>;
    /// `GET /sessions/{id}`.
    fn get_session(&mut self, id: &str) -> Result<Value, ApiError>;
    /// `POST /sessions/{id}/events` with `{"events": events}`.
    fn send_events(&mut self, session_id: &str, events: &[Value]) -> Result<(), ApiError>;
    /// `GET /sessions/{id}/events`: all events of the session up to now, in
    /// order, from all pages.
    fn list_events(&mut self, session_id: &str) -> Result<Vec<SessionEvent>, ApiError>;
    /// `GET /sessions/{id}/events/stream`: the events from now on.
    fn open_stream(&mut self, session_id: &str) -> Result<Box<dyn EventStream>, ApiError>;
}

/// The events of an open stream. The run reads it on its own thread, so it
/// is [`Send`].
pub trait EventStream: Send {
    /// The next event, or `None` at the end of the stream.
    fn next_event(&mut self) -> Result<Option<SessionEvent>, String>;
}

/// An [`AgentsApi`] that sends HTTP requests.
pub struct HttpAgentsApi {
    agent: Agent,
    base_url: String,
    api_key: String,
}

impl HttpAgentsApi {
    /// A client for the API at `base_url`, as in [`DEFAULT_BASE_URL`],
    /// with the key `api_key`.
    pub fn new(base_url: &str, api_key: &str) -> Self {
        HttpAgentsApi {
            agent: http::agent(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    fn get(&self, url: &str) -> Result<Response<Body>, ureq::Error> {
        self.agent
            .get(url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("anthropic-beta", BETA)
            .call()
    }

    fn get_json(&self, path: &str) -> Result<Value, ApiError> {
        let url = self.url(path);
        let response = self.get(&url);
        read_json(&url, response)
    }

    fn post_json(&self, path: &str, body: &Value) -> Result<Value, ApiError> {
        let url = self.url(path);
        let response = self
            .agent
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("anthropic-beta", BETA)
            .content_type("application/json")
            .send(body.to_string());
        read_json(&url, response)
    }
}

impl AgentsApi for HttpAgentsApi {
    fn create_agent(&mut self, body: &Value) -> Result<AgentRef, ApiError> {
        agent_ref("/agents", self.post_json("/agents", body)?)
    }

    fn update_agent(&mut self, id: &str, body: &Value) -> Result<AgentRef, ApiError> {
        let path = format!("/agents/{id}");
        agent_ref(&path, self.post_json(&path, body)?)
    }

    fn get_agent(&mut self, id: &str) -> Result<Value, ApiError> {
        self.get_json(&format!("/agents/{id}"))
    }

    fn create_environment(&mut self, body: &Value) -> Result<String, ApiError> {
        string_field(
            "/environments",
            &self.post_json("/environments", body)?,
            "id",
        )
    }

    fn create_session(&mut self, body: &Value) -> Result<String, ApiError> {
        string_field("/sessions", &self.post_json("/sessions", body)?, "id")
    }

    fn get_session(&mut self, id: &str) -> Result<Value, ApiError> {
        self.get_json(&format!("/sessions/{id}"))
    }

    fn send_events(&mut self, session_id: &str, events: &[Value]) -> Result<(), ApiError> {
        self.post_json(
            &format!("/sessions/{session_id}/events"),
            &json!({ "events": events }),
        )?;
        Ok(())
    }

    fn list_events(&mut self, session_id: &str) -> Result<Vec<SessionEvent>, ApiError> {
        let mut events = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let mut path = format!("/sessions/{session_id}/events?limit={EVENTS_PAGE_LIMIT}");
            if let Some(page) = &page {
                path.push_str(&format!("&page={page}"));
            }
            let response = self.get_json(&path)?;
            let data = response["data"].as_array().ok_or_else(|| {
                ApiError::new(format!("response from `{path}` has no `data` list"))
            })?;
            for event in data {
                events.push(parse_event(event).map_err(ApiError::new)?);
            }
            match response["next_page"].as_str() {
                Some(next) if !next.is_empty() => page = Some(next.to_string()),
                _ => return Ok(events),
            }
        }
    }

    fn open_stream(&mut self, session_id: &str) -> Result<Box<dyn EventStream>, ApiError> {
        let url = self.url(&format!("/sessions/{session_id}/events/stream"));
        let response = self
            .agent
            .get(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("anthropic-beta", BETA)
            .header("accept", "text/event-stream")
            .call();
        let response = response.map_err(|err| request_failed(&url, err))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.into_body().read_to_string().unwrap_or_default();
            return Err(status_error(&url, status.as_u16(), &body));
        }
        let reader = BufReader::new(response.into_body().into_reader());
        Ok(Box::new(HttpEventStream {
            url,
            reader: SseReader::new(reader),
        }))
    }
}

/// The events of an SSE stream of the API.
struct HttpEventStream {
    url: String,
    reader: SseReader<BufReader<BodyReader<'static>>>,
}

impl EventStream for HttpEventStream {
    fn next_event(&mut self) -> Result<Option<SessionEvent>, String> {
        let url = &self.url;
        let Some(message) = self
            .reader
            .next_message()
            .map_err(|err| format!("cannot read the stream of `{url}`: {err}"))?
        else {
            return Ok(None);
        };
        let mut event: Value = serde_json::from_str(&message.data)
            .map_err(|err| format!("an event of `{url}` is not JSON: {err}"))?;
        if event.get("type").is_none()
            && let (Some(name), Some(object)) = (message.event, event.as_object_mut())
        {
            object.insert("type".to_string(), Value::String(name));
        }
        parse_event(&event).map(Some)
    }
}

fn request_failed(url: &str, err: ureq::Error) -> ApiError {
    ApiError::new(format!("request to `{url}` failed: {err}"))
}

/// The error of a non-2xx status. It has the message of the API error, and
/// the request ID, if the body has them. Else it has the body.
fn status_error(url: &str, status: u16, body: &str) -> ApiError {
    let json: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let detail = match json["error"]["message"].as_str() {
        Some(message) => match json["request_id"].as_str() {
            Some(request_id) => format!("{message} (request {request_id})"),
            None => message.to_string(),
        },
        None => body.trim().to_string(),
    };
    ApiError {
        status: Some(status),
        message: format!("`{url}` responded with {status}: {detail}"),
    }
}

/// Reads the JSON body of the response to a request to `url`.
fn read_json(url: &str, response: Result<Response<Body>, ureq::Error>) -> Result<Value, ApiError> {
    let mut response = response.map_err(|err| request_failed(url, err))?;
    let status = response.status();
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|err| ApiError::new(format!("cannot read response from `{url}`: {err}")))?;
    if !status.is_success() {
        return Err(status_error(url, status.as_u16(), &body));
    }
    serde_json::from_str(&body)
        .map_err(|err| ApiError::new(format!("response from `{url}` is not JSON: {err}")))
}

fn string_field(path: &str, json: &Value, field: &str) -> Result<String, ApiError> {
    json[field]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| ApiError::new(format!("response from `{path}` has no `{field}`")))
}

fn agent_ref(path: &str, json: Value) -> Result<AgentRef, ApiError> {
    let id = string_field(path, &json, "id")?;
    let version = json["version"]
        .as_u64()
        .ok_or_else(|| ApiError::new(format!("response from `{path}` has no `version`")))?;
    Ok(AgentRef { id, version })
}
