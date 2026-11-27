//! Tests of the HTTP client in `src/agent_loop_claude/api.rs`, with a local
//! mock server. They do not call the real API.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use serde_json::{Value, json};

use atama::agent_loop_claude::api::{
    API_VERSION, AgentRef, AgentsApi, ApiError, BETA, DEFAULT_BASE_URL, EVENTS_PAGE_LIMIT,
    HttpAgentsApi,
};
use atama::agent_loop_claude::events::{IdleStop, SessionEvent};

/// One response of the mock server.
struct Answer {
    status: u16,
    content_type: &'static str,
    body: String,
}

fn json_answer(status: u16, body: Value) -> Answer {
    Answer {
        status,
        content_type: "application/json",
        body: body.to_string(),
    }
}

fn text_answer(status: u16, content_type: &'static str, body: &str) -> Answer {
    Answer {
        status,
        content_type,
        body: body.to_string(),
    }
}

/// A request that the mock server got.
#[derive(Debug)]
struct Request {
    /// The request line, as in `POST /v1/agents HTTP/1.1`.
    line: String,
    /// The headers, with lower case names.
    headers: Vec<(String, String)>,
    body: String,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap()
    }
}

/// Starts a local HTTP server that gives one answer for each request, in
/// order. Returns its base URL, with `/v1`, and the requests that it gets.
fn serve(answers: Vec<Answer>) -> (String, Receiver<Request>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let (sender, received) = mpsc::channel();
    thread::spawn(move || {
        for answer in answers {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut headers = Vec::new();
            let mut content_length = 0;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                let header = header.trim_end();
                if header.is_empty() {
                    break;
                }
                let (name, value) = header.split_once(':').unwrap();
                let name = name.to_lowercase();
                let value = value.trim().to_string();
                if name == "content-length" {
                    content_length = value.parse().unwrap();
                }
                headers.push((name, value));
            }
            let mut body = vec![0; content_length];
            reader.read_exact(&mut body).unwrap();
            sender
                .send(Request {
                    line: line.trim_end().to_string(),
                    headers,
                    body: String::from_utf8(body).unwrap(),
                })
                .unwrap();
            write!(
                stream,
                "HTTP/1.1 {} Test\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                answer.status,
                answer.content_type,
                answer.body.len(),
                answer.body
            )
            .unwrap();
        }
    });
    (url, received)
}

fn assert_headers(request: &Request) {
    assert_eq!(request.header("x-api-key"), Some("test-key"));
    assert_eq!(request.header("anthropic-version"), Some(API_VERSION));
    assert_eq!(request.header("anthropic-beta"), Some(BETA));
}

#[test]
fn the_default_base_url_is_the_anthropic_api() {
    assert_eq!(DEFAULT_BASE_URL, "https://api.anthropic.com/v1");
    assert_eq!(BETA, "managed-agents-2026-04-01");
}

#[test]
fn creates_updates_and_gets_an_agent() {
    let (url, requests) = serve(vec![
        json_answer(200, json!({"id": "agent_1", "version": 1, "name": "a"})),
        json_answer(200, json!({"id": "agent_1", "version": 2})),
        json_answer(200, json!({"id": "agent_1", "version": 2, "tools": []})),
    ]);
    // A trailing slash of the base URL is removed.
    let mut api = HttpAgentsApi::new(&format!("{url}/"), "test-key");
    let body = json!({"name": "a", "model": "claude-opus-5-5"});

    assert_eq!(
        api.create_agent(&body).unwrap(),
        AgentRef {
            id: "agent_1".into(),
            version: 1
        }
    );
    let request = requests.recv().unwrap();
    assert_eq!(request.line, "POST /v1/agents HTTP/1.1");
    assert_headers(&request);
    assert_eq!(request.header("content-type"), Some("application/json"));
    assert_eq!(request.json(), body);

    let update = json!({"version": 1, "system": "Be brief."});
    assert_eq!(api.update_agent("agent_1", &update).unwrap().version, 2);
    let request = requests.recv().unwrap();
    assert_eq!(request.line, "POST /v1/agents/agent_1 HTTP/1.1");
    assert_eq!(request.json(), update);

    assert_eq!(api.get_agent("agent_1").unwrap()["tools"], json!([]));
    let request = requests.recv().unwrap();
    assert_eq!(request.line, "GET /v1/agents/agent_1 HTTP/1.1");
    assert_headers(&request);
}

