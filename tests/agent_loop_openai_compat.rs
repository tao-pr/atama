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
    DEFAULT_EXPERTISE, DEFAULT_SUMMARIZER_PROMPT,
};
use atama::agent_loop_openai_compat::openai::{OpenAiClient, parse_reply, request_body};
use atama::agent_loop_openai_compat::tools::{
    Bash, ListFiles, MAX_RESULT_CHARS, ReadFile, TOOL_NAMES, WriteFile, default_tool_names,
    default_tools, tool_by_name, truncate,
};
use atama::agent_loop_openai_compat::{ChatClient, Message, Reply, Tool, ToolCall};
use atama::profile::ModelProfile;
use atama::schema::OutputSchema;

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

/// A chat completions response with a text reply that used `tokens` tokens.
fn text_response_with_usage(content: &str, tokens: u64) -> String {
    json!({
        "choices": [{"message": {"role": "assistant", "content": content}}],
        "usage": {"prompt_tokens": tokens - 1, "completion_tokens": 1, "total_tokens": tokens},
    })
    .to_string()
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

/// The model profile `m`: the model `model-1` on the server at `base_url`,
/// without an API key.
fn profile(base_url: &str) -> ModelProfile {
    ModelProfile {
        name: "m".into(),
        model: "model-1".into(),
        base_url: base_url.into(),
        api_key_env: None,
        token_budget: None,
    }
}

fn params(params: &[&str]) -> Vec<String> {
    params.iter().map(|param| param.to_string()).collect()
}

fn text(content: &str) -> Reply {
    Reply {
        content: Some(content.into()),
        tool_calls: Vec::new(),
        tokens: None,
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
        tokens: None,
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
    for agent in [AgentType::Coder, AgentType::Summarizer] {
        let agent_loop = AgentToolLoop::new(agent);
        assert_eq!(agent_loop.agent(), agent);
        assert_eq!(agent_loop.tool_names(), names(&default_tools(agent)));
    }
}

#[test]
fn only_coder_gets_tools() {
    let cases = [
        (AgentType::Coder, vec!["bash"]),
        (AgentType::Summarizer, vec![]),
    ];
    for (agent, expected) in cases {
        assert_eq!(names(&default_tools(agent)), expected, "{agent}");
    }
}

// Tool registry

#[test]
fn the_registry_makes_each_tool_by_its_name() {
    assert_eq!(
        TOOL_NAMES,
        ["bash", "read_file", "write_file", "list_files"]
    );
    for name in TOOL_NAMES {
        let tool = tool_by_name(name).expect(name);
        assert_eq!(tool.name(), name);
        assert!(!tool.description().is_empty(), "{name}");
        assert_eq!(tool.parameters()["type"], "object", "{name}");
    }
    assert!(tool_by_name("rm").is_none());
}

#[test]
fn default_tools_come_from_the_registry() {
    assert_eq!(default_tool_names(AgentType::Coder), ["bash"]);
    assert!(default_tool_names(AgentType::Summarizer).is_empty());
    for agent in [AgentType::Coder, AgentType::Summarizer] {
        assert_eq!(names(&default_tools(agent)), default_tool_names(agent));
    }
}

/// Makes the fresh directory `<target tmp>/<name>` and returns its path.
fn fresh_dir(name: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn file_tools_write_read_and_list_files() {
    let dir = fresh_dir("agent_file_tools");
    let path = dir.join("sub/a.txt");
    let path = path.to_str().unwrap();

    let result = WriteFile
        .call(&json!({"path": path, "content": "héllo"}))
        .unwrap();
    assert_eq!(result, format!("wrote 5 characters to `{path}`"));
    assert_eq!(ReadFile.call(&json!({"path": path})).unwrap(), "héllo");

    std::fs::write(dir.join("sub/b.md"), "x").unwrap();
    let sub = dir.join("sub");
    let sub = sub.to_str().unwrap();
    assert_eq!(
        ListFiles.call(&json!({"dir": sub})).unwrap(),
        format!("{sub}/a.txt\n{sub}/b.md\n")
    );
    assert_eq!(
        ListFiles
            .call(&json!({"dir": sub, "pattern": "*.md"}))
            .unwrap(),
        format!("{sub}/b.md\n")
    );
}

#[test]
fn file_tools_fail_on_missing_or_wrong_arguments() {
    let cases: [(Box<dyn Tool>, Value, &str); 5] = [
        (
            Box::new(ReadFile),
            json!({}),
            "missing string argument `path`",
        ),
        (
            Box::new(WriteFile),
            json!({"content": "x"}),
            "missing string argument `path`",
        ),
        (
            Box::new(WriteFile),
            json!({"path": "x"}),
            "missing string argument `content`",
        ),
        (
            Box::new(ListFiles),
            json!({"pattern": "*"}),
            "missing string argument `dir`",
        ),
        (
            Box::new(ListFiles),
            json!({"dir": ".", "pattern": 1}),
            "missing string argument `pattern`",
        ),
    ];
    for (mut tool, arguments, expected) in cases {
        assert_eq!(tool.call(&arguments).unwrap_err(), expected, "{arguments}");
    }
}

#[test]
fn read_file_fails_on_a_missing_file() {
    let dir = fresh_dir("agent_read_missing");
    let path = dir.join("none.txt");
    let err = ReadFile
        .call(&json!({"path": path.to_str().unwrap()}))
        .unwrap_err();
    assert!(err.contains("none.txt"), "{err}");
}

#[test]
fn returns_the_reply_when_the_model_calls_no_tools() {
    let mut client = ScriptedClient::new(vec![Ok(text("the answer"))]);
    let output = AgentToolLoop::new(AgentType::Coder)
        .run_with(&mut client, "find it", "the input")
        .unwrap();

    assert_eq!(output, "the answer");
    let [system, user] = client.conversations[0].as_slice() else {
        panic!("expected a system and a user message");
    };
    assert!(
        matches!(system, Message::System(prompt) if prompt.contains("coding agent")),
        "{system:?}"
    );
    assert_eq!(*user, Message::User("find it\n\nInput:\nthe input".into()));
    assert_eq!(client.tool_names[0], vec!["bash"]);
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
    let err = AgentToolLoop::new(AgentType::Coder)
        .run_with(&mut client, "", " \n")
        .unwrap_err();
    assert_eq!(err, "`agent:coder` needs `task=` or an input");
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
    let output = AgentToolLoop::new(AgentType::Coder)
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
    let err = AgentToolLoop::new(AgentType::Coder)
        .with_tools(vec![tool])
        .max_steps(2)
        .run_with(&mut client, "task", "")
        .unwrap_err();

    assert_eq!(err, "`agent:coder` did not finish in 2 steps");
    assert_eq!(arguments.borrow().len(), 2);
}

/// `reply` with a token count, as from `usage.total_tokens`.
fn with_tokens(reply: Reply, tokens: u64) -> Reply {
    Reply {
        tokens: Some(tokens),
        ..reply
    }
}

#[test]
fn stops_before_a_request_when_the_run_used_its_token_budget() {
    let (tool, arguments) = Recorder::boxed("lookup", Ok("found"));
    let mut client = ScriptedClient::new(vec![
        Ok(with_tokens(calls(&[("1", "lookup", "{}")]), 60)),
        Ok(with_tokens(calls(&[("2", "lookup", "{}")]), 50)),
    ]);
    let err = AgentToolLoop::new(AgentType::Coder)
        .with_tools(vec![tool])
        .token_budget(Some(100))
        .run_with(&mut client, "task", "")
        .unwrap_err();

    assert_eq!(
        err,
        "`agent:coder` used 110 tokens in 2 steps, and its token budget is 100"
    );
    // The tool calls of the last reply still run.
    assert_eq!(arguments.borrow().len(), 2);
    assert_eq!(client.conversations.len(), 2);
}

#[test]
fn returns_a_final_reply_that_goes_over_the_token_budget() {
    let (tool, _) = Recorder::boxed("lookup", Ok("found"));
    let mut client = ScriptedClient::new(vec![
        Ok(with_tokens(calls(&[("1", "lookup", "{}")]), 60)),
        Ok(with_tokens(text("done"), 70)),
    ]);
    let output = AgentToolLoop::new(AgentType::Coder)
        .with_tools(vec![tool])
        .token_budget(Some(100))
        .run_with(&mut client, "task", "")
        .unwrap();
    assert_eq!(output, "done");
}

#[test]
fn fails_with_a_token_budget_when_the_server_sends_no_token_count() {
    let mut client = ScriptedClient::new(vec![Ok(text("done"))]);
    let err = AgentToolLoop::new(AgentType::Coder)
        .token_budget(Some(100))
        .run_with(&mut client, "task", "")
        .unwrap_err();
    assert_eq!(
        err,
        "`agent:coder` has a token budget of 100, but the server did not send \
         `usage.total_tokens` in its response"
    );

    // Without a budget, a reply without a token count is fine.
    let mut client = ScriptedClient::new(vec![Ok(text("done"))]);
    let output = AgentToolLoop::new(AgentType::Coder)
        .run_with(&mut client, "task", "")
        .unwrap();
    assert_eq!(output, "done");
}

#[test]
fn debug_writes_the_tokens_of_each_reply_and_the_used_budget() {
    let (tool, _) = Recorder::boxed("lookup", Ok("found"));
    let mut client = ScriptedClient::new(vec![
        Ok(with_tokens(calls(&[("1", "lookup", "{}")]), 60)),
        Ok(with_tokens(text("done"), 30)),
    ]);
    let mut out = Vec::new();
    AgentToolLoop::new(AgentType::Coder)
        .with_tools(vec![tool])
        .token_budget(Some(100))
        .debug(&mut out)
        .run_with(&mut client, "task", "")
        .unwrap();
    let out = String::from_utf8(out).unwrap();
    assert!(
        out.contains(
            "=== agent:coder step 1: response (1 tool calls, 60 tokens, 60 of 100 tokens used) ==="
        ),
        "{out}"
    );
    assert!(
        out.contains(
            "=== agent:coder step 2: response (0 tool calls, 30 tokens, 90 of 100 tokens used) ==="
        ),
        "{out}"
    );

    // Without a budget, the header has only the tokens of the reply.
    let mut client = ScriptedClient::new(vec![Ok(with_tokens(text("done"), 30))]);
    let mut out = Vec::new();
    AgentToolLoop::new(AgentType::Coder)
        .debug(&mut out)
        .run_with(&mut client, "task", "")
        .unwrap();
    let out = String::from_utf8(out).unwrap();
    assert!(
        out.contains("=== agent:coder step 1: response (0 tool calls, 30 tokens) ==="),
        "{out}"
    );
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
            tokens: None,
        }),
        Message::Tool {
            call_id: "c1".into(),
            content: "a.txt".into(),
        },
        Message::Assistant(text("done")),
    ];
    let body = request_body("m", &messages, &default_tools(AgentType::Coder), None, None);
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

    let body = request_body("m", &messages[..1], &[], None, None);
    assert!(body.get("tools").is_none(), "{body}");
}

