//! Tests for `autonomy=` and `on_ask=` of the OpenAI compatible agent loop:
//! which tool calls wait for approval, and who answers them, in
//! `src/agent_loop_openai_compat.rs`.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use serde_json::{Value, json};

use atama::AgentToolLoop;
use atama::actor::AgentType;
use atama::agent_loop_openai_compat::agent_options::ToolGuard;
use atama::agent_loop_openai_compat::{ChatClient, Message, Reply, Tool, ToolCall};
use atama::profile::ModelProfile;
use atama::schema::OutputSchema;
use atama::tools::{Approver, Autonomy, OnAsk};

fn text(content: &str) -> Reply {
    Reply {
        content: Some(content.into()),
        ..Default::default()
    }
}

fn verdict(pass: bool, reason: &str) -> Reply {
    text(&json!({"pass": pass, "reason": reason}).to_string())
}

fn with_tokens(reply: Reply, tokens: u64) -> Reply {
    Reply {
        tokens: Some(tokens),
        ..reply
    }
}

/// A reply with one call of `name` with `arguments`.
fn call(id: &str, name: &str, arguments: &str) -> Reply {
    Reply {
        tool_calls: vec![ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: arguments.into(),
        }],
        ..Default::default()
    }
}

/// A client with one queue of scripted replies for the agent and the
/// judges. It records each conversation, and whether it was a judge
/// request.
#[derive(Default)]
struct ScriptedClient {
    replies: VecDeque<Reply>,
    requests: Vec<(Vec<Message>, bool)>,
}

impl ScriptedClient {
    fn new(replies: Vec<Reply>) -> Self {
        ScriptedClient {
            replies: replies.into(),
            ..Default::default()
        }
    }

    fn judge_requests(&self) -> usize {
        self.requests.iter().filter(|(_, judge)| *judge).count()
    }

    /// The text of the last message of the agent request `index`.
    fn last_message(&self, index: usize) -> String {
        let agent: Vec<_> = self.requests.iter().filter(|(_, judge)| !judge).collect();
        match agent[index].0.last().unwrap() {
            Message::Tool { content, .. } | Message::User(content) => content.clone(),
            other => panic!("unexpected message {other:?}"),
        }
    }
}

impl ChatClient for ScriptedClient {
    fn chat(&mut self, messages: &[Message], _: &[Box<dyn Tool>]) -> Result<Reply, String> {
        self.requests.push((messages.to_vec(), false));
        Ok(self.replies.pop_front().expect("no more replies"))
    }

    fn chat_without_tools(
        &mut self,
        messages: &[Message],
        _: Option<&OutputSchema>,
    ) -> Result<Reply, String> {
        self.requests.push((messages.to_vec(), true));
        Ok(self.replies.pop_front().expect("no more replies"))
    }
}

/// A tool that counts its calls. `read_only` says whether it only reads.
struct Counter {
    name: &'static str,
    read_only: bool,
    calls: Rc<Cell<u32>>,
}

impl Tool for Counter {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "counts"
    }

    fn parameters(&self) -> Value {
        json!({"type": "object"})
    }

    fn call(&mut self, _: &Value) -> Result<String, String> {
        self.calls.set(self.calls.get() + 1);
        Ok("ok".into())
    }

    fn read_only(&self) -> bool {
        self.read_only
    }
}

fn counter(name: &'static str, read_only: bool) -> (Box<dyn Tool>, Rc<Cell<u32>>) {
    let calls = Rc::new(Cell::new(0));
    (
        Box::new(Counter {
            name,
            read_only,
            calls: calls.clone(),
        }),
        calls,
    )
}

/// The tool name and the input of each question to an approver.
type Asked = Rc<RefCell<Vec<(String, Value)>>>;

/// An approver that records each question, and gives a scripted answer.
struct Recording {
    allow: bool,
    asked: Asked,
}

impl Approver for Recording {
    fn approve(&mut self, tool: &str, input: &Value) -> Result<(), String> {
        self.asked
            .borrow_mut()
            .push((tool.to_string(), input.clone()));
        if self.allow {
            Ok(())
        } else {
            Err(format!("the test denied `{tool}`"))
        }
    }
}

