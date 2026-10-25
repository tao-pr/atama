//! Tests for the OpenAI compatible agent loop of `agent:<type>` actions.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use serde_json::{Value, json};

use atama::AgentToolLoop;
use atama::actor::AgentType;
use atama::agent_loop_openai_compat::agent_options::{
    DEFAULT_EXPERTISE, DEFAULT_MAX_RESULTS, DEFAULT_SUMMARIZER_PROMPT, DEFAULT_WEBSEARCH_PROMPT,
};
use atama::agent_loop_openai_compat::openai::{OpenAiClient, parse_reply, request_body};
use atama::agent_loop_openai_compat::tools::{
    Bash, FetchUrl, MAX_RESULT_CHARS, TavilySearch, default_tools, tools_for, truncate,
};
use atama::agent_loop_openai_compat::{ChatClient, Message, Reply, Tool, ToolCall};

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

/// A URL where nothing listens, so connections to it are refused.
fn closed_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    format!("http://{}", listener.local_addr().unwrap())
}

/// The JSON body of a raw HTTP request.
fn request_json(request: &str) -> Value {
    let (_, body) = request.split_once("\r\n\r\n").expect("request has a body");
    serde_json::from_str(body).expect("request body is JSON")
}

/// A chat completions response with a text reply.
fn text_response(content: &str) -> String {
    json!({"choices": [{"message": {"role": "assistant", "content": content}}]}).to_string()
}

/// A chat completions response with one tool call.
fn tool_call_response(id: &str, name: &str, arguments: Value) -> String {
    json!({"choices": [{"message": {
        "role": "assistant",
        "content": null,
        "tool_calls": [{
            "id": id,
            "type": "function",
            "function": {"name": name, "arguments": arguments.to_string()},
        }],
    }}]})
    .to_string()
}

fn params(params: &[&str]) -> Vec<String> {
    params.iter().map(|param| param.to_string()).collect()
}

fn text(content: &str) -> Reply {
    Reply {
        content: Some(content.into()),
        tool_calls: Vec::new(),
    }
}

fn calls(calls: &[(&str, &str, &str)]) -> Reply {
    Reply {
        content: None,
        tool_calls: calls
            .iter()
            .map(|(id, name, arguments)| ToolCall {
                id: id.to_string(),
                name: name.to_string(),
                arguments: arguments.to_string(),
            })
            .collect(),
    }
}

/// Client that returns scripted replies and records each conversation that
/// it gets.
#[derive(Default)]
struct ScriptedClient {
    replies: VecDeque<Result<Reply, String>>,
    conversations: Vec<Vec<Message>>,
    tool_names: Vec<Vec<String>>,
}

impl ScriptedClient {
    fn new(replies: Vec<Result<Reply, String>>) -> Self {
        ScriptedClient {
            replies: replies.into(),
            ..Default::default()
        }
    }
}

impl ChatClient for ScriptedClient {
    fn chat(&mut self, messages: &[Message], tools: &[Box<dyn Tool>]) -> Result<Reply, String> {
        self.conversations.push(messages.to_vec());
        self.tool_names
            .push(tools.iter().map(|tool| tool.name().to_string()).collect());
        self.replies
            .pop_front()
            .expect("the loop asked for more replies than the script has")
    }
}

/// Tool that records its arguments and returns a scripted result.
struct Recorder {
    name: &'static str,
    result: Result<String, String>,
    arguments: Rc<RefCell<Vec<Value>>>,
}

impl Recorder {
    fn boxed(
        name: &'static str,
        result: Result<&str, &str>,
    ) -> (Box<dyn Tool>, Rc<RefCell<Vec<Value>>>) {
        let arguments = Rc::new(RefCell::new(Vec::new()));
        let tool = Recorder {
            name,
            result: result.map(str::to_string).map_err(str::to_string),
            arguments: Rc::clone(&arguments),
        };
        (Box::new(tool), arguments)
    }
}

impl Tool for Recorder {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "records its arguments"
    }

    fn parameters(&self) -> Value {
        json!({"type": "object"})
    }

    fn call(&mut self, arguments: &Value) -> Result<String, String> {
        self.arguments.borrow_mut().push(arguments.clone());
        self.result.clone()
    }
}

fn no_env(_: &str) -> Option<String> {
    None
}

// Loop