#[test]
fn request_body_turns_off_thinking_only_for_thinking_false() {
    let messages = [Message::User("task".into())];
    let body = request_body("m", &messages, &[], Some(false), None);
    assert_eq!(body["reasoning_effort"], "none");
    for thinking in [None, Some(true)] {
        let body = request_body("m", &messages, &[], thinking, None);
        assert!(body.get("reasoning_effort").is_none(), "{body}");
    }
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

    let json = serde_json::from_str(&text_response_with_usage("hi", 42)).unwrap();
    assert_eq!(parse_reply(&json).unwrap(), with_tokens(text("hi"), 42));

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
    assert_eq!(
        request_json(&request),
        request_body("m", &messages, &[], None, None)
    );

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

#[test]
fn run_loop_uses_max_steps_from_the_params() {
    let (url, _requests) = serve(vec![(200, tool_call_response("c1", "unknown", json!({})))]);
    let err = AgentToolLoop::new(AgentType::Coder)
        .run_loop(
            &params(&["model=m", "max_steps=1", "task=t"]),
            Some(&profile(&url)),
            "",
        )
        .unwrap_err();
    assert_eq!(err, "`agent:coder (m)` did not finish in 1 steps");
}

#[test]
fn run_loop_uses_the_token_budget_of_the_params_or_the_profile() {
    // The action sets the budget.
    let (url, _requests) = serve(vec![(
        200,
        json!({
            "choices": [{"message": {"tool_calls": [{
                "id": "c1",
                "type": "function",
                "function": {"name": "unknown", "arguments": "{}"},
            }]}}],
            "usage": {"total_tokens": 300},
        })
        .to_string(),
    )]);
    let err = AgentToolLoop::new(AgentType::Coder)
        .run_loop(
            &params(&["model=m", "token_budget=200", "task=t"]),
            Some(&profile(&url)),
            "",
        )
        .unwrap_err();
    assert_eq!(
        err,
        "`agent:coder (m)` used 300 tokens in 1 steps, and its token budget is 200"
    );

    // The profile sets the budget, and the server sends no token count.
    let (url, _requests) = serve(vec![(200, text_response("done"))]);
    let budget_profile = ModelProfile {
        token_budget: Some(200),
        ..profile(&url)
    };
    let err = AgentToolLoop::new(AgentType::Coder)
        .run_loop(&params(&["model=m", "task=t"]), Some(&budget_profile), "")
        .unwrap_err();
    assert!(err.contains("has a token budget of 200"), "{err}");

    // The server sends the token count of a final reply.
    let (url, _requests) = serve(vec![(200, text_response_with_usage("done", 150))]);
    let output = AgentToolLoop::new(AgentType::Coder)
        .run_loop(
            &params(&["model=m", "token_budget=200", "task=t"]),
            Some(&profile(&url)),
            "",
        )
        .unwrap();
    assert_eq!(output, "done");
}

#[test]
fn label_names_the_profile_after_the_loop_reads_it() {
    let mut agent = AgentToolLoop::new(AgentType::Coder);
    assert_eq!(agent.label(), "agent:coder");

    let (url, _requests) = serve(vec![(200, text_response("done"))]);
    agent
        .run_loop_with_env(
            &params(&["model=m", "task=t"]),
            Some(&profile(&url)),
            "",
            &no_env,
        )
        .unwrap();
    assert_eq!(agent.label(), "agent:coder (m)");
}

#[test]
fn run_loop_sends_thinking_false_to_the_server() {
    let (url, requests) = serve(vec![(200, text_response("hi")), (200, text_response("hi"))]);
    let run = |words: &[&str]| {
        AgentToolLoop::new(AgentType::Coder)
            .run_loop_with_env(&params(words), Some(&profile(&url)), "", &no_env)
            .unwrap();
        request_json(&requests.recv().unwrap())
    };
    let request = run(&["model=m", "task=t", "thinking=false"]);
    assert_eq!(request["reasoning_effort"], "none");
    let request = run(&["model=m", "task=t"]);
    assert!(request.get("reasoning_effort").is_none(), "{request}");
}

#[test]
fn run_loop_fails_on_invalid_configuration() {
    let err = AgentToolLoop::new(AgentType::Coder)
        .run_loop(&params(&["model=m", "model=m"]), Some(&profile("x")), "")
        .unwrap_err();
    assert_eq!(err, "option `model` is set twice");
}

// Tools

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
fn bash_tool_writes_a_file_with_a_heredoc() {
    let path = format!("target/bash-tool-heredoc-{}.py", std::process::id());
    let command = format!("cat > {path} <<'EOF'\nprint('hi')\nEOF");

    assert_eq!(Bash.call(&json!({"command": command})), Ok(String::new()));
    let content = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(content, "print('hi')\n");
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
    let output = AgentToolLoop::new(AgentType::Coder)
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
            "=== agent:coder step 1: request (2 messages, 2 new, tools: lookup, failing) ===",
            "=== agent:coder step 1: response (2 tool calls) ===",
            "=== agent:coder step 1: tool call 1 of 2: lookup (id call-1) ===",
            "=== agent:coder step 1: tool call 2 of 2: failing (id call-2) ===",
            "=== agent:coder step 2: request (5 messages, 3 new, tools: lookup, failing) ===",
            "=== agent:coder step 2: response (0 tool calls) ===",
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
            &params(&["model=m", "max_steps=4", "task=t"]),
            Some(&profile(&url)),
            "",
        )
        .unwrap();
    let out = String::from_utf8(out).unwrap();

    let expected = format!(
        "=== agent:coder (m) configuration ===\nprofile: m\nmodel: model-1\nbase_url: {url}\n\
         max_steps: 4\napi key: not set"
    );
    assert!(out.starts_with(&expected), "{out}");
    // Without `thinking=false`, the request has no `reasoning_effort`.
    assert!(!out.contains("reasoning_effort"), "{out}");
}