#[test]
fn creates_an_environment_and_a_session() {
    let (url, requests) = serve(vec![
        json_answer(200, json!({"id": "env_1"})),
        json_answer(200, json!({"id": "sesn_1", "status": "idle"})),
        json_answer(200, json!({"id": "sesn_1", "status": "running"})),
    ]);
    let mut api = HttpAgentsApi::new(&url, "test-key");

    let environment = json!({"name": "e", "config": {"type": "cloud"}});
    assert_eq!(api.create_environment(&environment).unwrap(), "env_1");
    let request = requests.recv().unwrap();
    assert_eq!(request.line, "POST /v1/environments HTTP/1.1");
    assert_eq!(request.json(), environment);

    let session = json!({"agent": "agent_1", "environment_id": "env_1"});
    assert_eq!(api.create_session(&session).unwrap(), "sesn_1");
    let request = requests.recv().unwrap();
    assert_eq!(request.line, "POST /v1/sessions HTTP/1.1");
    assert_eq!(request.json(), session);

    assert_eq!(api.get_session("sesn_1").unwrap()["status"], "running");
    let request = requests.recv().unwrap();
    assert_eq!(request.line, "GET /v1/sessions/sesn_1 HTTP/1.1");
}

#[test]
fn sends_events_in_one_request() {
    let (url, requests) = serve(vec![json_answer(200, json!({"data": []}))]);
    let mut api = HttpAgentsApi::new(&url, "test-key");
    let events = [
        json!({"type": "user.message", "content": [{"type": "text", "text": "Hi"}]}),
        json!({"type": "user.interrupt"}),
    ];
    api.send_events("sesn_1", &events).unwrap();
    let request = requests.recv().unwrap();
    assert_eq!(request.line, "POST /v1/sessions/sesn_1/events HTTP/1.1");
    assert_headers(&request);
    assert_eq!(request.json(), json!({"events": events}));
}

#[test]
fn lists_the_events_of_all_pages() {
    let (url, requests) = serve(vec![
        json_answer(
            200,
            json!({
                "data": [{"type": "session.status_running", "id": "sevt_1"}],
                "next_page": "page_2",
            }),
        ),
        json_answer(
            200,
            json!({
                "data": [{
                    "type": "session.status_idle",
                    "id": "sevt_2",
                    "stop_reason": {"type": "end_turn"},
                }],
                "next_page": null,
            }),
        ),
    ]);
    let mut api = HttpAgentsApi::new(&url, "test-key");
    assert_eq!(
        api.list_events("sesn_1").unwrap(),
        [
            SessionEvent::StatusRunning {
                id: "sevt_1".into()
            },
            SessionEvent::StatusIdle {
                id: "sevt_2".into(),
                stop: IdleStop::EndTurn
            },
        ]
    );
    assert_eq!(
        requests.recv().unwrap().line,
        format!("GET /v1/sessions/sesn_1/events?limit={EVENTS_PAGE_LIMIT} HTTP/1.1")
    );
    assert_eq!(
        requests.recv().unwrap().line,
        format!("GET /v1/sessions/sesn_1/events?limit={EVENTS_PAGE_LIMIT}&page=page_2 HTTP/1.1")
    );
}

#[test]
fn list_events_fails_on_a_bad_page() {
    let (url, _requests) = serve(vec![
        json_answer(200, json!({"events": []})),
        json_answer(200, json!({"data": [{"id": "x"}]})),
    ]);
    let mut api = HttpAgentsApi::new(&url, "test-key");
    let path = format!("/sessions/s/events?limit={EVENTS_PAGE_LIMIT}");
    assert_eq!(
        api.list_events("s").unwrap_err().message,
        format!("response from `{path}` has no `data` list")
    );
    assert_eq!(
        api.list_events("s").unwrap_err().message,
        "event without a type: {\"id\":\"x\"}"
    );
}

#[test]
fn streams_the_events_of_a_session() {
    let stream = "\
: heartbeat\n\n\
event: session.status_running\n\
data: {\"type\":\"session.status_running\",\"id\":\"sevt_1\"}\n\n\
event: agent.message\n\
data: {\"id\":\"sevt_2\",\"content\":[{\"type\":\"text\",\"text\":\"Done.\"}]}\n\n";
    let (url, requests) = serve(vec![text_answer(200, "text/event-stream", stream)]);
    let mut api = HttpAgentsApi::new(&url, "test-key");
    let mut events = api.open_stream("sesn_1").unwrap();
    assert_eq!(
        events.next_event().unwrap(),
        Some(SessionEvent::StatusRunning {
            id: "sevt_1".into()
        })
    );
    // An event without a type gets the name of its `event:` field.
    assert_eq!(
        events.next_event().unwrap(),
        Some(SessionEvent::AgentMessage {
            id: "sevt_2".into(),
            text: "Done.".into()
        })
    );
    assert_eq!(events.next_event().unwrap(), None);
    let request = requests.recv().unwrap();
    assert_eq!(
        request.line,
        "GET /v1/sessions/sesn_1/events/stream HTTP/1.1"
    );
    assert_headers(&request);
    assert_eq!(request.header("accept"), Some("text/event-stream"));
}

