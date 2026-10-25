//! Tests for the Tavily search API client in `src/tavily.rs`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use serde_json::json;

use atama::tavily::{API_KEY_ENV, DEFAULT_BASE_URL, SearchConfig, api_key};

/// Starts a local HTTP server that answers one request per response, in order.
///
/// Returns the server's base URL and a receiver that gets each raw request.
fn serve<B: AsRef<[u8]> + Send + 'static>(responses: Vec<(u16, B)>) -> (String, Receiver<String>) {
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
                body.as_ref().len()
            )
            .unwrap();
            stream.write_all(body.as_ref()).unwrap();
        }
    });

    (url, received)
}

/// An environment that has only the Tavily API key `key-1`.
fn with_key(name: &str) -> Option<String> {
    (name == API_KEY_ENV).then(|| "key-1".to_string())
}

fn params(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| word.to_string()).collect()
}

fn config(query: &str) -> SearchConfig {
    SearchConfig::from_params(&params(&[query]), "", with_key).unwrap()
}

// Configuration

#[test]
fn reads_the_query_and_the_defaults() {
    let config =
        SearchConfig::from_params(&params(&["who", "is", "Messi?"]), "in", with_key).unwrap();
    assert_eq!(
        config,
        SearchConfig {
            base_url: DEFAULT_BASE_URL.into(),
            api_key: "key-1".into(),
            query: "who is Messi?".into(),
            max_results: None,
            search_depth: None,
            topic: None,
            include_answer: None,
            include_raw_content: None,
        }
    );
}

#[test]
fn reads_every_option() {
    let config = SearchConfig::from_params(
        &params(&[
            "base_url=http://x.test",
            "max_results=3",
            "search_depth=advanced",
            "topic=news",
            "include_answer=basic",
            "include_raw_content=markdown",
            "rust",
        ]),
        "",
        with_key,
    )
    .unwrap();
    assert_eq!(config.base_url, "http://x.test");
    assert_eq!(config.max_results, Some(3));
    assert_eq!(config.search_depth.as_deref(), Some("advanced"));
    assert_eq!(config.topic.as_deref(), Some("news"));
    assert_eq!(config.include_answer.as_deref(), Some("basic"));
    assert_eq!(config.include_raw_content.as_deref(), Some("markdown"));
    assert_eq!(config.query, "rust");
}

#[test]
fn uses_the_input_as_query_if_the_params_have_none() {
    let config =
        SearchConfig::from_params(&params(&["topic=news"]), "  rust 2.0\n", with_key).unwrap();
    assert_eq!(config.query, "rust 2.0");
}

#[test]
fn rejects_invalid_params() {
    let cases = [
        (
            vec!["max_results=", "q"],
            "option `max_results` has no value",
        ),
        (
            vec!["topic=a", "topic=b", "q"],
            "option `topic` is set twice",
        ),
        (
            vec!["max_results=many", "q"],
            "`max_results=many` is not a number",
        ),
        (
            vec!["max_results=-1", "q"],
            "`max_results=-1` is not a number",
        ),
        (
            vec![],
            "`tavily_search` needs a query in its params or an input",
        ),
    ];
    for (words, message) in cases {
        assert_eq!(
            SearchConfig::from_params(&params(&words), " ", with_key),
            Err(message.to_string()),
            "{words:?}"
        );
    }
}

#[test]
fn needs_an_api_key() {
    let message = format!("no Tavily API key: set `{API_KEY_ENV}`");
    let no_env = |_: &str| None;
    assert_eq!(
        SearchConfig::from_params(&params(&["q"]), "", no_env),
        Err(message.clone())
    );
    let empty = |_: &str| Some(String::new());
    assert_eq!(api_key(empty), Err(message));
    assert_eq!(api_key(with_key), Ok("key-1".to_string()));
}

#[test]
fn request_body_has_only_the_options_that_are_set() {
    assert_eq!(config("rust").request_body(), json!({"query": "rust"}));

    let mut config = config("rust");
    config.max_results = Some(2);
    config.search_depth = Some("basic".into());
    config.topic = Some("finance".into());
    for (value, expected) in [
        ("true", json!(true)),
        ("false", json!(false)),
        ("advanced", json!("advanced")),
    ] {
        config.include_answer = Some(value.into());
        assert_eq!(
            config.request_body(),
            json!({
                "query": "rust",
                "max_results": 2,
                "search_depth": "basic",
                "topic": "finance",
                "include_answer": expected,
            })
        );
    }

    let mut raw = self::config("rust");
    for (value, expected) in [
        ("true", json!(true)),
        ("false", json!(false)),
        ("markdown", json!("markdown")),
    ] {
        raw.include_raw_content = Some(value.into());
        assert_eq!(
            raw.request_body(),
            json!({"query": "rust", "include_raw_content": expected})
        );
    }
}

// Search

#[test]
fn search_posts_the_body_with_the_bearer_key() {
    let (url, requests) = serve(vec![(200, r#"{"results":[]}"#.to_string())]);
    let mut config = config("rust");
    config.base_url = format!("{url}/");

    assert_eq!(config.search(), Ok(r#"{"results":[]}"#.to_string()));
    let request = requests.recv().unwrap();
    assert!(request.starts_with("POST /search "), "{request}");
    assert!(request.contains("authorization: Bearer key-1"), "{request}");
    assert!(
        request.contains("content-type: application/json"),
        "{request}"
    );
    assert!(request.ends_with(r#"{"query":"rust"}"#), "{request}");
}

#[test]
fn search_fails_on_error_status() {
    let (url, _requests) = serve(vec![(401, "bad key\n".to_string())]);
    let mut config = config("rust");
    config.base_url = url.clone();
    assert_eq!(
        config.search(),
        Err(format!(
            "`{url}/search` responded with 401 Unauthorized: bad key"
        ))
    );
}

#[test]
fn search_fails_on_a_response_that_is_not_utf8() {
    let (url, _requests) = serve(vec![(200, vec![0xff, 0xfe])]);
    let mut config = config("rust");
    config.base_url = url.clone();
    let err = config.search().unwrap_err();
    assert!(
        err.starts_with(&format!("cannot read response from `{url}/search`")),
        "{err}"
    );
}

#[test]
fn search_fails_when_the_server_is_unreachable() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);

    let mut config = config("rust");
    config.base_url = url.clone();
    let err = config.search().unwrap_err();
    assert!(
        err.starts_with(&format!("request to `{url}/search` failed")),
        "{err}"
    );
}