#[test]
fn debug_writes_the_reasoning_effort_of_each_request() {
    let (url, _requests) = serve(vec![(200, text_response("done"))]);
    let mut out = Vec::new();
    AgentToolLoop::new(AgentType::Summarizer)
        .debug(&mut out)
        .run_loop_with_env(
            &params(&["model=m", "thinking=false"]),
            Some(&profile(&url)),
            "some input",
            &no_env,
        )
        .unwrap();
    let out = String::from_utf8(out).unwrap();
    assert!(
        out.contains(
            "=== agent:summarizer (m) step 1: request (2 messages, 2 new, tools: , \
             reasoning_effort: none) ==="
        ),
        "{out}"
    );
}

// Summarizer

#[test]
fn summarizer_sends_the_input_without_tools_and_returns_the_summary() {
    let (url, requests) = serve(vec![(200, text_response("short summary"))]);
    let output = AgentToolLoop::new(AgentType::Summarizer).run_loop_with_env(
        &params(&["model=m", "max_output=1 sentence"]),
        Some(&profile(&url)),
        "Rust 1.80 came out on 25 July 2024.",
        &no_env,
    );
    assert_eq!(output, Ok("short summary".to_string()));
    let request = request_json(&requests.recv().unwrap());
    assert_eq!(request["model"], "model-1");
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
        .run_loop_with_env(&params(&["model=m"]), Some(&profile(&url)), "", &no_env)
        .unwrap_err();
    assert_eq!(err, "`agent:summarizer` needs an input to summarize");
}

