//! Tests for the `websearch` action in `src/websearch.rs`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use serde_json::{Value, json};

use atama::Actor;
use atama::websearch::{DEFAULT_MAX_RESULTS, WebSearchOptions};

/// Starts a local HTTP server that answers one request per response, in order.
///
/// Returns the server's base URL and a receiver that gets each raw request.
fn serve(responses: Vec<(u16, String)>) -> (String, Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("cannot bind test server");
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (requests, received) = mpsc::channel();

    thread::spawn(move || {
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().expect("cannot accept connection");
            let mut reader = BufReader::new(stream.try_clone().unwrap());

            let mut request = String::new();
            let mut content_length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                    content_length = value.trim().parse().unwrap();
                }
                request.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut request_body = vec![0; content_length];
            reader.read_exact(&mut request_body).unwrap();
            request.push_str(&String::from_utf8(request_body).unwrap());
            requests.send(request).unwrap();

            write!(
                stream,
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body.as_bytes()).unwrap();
        }
    });

    (url, received)
}

fn closed_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    format!("http://{}", listener.local_addr().unwrap())
}

/// The JSON body of a raw HTTP request.
fn request_json(request: &str) -> Value {
    let (_, body) = request.split_once("\r\n\r\n").expect("request has a body");
    serde_json::from_str(body).expect("request body is JSON")
}

/// An environment that has only the Tavily API key `key-1`.
fn tavily_env(name: &str) -> Option<String> {
    (name == "TAVILY_API_KEY").then(|| "key-1".to_string())
}

fn no_env(_: &str) -> Option<String> {
    None
}

/// The options of a `websearch` action, parsed from the job file text.
fn options(statement: &str) -> WebSearchOptions {
    match statement.parse::<Actor>().expect(statement) {
        Actor::WebSearch(options) => options,
        other => panic!("expected a websearch action, got {other:?}"),
    }
}

/// A Tavily response with one result, with the spacing and the extra fields
/// that a real response can have.
fn search_response() -> String {
    r#"{"query": "rust",  "results": [{"title": "Rust 2.0", "url": "https://r.test", "content": "Released.", "score": 0.9}], "response_time": 1.2}"#
        .to_string()
}

#[test]
fn returns_the_exact_tavily_response_body() {
    let (url, searches) = serve(vec![(200, search_response())]);
    let options = options(&format!(r#"websearch topic="rust release" base_url={url}"#));

    assert_eq!(options.search(&tavily_env, None), Ok(search_response()));
    let search = searches.recv().unwrap();
    assert!(search.starts_with("POST /search "), "{search}");
    assert!(search.contains("authorization: Bearer key-1"), "{search}");
    assert_eq!(
        request_json(&search),
        json!({"query": "rust release", "max_results": DEFAULT_MAX_RESULTS})
    );
}

#[test]
fn fetch_content_asks_for_markdown_pages() {
    let body = json!({"results": [{
        "title": "Rust 2.0",
        "url": "https://r.test",
        "content": "Released.",
        "raw_content": "# Rust 2.0\n\nFull notes.",
    }]})
    .to_string();
    let (url, searches) = serve(vec![(200, body.clone())]);
    let options = options(&format!(
        "websearch topic=rust max_results=2 fetch_content=true base_url={url}"
    ));

    assert_eq!(options.search(&tavily_env, None), Ok(body));
    assert_eq!(
        request_json(&searches.recv().unwrap()),
        json!({"query": "rust", "max_results": 2, "include_raw_content": "markdown"})
    );
}

#[test]
fn debug_writes_the_options_and_the_response() {
    let (url, _searches) = serve(vec![(200, search_response())]);
    let options = options(&format!(
        "websearch topic=rust max_results=2 base_url={url}"
    ));
    let mut out = Vec::new();

    // Debug output does not change the output.
    assert_eq!(
        options.search(&tavily_env, Some(&mut out)),
        Ok(search_response())
    );
    let pretty =
        serde_json::to_string_pretty(&serde_json::from_str::<Value>(&search_response()).unwrap())
            .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        format!(
            "=== websearch options ===\ntopic: rust\nmax_results: 2\nfetch_content: false\n\
             base_url: {url}\n=== websearch response ===\n{pretty}\n"
        )
    );
}

#[test]
fn returns_a_response_that_is_not_json_as_it_is() {
    let (url, _searches) = serve(vec![(200, "<html>busy</html>".to_string())]);
    let options = options(&format!("websearch topic=rust base_url={url}"));
    let mut out = Vec::new();

    assert_eq!(
        options.search(&tavily_env, Some(&mut out)),
        Ok("<html>busy</html>".to_string())
    );
    let out = String::from_utf8(out).unwrap();
    assert!(
        out.ends_with("=== websearch response ===\n<html>busy</html>\n"),
        "{out}"
    );
}

#[test]
fn fails_without_a_key_before_the_search() {
    // Nothing listens on this port, so a request would fail with another error.
    let options = options(&format!("websearch topic=rust base_url={}", closed_url()));
    assert_eq!(
        options.search(&no_env, None),
        Err("no Tavily API key: set `TAVILY_API_KEY`".to_string())
    );
}

#[test]
fn fails_when_the_search_fails() {
    let (url, _searches) = serve(vec![(401, "bad key".to_string())]);
    let options = options(&format!("websearch topic=rust base_url={url}"));
    assert_eq!(
        options.search(&tavily_env, None),
        Err(format!(
            "`{url}/search` responded with 401 Unauthorized: bad key"
        ))
    );
}

#[test]
fn the_actor_runs_the_search_and_writes_debug_output() {
    // The actor reads the real environment, which may or may not have a
    // key, so this test only checks that the actor runs the search. Nothing
    // listens on the port, so the search fails in both cases.
    let actor: Actor = format!("websearch topic=rust base_url={}", closed_url())
        .parse()
        .unwrap();
    let mut out = Vec::new();
    assert!(actor.act_with("ignored input", Some(&mut out)).is_err());
    let out = String::from_utf8(out).unwrap();
    assert!(
        out.starts_with("=== websearch options ===\ntopic: rust\n"),
        "{out}"
    );
}