fn recording(allow: bool) -> (Recording, Asked) {
    let asked = Rc::new(RefCell::new(Vec::new()));
    (
        Recording {
            allow,
            asked: asked.clone(),
        },
        asked,
    )
}

fn coder() -> AgentToolLoop<'static> {
    AgentToolLoop::new(AgentType::Coder)
}

#[test]
fn no_ask_runs_each_call_without_the_approver() {
    let (tool, calls) = counter("bash", false);
    let (approver, asked) = recording(false);
    let mut client = ScriptedClient::new(vec![call("c1", "bash", "{}"), text("done")]);
    let output = coder()
        .with_tools(vec![tool])
        .approver(approver)
        .run_with(&mut client, "Fix it.", "")
        .unwrap();
    assert_eq!(output, "done");
    assert_eq!(calls.get(), 1);
    assert!(asked.borrow().is_empty());
}

#[test]
fn ask_if_risky_asks_only_for_a_tool_that_can_change_something() {
    let (bash, bash_calls) = counter("bash", false);
    let (read, read_calls) = counter("read_file", true);
    let (approver, asked) = recording(false);
    let mut client = ScriptedClient::new(vec![
        call("c1", "read_file", r#"{"path": "a"}"#),
        call("c2", "bash", r#"{"command": "rm -rf x"}"#),
        text("done"),
    ]);
    let mut out = Vec::new();
    let output = coder()
        .with_tools(vec![bash, read])
        .autonomy(Autonomy::AskIfRisky)
        .approver(approver)
        .debug(&mut out)
        .run_with(&mut client, "Fix it.", "")
        .unwrap();
    assert_eq!(output, "done");
    assert_eq!(read_calls.get(), 1);
    assert_eq!(bash_calls.get(), 0);
    assert_eq!(
        *asked.borrow(),
        [("bash".to_string(), json!({"command": "rm -rf x"}))]
    );
    // The model gets the reason, and the run goes on.
    assert_eq!(client.last_message(2), "error: the test denied `bash`");
    let out = String::from_utf8(out).unwrap();
    assert!(
        out.contains("tool call 1 of 1: bash (id c2) denied ===\narguments:\n"),
        "{out}"
    );
    assert!(out.contains("reason: the test denied `bash`"), "{out}");
}

#[test]
fn always_ask_asks_for_each_call_and_runs_an_allowed_call() {
    let (read, read_calls) = counter("read_file", true);
    let (approver, asked) = recording(true);
    let mut client = ScriptedClient::new(vec![call("c1", "read_file", ""), text("done")]);
    coder()
        .with_tools(vec![read])
        .autonomy(Autonomy::AlwaysAsk)
        .approver(approver)
        .run_with(&mut client, "Read it.", "")
        .unwrap();
    assert_eq!(read_calls.get(), 1);
    // An empty argument string is an empty object.
    assert_eq!(*asked.borrow(), [("read_file".to_string(), json!({}))]);
}

#[test]
fn a_call_that_fails_without_running_asks_nobody() {
    let (bash, bash_calls) = counter("bash", false);
    let (approver, asked) = recording(true);
    let mut client = ScriptedClient::new(vec![
        call("c1", "unknown", "{}"),
        call("c2", "bash", "not json"),
        text("done"),
    ]);
    coder()
        .with_tools(vec![bash])
        .autonomy(Autonomy::AlwaysAsk)
        .approver(approver)
        .run_with(&mut client, "Fix it.", "")
        .unwrap();
    assert!(asked.borrow().is_empty());
    assert_eq!(bash_calls.get(), 0);
    assert_eq!(client.last_message(1), "error: unknown tool `unknown`");
    assert!(
        client
            .last_message(2)
            .starts_with("error: arguments of `bash` are not JSON"),
        "{}",
        client.last_message(2)
    );
}

#[test]
fn on_ask_deny_denies_without_an_approver() {
    let (bash, bash_calls) = counter("bash", false);
    let mut client = ScriptedClient::new(vec![call("c1", "bash", "{}"), text("done")]);
    coder()
        .with_tools(vec![bash])
        .autonomy(Autonomy::AskIfRisky)
        .on_ask(OnAsk::Deny)
        .run_with(&mut client, "Fix it.", "")
        .unwrap();
    assert_eq!(bash_calls.get(), 0);
    assert_eq!(
        client.last_message(1),
        "error: the call of `bash` was denied, because `on_ask=deny` denies each call that \
         waits for approval"
    );
}

#[test]
fn on_ask_judge_asks_the_guard_and_counts_its_tokens() {
    let (bash, bash_calls) = counter("bash", false);
    let mut client = ScriptedClient::new(vec![
        with_tokens(call("c1", "bash", r#"{"command": "git push"}"#), 10),
        with_tokens(verdict(false, "it pushes"), 3),
        with_tokens(call("c2", "bash", r#"{"command": "ls"}"#), 10),
        with_tokens(verdict(true, "fine"), 3),
        with_tokens(text("done"), 10),
    ]);
    let mut out = Vec::new();
    let output = coder()
        .with_tools(vec![bash])
        .autonomy(Autonomy::AskIfRisky)
        .on_ask(OnAsk::Judge)
        .guardrails(Some("Do not push.".into()))
        .token_budget(Some(1000))
        .debug(&mut out)
        .run_with(&mut client, "Fix it.", "")
        .unwrap();
    assert_eq!(output, "done");
    assert_eq!(bash_calls.get(), 1);
    assert_eq!(client.judge_requests(), 2);
    assert_eq!(
        client.last_message(1),
        "error: blocked by guardrail: it pushes"
    );
    let out = String::from_utf8(out).unwrap();
    // 3 agent replies and 2 verdicts.
    assert!(out.contains("36 of 1000 tokens used"), "{out}");
}

#[test]
fn a_judge_reply_without_tokens_fails_a_run_with_a_budget() {
    let (bash, bash_calls) = counter("bash", false);
    let mut client = ScriptedClient::new(vec![
        with_tokens(call("c1", "bash", "{}"), 10),
        verdict(true, "fine"),
    ]);
    let err = coder()
        .with_tools(vec![bash])
        .autonomy(Autonomy::AlwaysAsk)
        .on_ask(OnAsk::Judge)
        .guardrails(Some("Do not push.".into()))
        .token_budget(Some(1000))
        .run_with(&mut client, "Fix it.", "")
        .unwrap_err();
    assert!(err.contains("did not send `usage.total_tokens`"), "{err}");
    assert_eq!(bash_calls.get(), 0);
}

#[test]
fn a_denied_call_is_not_a_violation() {
    let (bash, _) = counter("bash", false);
    let mut client = ScriptedClient::new(vec![
        call("c1", "bash", "{}"),
        verdict(false, "first"),
        call("c2", "bash", "{}"),
        verdict(false, "second"),
        text("done"),
    ]);
    let output = AgentToolLoop::new(AgentType::Autonomous)
        .max_steps(10)
        .with_tools(vec![bash])
        .autonomy(Autonomy::AlwaysAsk)
        .on_ask(OnAsk::Judge)
        .guardrails(Some("Do not push.".into()))
        .guard(ToolGuard::Prompt)
        .max_violations(1)
        .run_with(&mut client, "Fix it.", "")
        .unwrap();
    assert_eq!(output, "done");
}

#[test]
fn on_ask_judge_uses_the_judge_client_if_it_is_set() {
    let (bash, bash_calls) = counter("bash", false);
    let mut agent = ScriptedClient::new(vec![call("c1", "bash", "{}"), text("done")]);
    let judge = ScriptedClient::new(vec![verdict(true, "fine")]);
    coder()
        .with_tools(vec![bash])
        .autonomy(Autonomy::AlwaysAsk)
        .on_ask(OnAsk::Judge)
        .guardrails(Some("Do not push.".into()))
        .judge_client(judge)
        .run_with(&mut agent, "Fix it.", "")
        .unwrap();
    assert_eq!(bash_calls.get(), 1);
    assert_eq!(agent.judge_requests(), 0);
}

// The options of the action, through `run_loop_with_env` and a mock server.

/// Starts a local HTTP server that answers one request per body, in
/// order. Returns its base URL and a receiver that gets each request body.
fn serve(bodies: Vec<String>) -> (String, Receiver<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("cannot bind test server");
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (requests, received) = mpsc::channel();
    thread::spawn(move || {
        for body in bodies {
            let (mut stream, _) = listener.accept().expect("cannot accept connection");
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut content_length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                    content_length = value.trim().parse().unwrap();
                }
                if line == "\r\n" {
                    break;
                }
            }
            let mut request = vec![0; content_length];
            reader.read_exact(&mut request).unwrap();
            requests
                .send(serde_json::from_slice(&request).unwrap())
                .unwrap();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body.as_bytes()).unwrap();
        }
    });
    (url, received)
}

fn text_response(content: &str) -> String {
    json!({"choices": [{"message": {"role": "assistant", "content": content}}]}).to_string()
}

fn tool_call_response(name: &str, arguments: Value) -> String {
    json!({"choices": [{"message": {
        "role": "assistant",
        "content": null,
        "tool_calls": [{
            "id": "c1",
            "type": "function",
            "function": {"name": name, "arguments": arguments.to_string()},
        }],
    }}]})
    .to_string()
}

fn profile(name: &str, base_url: &str) -> ModelProfile {
    ModelProfile {
        name: name.into(),
        model: "model-1".into(),
        base_url: base_url.into(),
        api_key_env: None,
        token_budget: None,
        thinking: None,
        effort: None,
    }
}

fn params(params: &[&str]) -> Vec<String> {
    params.iter().map(|param| param.to_string()).collect()
}

fn no_env(_: &str) -> Option<String> {
    None
}

/// The content of the last message of a request body.
fn last_content(body: &Value) -> String {
    body["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn the_options_of_the_action_deny_a_bash_call_of_a_coder() {
    let (url, requests) = serve(vec![
        tool_call_response("bash", json!({"command": "touch should-not-exist"})),
        text_response("done"),
    ]);
    let mut out = Vec::new();
    let output = coder()
        .debug(&mut out)
        .run_loop_with_env(
            &params(&[
                "model=m",
                "autonomy=ask_if_risky",
                "on_ask=deny",
                "task=Fix it.",
            ]),
            Some(&profile("m", &url)),
            "",
            &no_env,
        )
        .unwrap();
    assert_eq!(output, "done");
    requests.recv().unwrap();
    let second = requests.recv().unwrap();
    assert!(
        last_content(&second).contains("`on_ask=deny` denies each call"),
        "{second}"
    );
    assert!(!std::path::Path::new("should-not-exist").exists());
    let out = String::from_utf8(out).unwrap();
    assert!(
        out.contains("autonomy: ask_if_risky\non_ask: deny"),
        "{out}"
    );
}

#[test]
fn the_judge_of_a_coder_uses_the_profile_of_judge_model() {
    let (url, requests) = serve(vec![
        tool_call_response("bash", json!({"command": "git push"})),
        text_response("done"),
    ]);
    let (judge_url, judge_requests) = serve(vec![text_response(
        &json!({"pass": false, "reason": "it pushes"}).to_string(),
    )]);
    let mut out = Vec::new();
    let output = coder()
        .judge_profile(Some(profile("judge", &judge_url)))
        .debug(&mut out)
        .run_loop_with_env(
            &params(&[
                "model=m",
                "autonomy=always_ask",
                "on_ask=judge",
                "guardrails=Do not push.",
                "judge_model=judge",
                "task=Fix it.",
            ]),
            Some(&profile("m", &url)),
            "",
            &no_env,
        )
        .unwrap();
    assert_eq!(output, "done");
    let first = requests.recv().unwrap();
    // The guardrails are also in the system prompt of the agent.
    assert!(
        first["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("Do not push."),
        "{first}"
    );
    let judge = judge_requests.recv().unwrap();
    assert!(
        judge["messages"][1]["content"]
            .as_str()
            .unwrap()
            .contains("git push")
    );
    let second = requests.recv().unwrap();
    assert_eq!(
        last_content(&second),
        "error: blocked by guardrail: it pushes"
    );
    let out = String::from_utf8(out).unwrap();
    assert!(
        out.contains("guardrails: Do not push.\njudge_model: judge"),
        "{out}"
    );
}