// Output schema

fn title_schema() -> OutputSchema {
    OutputSchema {
        name: "article".into(),
        schema: json!({
            "type": "object",
            "properties": {"title": {"type": "string"}},
            "required": ["title"],
        }),
    }
}

#[test]
fn output_schema_goes_into_the_system_prompt_and_checks_the_reply() {
    let mut client = ScriptedClient::new(vec![Ok(text("```json\n{\"title\": \"T\"}\n```"))]);
    let output = AgentToolLoop::new(AgentType::Summarizer)
        .output_schema(Some(title_schema()))
        .run_with(&mut client, "task", "input")
        .unwrap();
    assert_eq!(output, "{\"title\": \"T\"}");
    let Message::System(system) = &client.conversations[0][0] else {
        panic!("the first message is the system prompt");
    };
    assert!(
        system.ends_with(&format!("\n\n{}", title_schema().instruction())),
        "{system}"
    );
}

#[test]
fn a_reply_that_does_not_match_the_schema_goes_back_to_the_model() {
    let mut client = ScriptedClient::new(vec![
        Ok(text("The title is T.")),
        Ok(calls(&[("c1", "rec", "{}")])),
        Ok(text("{\"title\": \"T\"}")),
    ]);
    let (tool, _) = Recorder::boxed("rec", Ok("ok"));
    let mut out = Vec::new();
    let output = AgentToolLoop::new(AgentType::Coder)
        .with_tools(vec![tool])
        .output_schema(Some(title_schema()))
        .debug(&mut out)
        .run_with(&mut client, "task", "")
        .unwrap();
    assert_eq!(output, "{\"title\": \"T\"}");

    let second = &client.conversations[1];
    assert_eq!(second.len(), 4);
    assert_eq!(second[2], Message::Assistant(text("The title is T.")));
    let Message::User(feedback) = &second[3] else {
        panic!("the error goes to the model as a user message");
    };
    assert!(
        feedback.starts_with("Your reply is not valid: the reply is not valid JSON: "),
        "{feedback}"
    );
    assert!(
        feedback.ends_with("\nReply again with only the JSON value."),
        "{feedback}"
    );

    let out = String::from_utf8(out).unwrap();
    assert!(
        out.contains(
            "=== agent:coder step 1: reply does not match output schema `article` ===\n\
             the reply is not valid JSON: "
        ),
        "{out}"
    );
    assert!(
        out.contains("tools: rec, output_schema: article) ==="),
        "{out}"
    );
}