fn names(tools: &[Box<dyn Tool>]) -> Vec<String> {
    tools.iter().map(|tool| tool.name().to_string()).collect()
}

#[test]
fn has_the_default_tools_of_its_agent_type() {
    for agent in [
        AgentType::WebSearch,
        AgentType::Coder,
        AgentType::Summarizer,
    ] {
        let agent_loop = AgentToolLoop::new(agent);
        assert_eq!(agent_loop.agent(), agent);
        assert_eq!(agent_loop.tool_names(), names(&default_tools(agent)));
    }
}

#[test]
fn websearch_gets_tavily_search_only_with_an_api_key() {
    let no_env = |_: &str| None;
    let with_key = |name: &str| (name == "TAVILY_API_KEY").then(|| "key".to_string());
    let empty_key = |name: &str| (name == "TAVILY_API_KEY").then(String::new);
    let cases = [
        (tools_for(AgentType::WebSearch, no_env), vec!["fetch_url"]),
        (
            tools_for(AgentType::WebSearch, empty_key),
            vec!["fetch_url"],
        ),
        (
            tools_for(AgentType::WebSearch, with_key),
            vec!["fetch_url", "tavily_search"],
        ),
        (tools_for(AgentType::Coder, with_key), vec!["bash"]),
        (tools_for(AgentType::Summarizer, with_key), vec![]),
    ];
    for (tools, expected) in cases {
        assert_eq!(names(&tools), expected);
    }
}

#[test]
fn returns_the_reply_when_the_model_calls_no_tools() {
    let mut client = ScriptedClient::new(vec![Ok(text("the answer"))]);
    let output = AgentToolLoop::new(AgentType::WebSearch)
        .run_with(&mut client, "find it", "the input")
        .unwrap();

    assert_eq!(output, "the answer");
    let [system, user] = client.conversations[0].as_slice() else {
        panic!("expected a system and a user message");
    };
    assert!(
        matches!(system, Message::System(prompt) if prompt.contains("web research agent")),
        "{system:?}"
    );
    assert_eq!(*user, Message::User("find it\n\nInput:\nthe input".into()));
    // The tools depend on `TAVILY_API_KEY` (see `tools_for`), so compare
    // with the default tools of this process, not a fixed list.
    assert_eq!(
        client.tool_names[0],
        names(&default_tools(AgentType::WebSearch))
    );
}

#[test]
fn sends_only_the_task_or_only_the_input() {
    let cases = [("task", "  \n", "task"), ("", "input\n", "input")];
    for (task, input, prompt) in cases {
        let mut client = ScriptedClient::new(vec![Ok(text("ok"))]);
        AgentToolLoop::new(AgentType::Coder)
            .run_with(&mut client, task, input)
            .unwrap();
        assert_eq!(client.conversations[0][1], Message::User(prompt.into()));
        assert!(
            matches!(&client.conversations[0][0], Message::System(p) if p.contains("coding agent"))
        );
    }
}

#[test]
fn returns_empty_output_for_a_reply_without_text() {
    let mut client = ScriptedClient::new(vec![Ok(Reply::default())]);
    let output = AgentToolLoop::new(AgentType::Coder)
        .run_with(&mut client, "task", "")
        .unwrap();
    assert_eq!(output, "");
}

#[test]
fn fails_without_a_task_or_an_input() {
    let mut client = ScriptedClient::new(Vec::new());
    let err = AgentToolLoop::new(AgentType::WebSearch)
        .run_with(&mut client, "", " \n")
        .unwrap_err();
    assert_eq!(err, "`agent:websearch` needs `task=` or an input");
    assert!(client.conversations.is_empty());
}