#[test]
fn a_stream_fails_on_an_event_that_is_not_json() {
    let (url, _requests) = serve(vec![
        text_answer(200, "text/event-stream", "data: {oops\n\n"),
        text_answer(200, "text/event-stream", "data: [1]\n\n"),
    ]);
    let mut api = HttpAgentsApi::new(&url, "test-key");
    let stream_url = format!("{url}/sessions/s/events/stream");
    let error = api.open_stream("s").unwrap().next_event().unwrap_err();
    assert!(
        error.starts_with(&format!("an event of `{stream_url}` is not JSON:")),
        "{error}"
    );
    assert_eq!(
        api.open_stream("s").unwrap().next_event().unwrap_err(),
        "event without a type: [1]"
    );
}

#[test]
fn a_stream_with_an_error_status_fails() {
    let (url, _requests) = serve(vec![json_answer(
        404,
        json!({
            "type": "error",
            "error": {"type": "not_found_error", "message": "no such session"},
            "request_id": "req_1",
        }),
    )]);
    let mut api = HttpAgentsApi::new(&url, "test-key");
    let error = match api.open_stream("s") {
        Ok(_) => panic!("the stream opened"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        ApiError {
            status: Some(404),
            message: format!(
                "`{url}/sessions/s/events/stream` responded with 404: no such session (request req_1)"
            ),
        }
    );
}

#[test]
fn an_error_has_the_status_and_the_message_of_the_api() {
    let (url, _requests) = serve(vec![
        json_answer(
            409,
            json!({"type": "error", "error": {"type": "invalid_request_error", "message": "version mismatch"}}),
        ),
        text_answer(500, "text/plain", " server broke \n"),
    ]);
    let mut api = HttpAgentsApi::new(&url, "test-key");
    let error = api
        .update_agent("agent_1", &json!({"version": 1}))
        .unwrap_err();
    assert_eq!(error.status, Some(409));
    assert_eq!(
        error.to_string(),
        format!("`{url}/agents/agent_1` responded with 409: version mismatch")
    );
    let error: String = api.get_agent("agent_1").unwrap_err().into();
    assert_eq!(
        error,
        format!("`{url}/agents/agent_1` responded with 500: server broke")
    );
}

#[test]
fn fails_on_a_response_without_the_needed_fields() {
    let (url, _requests) = serve(vec![
        json_answer(200, json!({"version": 1})),
        json_answer(200, json!({"id": "agent_1"})),
        json_answer(200, json!({})),
        text_answer(200, "application/json", "not json"),
    ]);
    let mut api = HttpAgentsApi::new(&url, "test-key");
    assert_eq!(
        api.create_agent(&json!({})).unwrap_err().message,
        "response from `/agents` has no `id`"
    );
    assert_eq!(
        api.update_agent("agent_1", &json!({})).unwrap_err().message,
        "response from `/agents/agent_1` has no `version`"
    );
    let error = api.create_session(&json!({})).unwrap_err();
    assert_eq!(error.status, None);
    assert_eq!(error.message, "response from `/sessions` has no `id`");
    let error = api.create_environment(&json!({})).unwrap_err().message;
    assert!(
        error.starts_with(&format!("response from `{url}/environments` is not JSON:")),
        "{error}"
    );
}

#[test]
fn fails_when_the_server_does_not_answer() {
    // A port with no server: bind it, then close it.
    let url = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}/v1", listener.local_addr().unwrap())
    };
    let mut api = HttpAgentsApi::new(&url, "test-key");
    let error = api.get_session("s").unwrap_err();
    assert_eq!(error.status, None);
    assert!(
        error
            .message
            .starts_with(&format!("request to `{url}/sessions/s` failed:")),
        "{}",
        error.message
    );
    let error = match api.open_stream("s") {
        Ok(_) => panic!("the stream opened"),
        Err(error) => error.message,
    };
    assert!(
        error.starts_with(&format!(
            "request to `{url}/sessions/s/events/stream` failed:"
        )),
        "{error}"
    );
}

#[test]
fn send_events_fails_on_an_error_status() {
    let (url, _requests) = serve(vec![json_answer(
        400,
        json!({"type": "error", "error": {"type": "invalid_request_error", "message": "session is waiting"}}),
    )]);
    let mut api = HttpAgentsApi::new(&url, "test-key");
    let error = api
        .send_events("s", &[json!({"type": "user.message"})])
        .unwrap_err();
    assert_eq!(error.status, Some(400));
    assert_eq!(
        error.message,
        format!("`{url}/sessions/s/events` responded with 400: session is waiting")
    );
}