#[test]
fn fails_with_the_last_schema_error_after_max_steps() {
    let mut client = ScriptedClient::new(vec![Ok(text("{}")), Ok(text("{\"title\": 1}"))]);
    let err = AgentToolLoop::new(AgentType::Summarizer)
        .output_schema(Some(title_schema()))
        .max_steps(2)
        .run_with(&mut client, "task", "input")
        .unwrap_err();
    assert!(
        err.starts_with(
            "`agent:summarizer` did not reply with JSON that matches output schema `article` \
             in 2 steps, the last error: the reply does not match the schema:\nat `/title`: "
        ),
        "{err}"
    );
}

#[test]
fn request_body_has_the_response_format_of_the_schema() {
    let messages = [Message::User("task".into())];
    let body = request_body("m", &messages, &[], None, Some(&title_schema()));
    assert_eq!(body["response_format"], title_schema().response_format());
    let body = request_body("m", &messages, &[], None, None);
    assert!(body.get("response_format").is_none(), "{body}");
}

#[test]
fn run_loop_uses_the_schema_block_that_output_schema_names() {
    let (url, requests) = serve(vec![(200, text_response("{\"title\": \"T\"}"))]);
    let mut out = Vec::new();
    let output = AgentToolLoop::new(AgentType::Summarizer)
        .schema_block(Some(title_schema()))
        .debug(&mut out)
        .run_loop_with_env(
            &params(&["model=m", "output_schema=article"]),
            Some(&profile(&url)),
            "some input",
            &no_env,
        )
        .unwrap();
    assert_eq!(output, "{\"title\": \"T\"}");
    let request = request_json(&requests.recv().unwrap());
    assert_eq!(request["response_format"], title_schema().response_format());
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("\noutput_schema: article\n"), "{out}");
}