#[test]
fn runs_tool_calls_and_sends_the_results_back() {
    let (tool, arguments) = Recorder::boxed("lookup", Ok("found"));
    let mut client = ScriptedClient::new(vec![
        Ok(calls(&[
            ("call-1", "lookup", r#"{"q":"a"}"#),
            ("call-2", "lookup", ""),
        ])),
        Ok(text("done")),
    ]);
    let output = AgentToolLoop::new(AgentType::WebSearch)
        .with_tools(vec![tool])
        .run_with(&mut client, "task", "")
        .unwrap();

    assert_eq!(output, "done");
    assert_eq!(*arguments.borrow(), [json!({"q": "a"}), json!({})]);
    let second = &client.conversations[1];
    assert_eq!(second.len(), 5);
    assert_eq!(
        second[2],
        Message::Assistant(calls(&[
            ("call-1", "lookup", r#"{"q":"a"}"#),
            ("call-2", "lookup", ""),
        ]))
    );
    assert_eq!(
        second[3],
        Message::Tool {
            call_id: "call-1".into(),
            content: "found".into(),
        }
    );
    assert_eq!(
        second[4],
        Message::Tool {
            call_id: "call-2".into(),
            content: "found".into(),
        }
    );
}

#[test]
fn sends_tool_errors_to_the_model_and_goes_on() {
    let (failing, _) = Recorder::boxed("failing", Err("it broke"));
    let mut client = ScriptedClient::new(vec![
        Ok(calls(&[
            ("1", "failing", "{}"),
            ("2", "missing", "{}"),
            ("3", "failing", "not json"),
        ])),
        Ok(text("recovered")),
    ]);
    let output = AgentToolLoop::new(AgentType::Coder)
        .with_tools(vec![failing])
        .run_with(&mut client, "task", "")
        .unwrap();

    assert_eq!(output, "recovered");
    let results: Vec<&str> = client.conversations[1][3..]
        .iter()
        .map(|message| match message {
            Message::Tool { content, .. } => content.as_str(),
            other => panic!("expected a tool result, got {other:?}"),
        })
        .collect();
    assert_eq!(results[0], "error: it broke");
    assert_eq!(results[1], "error: unknown tool `missing`");
    assert!(
        results[2].starts_with("error: arguments of `failing` are not JSON"),
        "{}",
        results[2]
    );
}

#[test]
fn fails_when_the_model_does_not_finish_in_max_steps() {
    let (tool, arguments) = Recorder::boxed("lookup", Ok("found"));
    let mut client = ScriptedClient::new(vec![
        Ok(calls(&[("1", "lookup", "{}")])),
        Ok(calls(&[("2", "lookup", "{}")])),
    ]);
    let err = AgentToolLoop::new(AgentType::WebSearch)
        .with_tools(vec![tool])
        .max_steps(2)
        .run_with(&mut client, "task", "")
        .unwrap_err();

    assert_eq!(err, "`agent:websearch` did not finish in 2 steps");
    assert_eq!(arguments.borrow().len(), 2);
}

#[test]
fn fails_when_the_client_fails() {
    let mut client = ScriptedClient::new(vec![Err("model is down".into())]);
    let err = AgentToolLoop::new(AgentType::Coder)
        .run_with(&mut client, "task", "")
        .unwrap_err();
    assert_eq!(err, "model is down");
}

// OpenAI compatible client

#[test]
fn request_body_has_model_messages_and_tools() {
    let messages = [
        Message::System("sys".into()),
        Message::User("task".into()),
        Message::Assistant(Reply {
            content: Some("thinking".into()),
            tool_calls: calls(&[("c1", "bash", r#"{"command":"ls"}"#)]).tool_calls,
        }),
        Message::Tool {
            call_id: "c1".into(),
            content: "a.txt".into(),
        },
        Message::Assistant(text("done")),
    ];
    let body = request_body("m", &messages, &default_tools(AgentType::Coder));
    assert_eq!(
        body,
        json!({
            "model": "m",
            "messages": [
                {"role": "system", "content": "sys"},
                {"role": "user", "content": "task"},
                {"role": "assistant", "content": "thinking", "tool_calls": [{
                    "id": "c1",
                    "type": "function",
                    "function": {"name": "bash", "arguments": "{\"command\":\"ls\"}"},
                }]},
                {"role": "tool", "tool_call_id": "c1", "content": "a.txt"},
                {"role": "assistant", "content": "done"},
            ],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "bash",
                    "description": Bash.description(),
                    "parameters": Bash.parameters(),
                },
            }],
        })
    );

    let body = request_body("m", &messages[..1], &[]);
    assert!(body.get("tools").is_none(), "{body}");
}

#[test]
fn parses_text_and_tool_call_replies() {
    let reply = parse_reply(&serde_json::from_str(&text_response("hi")).unwrap()).unwrap();
    assert_eq!(reply, text("hi"));

    let json =
        serde_json::from_str(&tool_call_response("c1", "bash", json!({"command": "ls"}))).unwrap();
    assert_eq!(
        parse_reply(&json).unwrap(),
        calls(&[("c1", "bash", r#"{"command":"ls"}"#)])
    );

    let incomplete = json!({"choices": [{"message": {"tool_calls": [{}]}}]});
    assert_eq!(parse_reply(&incomplete).unwrap(), calls(&[("", "", "")]));
    assert_eq!(parse_reply(&json!({"choices": []})), None);
    assert_eq!(parse_reply(&json!({})), None);
}

#[test]
fn client_posts_to_chat_completions_with_the_api_key() {
    let (url, requests) = serve(vec![(200, text_response("hi")), (200, text_response("hi"))]);
    let messages = [Message::User("task".into())];

    let mut client = OpenAiClient::new(&format!("{url}/v1/"), "m", Some("secret".into()));
    assert_eq!(client.chat(&messages, &[]), Ok(text("hi")));
    let request = requests.recv().unwrap();
    assert!(
        request.starts_with("POST /v1/chat/completions "),
        "{request}"
    );
    assert!(
        request
            .to_lowercase()
            .contains("authorization: bearer secret"),
        "{request}"
    );
    assert!(
        request
            .to_lowercase()
            .contains("content-type: application/json"),
        "{request}"
    );
    assert_eq!(request_json(&request), request_body("m", &messages, &[]));

    let mut client = OpenAiClient::new(&url, "m", None);
    client.chat(&messages, &[]).unwrap();
    let request = requests.recv().unwrap();
    assert!(
        !request.to_lowercase().contains("authorization"),
        "{request}"
    );
}

#[test]
fn client_fails_on_bad_responses() {
    let (url, _requests) = serve(vec![
        (401, "bad key".to_string()),
        (200, "not json".to_string()),
        (200, "{}".to_string()),
    ]);
    let endpoint = format!("{url}/chat/completions");
    let mut client = OpenAiClient::new(&url, "m", None);
    let messages = [Message::User("task".into())];

    let err = client.chat(&messages, &[]).unwrap_err();
    assert_eq!(
        err,
        format!("`{endpoint}` responded with 401 Unauthorized: bad key")
    );
    let err = client.chat(&messages, &[]).unwrap_err();
    assert!(
        err.starts_with(&format!("response from `{endpoint}` is not JSON")),
        "{err}"
    );
    let err = client.chat(&messages, &[]).unwrap_err();
    assert_eq!(err, format!("response from `{endpoint}` has no message"));

    let url = closed_url();
    let err = OpenAiClient::new(&url, "m", None)
        .chat(&messages, &[])
        .unwrap_err();
    assert!(
        err.starts_with(&format!("request to `{url}/chat/completions` failed")),
        "{err}"
    );
}

/// An environment that has only the Tavily API key `key-1`.
fn tavily_env(name: &str) -> Option<String> {
    (name == "TAVILY_API_KEY").then(|| "key-1".to_string())
}

/// A Tavily response with one result.
fn search_response() -> String {
    json!({"results": [{"title": "Rust 2.0", "url": "https://r.test", "content": "Released."}]})
        .to_string()
}

#[test]
fn websearch_searches_the_topic_then_the_model_analyses_the_results() {
    let (page_url, pages) = serve(vec![(200, "page text".to_string())]);
    let (search_url, searches) = serve(vec![(200, search_response())]);
    let (url, requests) = serve(vec![
        (
            200,
            tool_call_response(
                "c1",
                "fetch_url",
                json!({"url": format!("{page_url}/news")}),
            ),
        ),
        (200, text_response("summary")),
    ]);

    let output = AgentToolLoop::new(AgentType::WebSearch)
        .search_base_url(&search_url)
        .run_loop_with_env(
            &params(&[
                "model=m",
                &format!("base_url={url}"),
                "topic=rust release",
                "task=list the changes",
            ]),
            "the input",
            &tavily_env,
        );

    assert_eq!(output, Ok("summary".to_string()));
    let search = searches.recv().unwrap();
    assert!(search.starts_with("POST /search "), "{search}");
    assert!(search.contains("authorization: Bearer key-1"), "{search}");
    assert_eq!(
        request_json(&search),
        json!({"query": "rust release", "max_results": DEFAULT_MAX_RESULTS})
    );
    assert!(pages.recv().unwrap().starts_with("GET /news "));
    let first = request_json(&requests.recv().unwrap());
    assert_eq!(first["model"], "m");
    assert_eq!(
        first["messages"][1]["content"],
        "list the changes\n\nTopic: rust release\n\nSearch results:\n\
         1. Rust 2.0\nhttps://r.test\nReleased.\n\nInput:\nthe input"
    );
    let second = request_json(&requests.recv().unwrap());
    assert_eq!(
        second["messages"][3],
        json!({"role": "tool", "tool_call_id": "c1", "content": "page text"})
    );
}

#[test]
fn websearch_uses_max_results_and_the_default_prompt() {
    let (search_url, searches) = serve(vec![(200, search_response())]);
    let (url, requests) = serve(vec![(200, text_response("summary"))]);
    let mut out = Vec::new();

    AgentToolLoop::new(AgentType::WebSearch)
        .search_base_url(&search_url)
        .debug(&mut out)
        .run_loop_with_env(
            &params(&[
                "model=m",
                &format!("base_url={url}"),
                "topic=rust",
                "max_results=2",
            ]),
            "",
            &tavily_env,
        )
        .unwrap();

    assert_eq!(request_json(&searches.recv().unwrap())["max_results"], 2);
    let first = request_json(&requests.recv().unwrap());
    let prompt = first["messages"][1]["content"].as_str().unwrap();
    assert!(
        prompt.starts_with(&format!("{DEFAULT_WEBSEARCH_PROMPT}\n\nTopic: rust\n")),
        "{prompt}"
    );
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("\ntopic: rust\nmax_results: 2\n"), "{out}");
    let pretty =
        serde_json::to_string_pretty(&serde_json::from_str::<Value>(&search_response()).unwrap())
            .unwrap();
    let response = format!("=== agent:websearch search response ===\n{pretty}\n");
    let results =
        "=== agent:websearch search: rust (max_results 2) ===\n1. Rust 2.0\nhttps://r.test\n";
    let response_at = out.find(&response).expect(&out);
    let results_at = out.find(results).expect(&out);
    assert!(response_at < results_at, "{out}");
}

#[test]
fn websearch_debug_writes_a_search_response_that_is_not_json_as_it_is() {
    let (search_url, _searches) = serve(vec![(200, "<html>busy</html>".to_string())]);
    let mut out = Vec::new();
    let err = AgentToolLoop::new(AgentType::WebSearch)
        .search_base_url(&search_url)
        .debug(&mut out)
        .run_loop_with_env(&params(&["model=m", "topic=rust"]), "", &tavily_env)
        .unwrap_err();

    assert!(err.starts_with("Tavily response is not JSON"), "{err}");
    let out = String::from_utf8(out).unwrap();
    assert!(
        out.contains("=== agent:websearch search response ===\n<html>busy</html>\n"),
        "{out}"
    );
}

#[test]
fn websearch_fails_before_asking_the_model_without_topic_or_key() {
    // Nothing listens on these ports, so a request would fail with another error.
    let url = closed_url();
    let cases = [
        (
            vec![
                "model=m".to_string(),
                format!("base_url={url}"),
                "task=t".into(),
            ],
            "`agent:websearch` needs a topic, as in `topic=\"latest Rust release\"`",
        ),
        (
            vec![
                "model=m".to_string(),
                format!("base_url={url}"),
                "topic=rust".into(),
            ],
            "no Tavily API key: set `TAVILY_API_KEY`",
        ),
    ];
    for (params, message) in cases {
        let err = AgentToolLoop::new(AgentType::WebSearch)
            .search_base_url(&url)
            .run_loop_with_env(&params, "", &no_env)
            .unwrap_err();
        assert_eq!(err, message);
    }
}

#[test]
fn websearch_fails_when_the_search_fails() {
    let (search_url, _searches) = serve(vec![(401, "bad key".to_string())]);
    let err = AgentToolLoop::new(AgentType::WebSearch)
        .search_base_url(&search_url)
        .run_loop_with_env(&params(&["model=m", "topic=rust"]), "", &tavily_env)
        .unwrap_err();
    assert_eq!(
        err,
        format!("`{search_url}/search` responded with 401 Unauthorized: bad key")
    );
}

#[test]
fn run_loop_uses_max_steps_from_the_params() {
    let (url, _requests) = serve(vec![(200, tool_call_response("c1", "unknown", json!({})))]);
    let err = AgentToolLoop::new(AgentType::Coder)
        .run_loop(
            &params(&[
                "model=m",
                &format!("base_url={url}"),
                "max_steps=1",
                "task=t",
            ]),
            "",
        )
        .unwrap_err();
    assert_eq!(err, "`agent:coder` did not finish in 1 steps");
}

#[test]
fn run_loop_fails_on_invalid_configuration() {
    let err = AgentToolLoop::new(AgentType::Coder)
        .run_loop(&params(&["model=a", "model=b"]), "")
        .unwrap_err();
    assert_eq!(err, "option `model` is set twice");
}

// Tools

#[test]
fn fetch_url_returns_the_response_body() {
    let (url, requests) = serve(vec![(200, "hello".to_string()), (404, "gone".to_string())]);
    let mut tool = FetchUrl;
    assert_eq!(tool.name(), "fetch_url");
    assert_eq!(tool.parameters()["required"], json!(["url"]));

    assert_eq!(
        tool.call(&json!({"url": format!("{url}/a")})),
        Ok("hello".into())
    );
    assert!(requests.recv().unwrap().starts_with("GET /a "));
    let err = tool.call(&json!({"url": url})).unwrap_err();
    assert!(err.contains("responded with 404"), "{err}");
    assert_eq!(
        tool.call(&json!({"link": "x"})),
        Err("missing string argument `url`".into())
    );
}

#[test]
fn tavily_search_tool_returns_the_results_as_text() {
    let body = json!({
        "answer": "Rust 2.0",
        "results": [{"title": "Rust Blog", "url": "https://r.test", "content": "News."}],
    })
    .to_string();
    let (url, requests) = serve(vec![(200, body), (500, "down".to_string())]);
    let mut tool = TavilySearch::new(&url, "key-1");
    assert_eq!(tool.name(), "tavily_search");
    assert!(tool.description().contains("fetch_url"));
    assert_eq!(tool.parameters()["required"], json!(["query"]));

    assert_eq!(
        tool.call(&json!({"query": "rust", "max_results": 1, "topic": "news"})),
        Ok("Answer: Rust 2.0\n\n1. Rust Blog\nhttps://r.test\nNews.".into())
    );
    let request = requests.recv().unwrap();
    assert!(request.starts_with("POST /search "), "{request}");
    assert!(request.contains("authorization: Bearer key-1"), "{request}");
    let sent: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(
        sent,
        json!({"query": "rust", "max_results": 1, "topic": "news", "include_answer": true})
    );

    let err = tool
        .call(&json!({"query": "rust", "max_results": null}))
        .unwrap_err();
    assert!(err.contains("responded with 500"), "{err}");
    assert_eq!(
        tool.call(&json!({"query": "rust", "max_results": -1})),
        Err("argument `max_results` is not a positive integer".into())
    );
    assert_eq!(
        tool.call(&json!({"q": "rust"})),
        Err("missing string argument `query`".into())
    );
}

#[test]
fn bash_tool_returns_stdout_or_the_error() {
    let mut tool = Bash;
    assert_eq!(tool.name(), "bash");
    assert_eq!(tool.parameters()["required"], json!(["command"]));

    assert_eq!(
        tool.call(&json!({"command": "echo one; echo two"})),
        Ok("one\ntwo\n".into())
    );
    let err = tool
        .call(&json!({"command": "echo oops >&2; exit 3"}))
        .unwrap_err();
    assert!(err.contains("oops"), "{err}");
    assert_eq!(
        tool.call(&json!({"command": 1})),
        Err("missing string argument `command`".into())
    );
}

#[test]
fn cuts_long_tool_results() {
    assert_eq!(truncate("short".into()), "short");
    let exact = "é".repeat(MAX_RESULT_CHARS);
    assert_eq!(truncate(exact.clone()), exact);

    let long = format!("{exact}tail");
    assert_eq!(
        truncate(long),
        format!("{exact}\n[cut: the result has more than {MAX_RESULT_CHARS} characters]")
    );
}

// Debug output

#[test]
fn debug_writes_requests_replies_and_tool_calls() {
    let (lookup, _) = Recorder::boxed("lookup", Ok("found it"));
    let (failing, _) = Recorder::boxed("failing", Err("it broke"));
    let mut client = ScriptedClient::new(vec![
        Ok(calls(&[
            ("call-1", "lookup", r#"{"q":"rust"}"#),
            ("call-2", "failing", "not json"),
        ])),
        Ok(text("the answer")),
    ]);
    let mut out = Vec::new();
    let output = AgentToolLoop::new(AgentType::WebSearch)
        .with_tools(vec![lookup, failing])
        .debug(&mut out)
        .run_with(&mut client, "find it", "")
        .unwrap();
    assert_eq!(output, "the answer");
    let out = String::from_utf8(out).unwrap();

    let blocks: Vec<&str> = out
        .lines()
        .filter(|line| line.starts_with("=== "))
        .collect();
    assert_eq!(
        blocks,
        [
            "=== agent:websearch step 1: request (2 messages, 2 new, tools: lookup, failing) ===",
            "=== agent:websearch step 1: response (2 tool calls) ===",
            "=== agent:websearch step 1: tool call 1 of 2: lookup (id call-1) ===",
            "=== agent:websearch step 1: tool call 2 of 2: failing (id call-2) ===",
            "=== agent:websearch step 2: request (5 messages, 3 new, tools: lookup, failing) ===",
            "=== agent:websearch step 2: response (0 tool calls) ===",
        ],
        "{out}"
    );
    // Messages are pretty JSON in the chat completions format.
    assert!(
        out.contains("    \"role\": \"user\",\n    \"content\": \"find it\"\n"),
        "{out}"
    );
    assert!(out.contains("\"content\": \"the answer\""), "{out}");
    // Tool calls show their arguments as pretty JSON and their result.
    assert!(
        out.contains("arguments:\n{\n  \"q\": \"rust\"\n}\nresult: ok, 8 characters\n"),
        "{out}"
    );
    assert!(
        out.contains("arguments:\nnot json\nresult: error: arguments of `failing` are not JSON"),
        "{out}"
    );
    // The second request sends only the reply and the tool results as new.
    let second = &out[out.find("step 2: request").unwrap()..];
    assert!(
        !second[..second.find("step 2: response").unwrap()].contains("find it"),
        "{out}"
    );
}

#[test]
fn debug_writes_the_configuration_without_the_api_key() {
    let (url, _requests) = serve(vec![(200, text_response("done"))]);
    let mut out = Vec::new();
    AgentToolLoop::new(AgentType::Coder)
        .debug(&mut out)
        .run_loop(
            &params(&[
                "model=m",
                &format!("base_url={url}"),
                "max_steps=4",
                "task=t",
            ]),
            "",
        )
        .unwrap();
    let out = String::from_utf8(out).unwrap();

    let expected = format!(
        "=== agent:coder configuration ===\nmodel: m\nbase_url: {url}\nmax_steps: 4\napi key: "
    );
    assert!(out.starts_with(&expected), "{out}");
    let key_line = out.lines().nth(4).unwrap();
    assert!(
        key_line == "api key: set" || key_line == "api key: not set",
        "{out}"
    );
}

// Summarizer

#[test]
fn summarizer_sends_the_input_without_tools_and_returns_the_summary() {
    let (url, requests) = serve(vec![(200, text_response("short summary"))]);
    let output = AgentToolLoop::new(AgentType::Summarizer).run_loop_with_env(
        &params(&[
            "model=m",
            &format!("base_url={url}"),
            "max_output=1 sentence",
        ]),
        "Rust 1.80 came out on 25 July 2024.",
        &no_env,
    );
    assert_eq!(output, Ok("short summary".to_string()));
    let request = request_json(&requests.recv().unwrap());
    assert_eq!(request.get("tools"), None);
    let system = request["messages"][0]["content"].as_str().unwrap();
    assert!(
        system.starts_with("You are a summarizing agent."),
        "{system}"
    );
    assert!(
        system.contains("numbers, quantities, dates or times"),
        "{system}"
    );
    assert_eq!(
        request["messages"][1]["content"],
        format!(
            "{DEFAULT_SUMMARIZER_PROMPT}\n\nExpertise: {DEFAULT_EXPERTISE}\n\
             Length of the summary: at most 1 sentence\n\n\
             Input:\nRust 1.80 came out on 25 July 2024."
        )
    );
}

#[test]
fn summarizer_fails_before_asking_the_model_without_an_input() {
    // Nothing listens on this port, so a request would fail with another error.
    let url = closed_url();
    let err = AgentToolLoop::new(AgentType::Summarizer)
        .run_loop_with_env(
            &params(&["model=m", &format!("base_url={url}")]),
            "",
            &no_env,
        )
        .unwrap_err();
    assert_eq!(err, "`agent:summarizer` needs an input to summarize");
}