#[test]
fn run_loop_reads_a_schema_file() {
    let dir = std::env::temp_dir().join(format!("atama-loop-schema-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("story.json");
    std::fs::write(&path, title_schema().schema.to_string()).unwrap();
    let (url, requests) = serve(vec![(200, text_response("{\"title\": \"T\"}"))]);
    AgentToolLoop::new(AgentType::Coder)
        .run_loop_with_env(
            &params(&[
                "model=m",
                "task=t",
                &format!("output_schema={}", path.display()),
            ]),
            Some(&profile(&url)),
            "",
            &no_env,
        )
        .unwrap();
    let request = request_json(&requests.recv().unwrap());
    assert_eq!(request["response_format"]["json_schema"]["name"], "story");
    assert_eq!(
        request["response_format"]["json_schema"]["schema"],
        title_schema().schema
    );
}

#[test]
fn run_loop_fails_on_a_schema_that_it_cannot_load() {
    let run = |value: &str, block: Option<OutputSchema>| {
        AgentToolLoop::new(AgentType::Coder)
            .schema_block(block)
            .run_loop_with_env(
                &params(&["model=m", "task=t", &format!("output_schema={value}")]),
                Some(&profile(&closed_url())),
                "",
                &no_env,
            )
            .unwrap_err()
    };
    assert_eq!(
        run("article", None),
        "output schema `article` is not defined in the job"
    );
    assert_eq!(
        run("story", Some(title_schema())),
        "output schema `story` is not defined in the job"
    );
    assert!(
        run("./no/such.json", None).starts_with("cannot read output schema file"),
        "file"
    );
}
