//! Tests of a run of a `claude-agent` action (`ClaudeAgentRun` in
//! `src/agent_loop_claude/run.rs`), with a mock API and scripted event
//! streams. They do not call the real API.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use atama::agent_loop_claude::ClaudeAgentRun;
use atama::agent_loop_claude::api::{AgentRef, AgentsApi, ApiError, EventStream};
use atama::agent_loop_claude::custom_tools::LocalTools;
use atama::agent_loop_claude::events::{SessionEvent, parse_event};
use atama::agent_loop_claude::run::MAX_RECONNECTS;
use atama::profile::ModelProfile;
use atama::schema::OutputSchema;
use atama::tools::{Approver, Tool};

/// One item of a scripted stream.
#[derive(Clone)]
enum Item {
    Event(SessionEvent),
    /// The stream fails with this error.
    Fail(String),
    /// The stream waits, and then ends.
    Wait(Duration),
    /// The stream waits, and then goes on with the next item.
    Pause(Duration),
}

fn event(json: Value) -> Item {
    Item::Event(parse_event(&json).unwrap())
}

fn message(id: &str, text: &str) -> Item {
    event(json!({"type": "agent.message", "id": id, "content": [{"type": "text", "text": text}]}))
}

fn idle(id: &str, stop: &str) -> Item {
    event(json!({"type": "session.status_idle", "id": id, "stop_reason": {"type": stop}}))
}

/// The state of the mock API, shared with the test.
#[derive(Default)]
struct State {
    /// Each request other than `send_events`, with its body.
    requests: Vec<(String, Value)>,
    /// Each event that the run sent.
    sent: Vec<Value>,
    /// The scripts of the streams, one for each `open_stream`.
    streams: VecDeque<Vec<Item>>,
    /// The answer of `list_events`.
    history: Vec<SessionEvent>,
    /// The answer of `get_agent`.
    agent: Value,
    /// Makes `send_events` fail when the run sent this number of events.
    send_error: Option<(usize, String)>,
}

#[derive(Clone, Default)]
struct MockApi(Rc<RefCell<State>>);

impl MockApi {
    fn with_streams(streams: Vec<Vec<Item>>) -> Self {
        let api = MockApi::default();
        api.0.borrow_mut().streams = streams.into();
        api
    }

    fn sent(&self) -> Vec<Value> {
        self.0.borrow().sent.clone()
    }

    fn requests(&self) -> Vec<(String, Value)> {
        self.0.borrow().requests.clone()
    }

    fn request(&self, name: &str) -> Value {
        self.requests()
            .into_iter()
            .find(|(request, _)| request == name)
            .map(|(_, body)| body)
            .unwrap_or_else(|| panic!("no request `{name}`"))
    }

    fn record(&self, name: &str, body: Value) {
        self.0.borrow_mut().requests.push((name.to_string(), body));
    }
}

impl AgentsApi for MockApi {
    fn create_agent(&mut self, body: &Value) -> Result<AgentRef, ApiError> {
        self.record("create_agent", body.clone());
        Ok(AgentRef {
            id: "agent_1".into(),
            version: 1,
        })
    }

    fn update_agent(&mut self, id: &str, body: &Value) -> Result<AgentRef, ApiError> {
        self.record("update_agent", body.clone());
        Ok(AgentRef {
            id: id.into(),
            version: 2,
        })
    }

    fn get_agent(&mut self, id: &str) -> Result<Value, ApiError> {
        self.record("get_agent", json!(id));
        Ok(self.0.borrow().agent.clone())
    }

    fn create_environment(&mut self, body: &Value) -> Result<String, ApiError> {
        self.record("create_environment", body.clone());
        Ok("env_1".into())
    }

    fn create_session(&mut self, body: &Value) -> Result<String, ApiError> {
        self.record("create_session", body.clone());
        Ok("sesn_1".into())
    }

    fn get_session(&mut self, _id: &str) -> Result<Value, ApiError> {
        unimplemented!()
    }

    fn send_events(&mut self, session_id: &str, events: &[Value]) -> Result<(), ApiError> {
        assert_eq!(session_id, "sesn_1");
        if let Some((after, error)) = self.0.borrow().send_error.clone()
            && self.0.borrow().sent.len() >= after
        {
            return Err(ApiError {
                status: Some(400),
                message: error,
            });
        }
        self.0.borrow_mut().sent.extend(events.iter().cloned());
        Ok(())
    }

    fn list_events(&mut self, session_id: &str) -> Result<Vec<SessionEvent>, ApiError> {
        self.record("list_events", json!(session_id));
        Ok(self.0.borrow().history.clone())
    }

    fn open_stream(&mut self, session_id: &str) -> Result<Box<dyn EventStream>, ApiError> {
        self.record("open_stream", json!(session_id));
        let items = self.0.borrow_mut().streams.pop_front().unwrap_or_default();
        Ok(Box::new(MockStream {
            items: items.into(),
        }))
    }
}

struct MockStream {
    items: VecDeque<Item>,
}

impl EventStream for MockStream {
    fn next_event(&mut self) -> Result<Option<SessionEvent>, String> {
        match self.items.pop_front() {
            None => Ok(None),
            Some(Item::Event(event)) => Ok(Some(event)),
            Some(Item::Fail(error)) => Err(error),
            Some(Item::Wait(duration)) => {
                thread::sleep(duration);
                Ok(None)
            }
            Some(Item::Pause(duration)) => {
                thread::sleep(duration);
                self.next_event()
            }
        }
    }
}

/// A local tool that returns the `text` of its input, or fails if the input
/// has `fail`.
struct Echo {
    calls: Rc<Cell<u32>>,
}

impl Tool for Echo {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "Returns its text."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object"})
    }

    fn call(&mut self, arguments: &Value) -> Result<String, String> {
        self.calls.set(self.calls.get() + 1);
        if arguments.get("fail").is_some() {
            return Err("echo failed".into());
        }
        Ok(arguments["text"].as_str().unwrap_or_default().to_string())
    }
}

fn echo() -> (LocalTools, Rc<Cell<u32>>) {
    let calls = Rc::new(Cell::new(0));
    let tools = LocalTools::with_tools(vec![Box::new(Echo {
        calls: calls.clone(),
    })]);
    (tools, calls)
}

/// An approver that allows or denies each call, and keeps the names.
struct Fixed {
    allow: bool,
    asked: Rc<RefCell<Vec<String>>>,
}

impl Approver for Fixed {
    fn approve(&mut self, tool: &str, _input: &Value) -> Result<(), String> {
        self.asked.borrow_mut().push(tool.to_string());
        if self.allow {
            Ok(())
        } else {
            Err(format!("no to `{tool}`"))
        }
    }
}

/// A writer that keeps what it gets, for the debug output.
#[derive(Clone, Default)]
struct Output(Arc<Mutex<Vec<u8>>>);

impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn profile() -> ModelProfile {
    ModelProfile {
        name: "claude".into(),
        model: "claude-opus-5-5".into(),
        base_url: "http://localhost:1/v1".into(),
        api_key_env: Some("CLAUDE_KEY".into()),
        token_budget: None,
        thinking: None,
        effort: Some("high".into()),
    }
}

fn env(name: &str) -> Option<String> {
    (name == "CLAUDE_KEY").then(|| "sk-test".to_string())
}

/// A lock path in a new, empty directory for one test.
fn lock_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("atama-claude-run-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir.join("claude-lock.json")
}

fn params(options: &[&str]) -> Vec<String> {
    let mut params = vec!["model=claude".to_string()];
    params.extend(options.iter().map(|option| option.to_string()));
    params
}

/// Runs an action with the mock API, the local tool `echo` and a fast poll.
fn run_with(
    name: &str,
    api: &MockApi,
    options: &[&str],
    input: &str,
    setup: impl FnOnce(ClaudeAgentRun<'static>) -> ClaudeAgentRun<'static>,
) -> Result<String, String> {
    let run = ClaudeAgentRun::new("atama-job-task-1")
        .api(api.clone())
        .lock_path(lock_path(name))
        .poll_interval(Duration::from_millis(5));
    setup(run).run_with_env(&params(options), Some(&profile()), input, &env)
}

fn run(name: &str, api: &MockApi, options: &[&str]) -> Result<String, String> {
    let (tools, _) = echo();
    run_with(name, api, options, "", |run| run.with_tools(tools))
}

fn user_message(text: &str) -> Value {
    json!({"type": "user.message", "content": [{"type": "text", "text": text}]})
}

#[test]
fn returns_the_last_message_of_the_turn() {
    let api = MockApi::with_streams(vec![vec![
        event(json!({"type": "session.status_running", "id": "e1"})),
        message("e2", "First."),
        event(json!({"type": "agent.tool_use", "id": "e3", "name": "web_search", "input": {}})),
        message("e4", "Done."),
        event(json!({"type": "session.usage", "id": "e5", "usage": {"input_tokens": 10}})),
        idle("e6", "end_turn"),
    ]]);
    let (tools, _) = echo();
    let path = lock_path("reply");
    let output = ClaudeAgentRun::new("atama-job-task-1")
        .api(api.clone())
        .lock_path(&path)
        .with_tools(tools)
        .run_with_env(
            &params(&["task=Say done."]),
            Some(&profile()),
            "the input",
            &env,
        )
        .unwrap();
    assert_eq!(output, "Done.");

    assert_eq!(
        api.request("create_agent"),
        json!({
            "name": "atama-job-task-1",
            "model": {"id": "claude-opus-5-5", "effort": "high"},
            "tools": [],
        })
    );
    assert_eq!(api.request("create_environment")["name"], "atama-default");
    assert_eq!(
        api.request("create_session"),
        json!({
            "agent": {"type": "agent", "id": "agent_1", "version": 1},
            "environment_id": "env_1",
            "title": "atama: atama-job-task-1",
        })
    );
    assert_eq!(api.sent(), [user_message("Say done.\n\nInput:\nthe input")]);
    // The stream opens before the first message.
    let names: Vec<String> = api.requests().into_iter().map(|(name, _)| name).collect();
    assert_eq!(
        names,
        [
            "create_agent",
            "create_environment",
            "create_session",
            "open_stream"
        ]
    );
    // The lock file keeps the agent, so the next run uses it.
    let lock: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(lock["agents"]["atama-job-task-1"]["id"], "agent_1");
}

#[test]
fn runs_the_local_tools_and_sends_their_results() {
    let api = MockApi::with_streams(vec![vec![
        event(
            json!({"type": "agent.custom_tool_use", "id": "t1", "name": "local_echo", "input": {"text": "hi"}}),
        ),
        event(
            json!({"type": "agent.custom_tool_use", "id": "t2", "name": "local_echo", "input": {"fail": true}}),
        ),
        event(
            json!({"type": "agent.custom_tool_use", "id": "t3", "name": "local_other", "input": {}}),
        ),
        idle("i1", "requires_action"),
        message("m1", "ok"),
        idle("i2", "end_turn"),
    ]]);
    let (tools, calls) = echo();
    let output = run_with("tools", &api, &["task=Use echo."], "", |run| {
        run.with_tools(tools)
    })
    .unwrap();
    assert_eq!(output, "ok");
    assert_eq!(calls.get(), 2);
    let result = |id: &str, text: &str, is_error: bool| {
        json!({
            "type": "user.custom_tool_result",
            "custom_tool_use_id": id,
            "content": [{"type": "text", "text": text}],
            "is_error": is_error,
        })
    };
    assert_eq!(
        api.sent()[1..],
        [
            result("t1", "hi", false),
            result("t2", "echo failed", true),
            result("t3", "unknown tool `local_other`", true),
        ]
    );
}

#[test]
fn asks_the_approver_for_a_native_call_that_waits() {
    for allow in [true, false] {
        let api = MockApi::with_streams(vec![vec![
            event(json!({
                "type": "agent.tool_use",
                "id": "u1",
                "name": "bash",
                "input": {"command": "ls"},
                "evaluated_permission": "ask",
            })),
            message("m1", "done"),
            idle("i1", "end_turn"),
        ]]);
        let asked = Rc::new(RefCell::new(Vec::new()));
        let approver: Rc<RefCell<dyn Approver>> = Rc::new(RefCell::new(Fixed {
            allow,
            asked: asked.clone(),
        }));
        let (tools, _) = echo();
        run_with("ask", &api, &["task=x"], "", |run| {
            run.with_tools(tools).approver(approver)
        })
        .unwrap();
        assert_eq!(*asked.borrow(), ["bash"]);
        let expected = if allow {
            json!({"type": "user.tool_confirmation", "tool_use_id": "u1", "result": "allow"})
        } else {
            json!({
                "type": "user.tool_confirmation",
                "tool_use_id": "u1",
                "result": "deny",
                "deny_message": "no to `bash`",
            })
        };
        assert_eq!(api.sent()[1], expected);
    }
}

#[test]
fn asks_the_schema_again_until_the_reply_matches() {
    let schema = OutputSchema {
        name: "answer".into(),
        schema: json!({
            "type": "object",
            "properties": {"n": {"type": "integer"}},
            "required": ["n"],
        }),
    };
    let api = MockApi::with_streams(vec![vec![
        message("m1", "Not JSON."),
        idle("i1", "end_turn"),
        message("m2", "{\"n\": 3}"),
        idle("i2", "end_turn"),
    ]]);
    let (tools, _) = echo();
    let output = run_with(
        "schema",
        &api,
        &["task=Count.", "output_schema=answer"],
        "",
        |run| run.with_tools(tools).schema_block(Some(schema.clone())),
    )
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&output).unwrap(),
        json!({"n": 3})
    );
    let sent = api.sent();
    assert_eq!(sent.len(), 2);
    let first = sent[0]["content"][0]["text"].as_str().unwrap();
    assert!(first.starts_with("Count.\n\n"), "{first}");
    assert!(first.contains(&schema.instruction()), "{first}");
    let retry = sent[1]["content"][0]["text"].as_str().unwrap();
    assert!(retry.starts_with("Your reply is not valid: "), "{retry}");

    // With one turn, the first invalid reply fails the run.
    let api = MockApi::with_streams(vec![vec![message("m1", "No."), idle("i1", "end_turn")]]);
    let (tools, _) = echo();
    let error = run_with(
        "schema-once",
        &api,
        &["task=Count.", "output_schema=answer", "max_turns=1"],
        "",
        |run| run.with_tools(tools).schema_block(Some(schema)),
    )
    .unwrap_err();
    assert!(
        error.starts_with(
            "agent:claude-agent (claude): the reply does not match the output schema after 1 \
             turns: "
        ),
        "{error}"
    );
}

/// A success check that fails until it ran `passes_at` times. It counts
/// its runs in a file in a new directory.
fn counting_check(name: &str, passes_at: u32) -> (String, PathBuf) {
    let dir = lock_path(name).parent().unwrap().to_path_buf();
    let count = dir.join("count");
    let command = format!(
        "n=$(( $(cat {0} 2>/dev/null || echo 0) + 1 )); echo $n > {0}; \
         echo \"run $n\"; [ $n -ge {passes_at} ]",
        count.display()
    );
    (command, count)
}

#[test]
fn sends_a_failed_success_check_until_it_passes() {
    let (command, count) = counting_check("success", 2);
    let api = MockApi::with_streams(vec![vec![
        message("m1", "First try."),
        idle("i1", "end_turn"),
        message("m2", "Second try."),
        idle("i2", "end_turn"),
    ]]);
    let option = format!("success_check={command}");
    let debug = Output::default();
    let (tools, _) = echo();
    let output = run_with("success", &api, &["task=Work.", &option], "", |run| {
        run.with_tools(tools).debug(debug.clone())
    })
    .unwrap();
    assert_eq!(output, "Second try.");
    assert_eq!(fs::read_to_string(&count).unwrap().trim(), "2");

    let sent = api.sent();
    assert_eq!(sent.len(), 2);
    let retry = sent[1]["content"][0]["text"].as_str().unwrap();
    assert!(
        retry.starts_with(&format!(
            "The task is not done yet, because the success check `{command}` failed:\n\
             exit status: 1\nrun 1\n"
        )),
        "{retry}"
    );
    let debug = String::from_utf8(debug.0.lock().unwrap().clone()).unwrap();
    assert!(debug.contains("success check `"), "{debug}");
    assert!(debug.contains("` failed: exit status: 1"), "{debug}");
    assert!(debug.contains("` passed"), "{debug}");
}

#[test]
fn fails_when_the_success_check_fails_at_the_last_turn() {
    let (command, _) = counting_check("success-fail", 10);
    let api = MockApi::with_streams(vec![vec![
        message("m1", "One."),
        idle("i1", "end_turn"),
        message("m2", "Two."),
        idle("i2", "end_turn"),
    ]]);
    let option = format!("success_check={command}");
    let error = run("success-fail", &api, &["task=Work.", &option, "max_turns=2"]).unwrap_err();
    assert_eq!(
        error,
        format!(
            "agent:claude-agent (claude): the success check `{command}` failed after 2 turns: \
             exit status: 1\nrun 2"
        )
    );
    assert_eq!(api.sent().len(), 2);
}

#[test]
fn checks_the_schema_before_the_success_check() {
    let schema = OutputSchema {
        name: "answer".into(),
        schema: json!({"type": "object", "required": ["n"]}),
    };
    let (command, count) = counting_check("schema-success", 1);
    let api = MockApi::with_streams(vec![vec![
        message("m1", "Not JSON."),
        idle("i1", "end_turn"),
        message("m2", "{\"n\": 1}"),
        idle("i2", "end_turn"),
    ]]);
    let option = format!("success_check={command}");
    let (tools, _) = echo();
    let output = run_with(
        "schema-success",
        &api,
        &["task=Count.", "output_schema=answer", &option],
        "",
        |run| run.with_tools(tools).schema_block(Some(schema)),
    )
    .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&output).unwrap(), json!({"n": 1}));
    // The check ran once, after the reply matched the schema.
    assert_eq!(fs::read_to_string(&count).unwrap().trim(), "1");
}

#[test]
fn fails_when_the_session_stops() {
    let cases = [
        (
            vec![
                event(
                    json!({"type": "session.error", "id": "x1", "error": {"message": "model down"}}),
                ),
                idle("x2", "retries_exhausted"),
            ],
            "the session `sesn_1` failed after its retries: model down",
        ),
        (
            vec![event(
                json!({"type": "session.status_terminated", "id": "x1"}),
            )],
            "the session `sesn_1` ended before the agent finished its turn",
        ),
        (
            vec![idle("x1", "paused")],
            "the session `sesn_1` stopped with `paused`",
        ),
        (
            vec![idle("x1", "end_turn")],
            "the session `sesn_1` ended its turn without a message",
        ),
    ];
    for (index, (events, expected)) in cases.into_iter().enumerate() {
        let api = MockApi::with_streams(vec![events]);
        assert_eq!(
            run(&format!("stop-{index}"), &api, &["task=x"]).unwrap_err(),
            format!("agent:claude-agent (claude): {expected}")
        );
    }
}

#[test]
fn a_session_with_a_budget_fails_at_the_budget() {
    let api = MockApi::with_streams(vec![vec![
        event(
            json!({"type": "session.usage", "id": "u1", "usage": {"list_cost": {"amount": "151"}}}),
        ),
        idle("i1", "budget_reached"),
    ]]);
    assert_eq!(
        run("budget", &api, &["task=x", "max_cost_usd=1.5"]).unwrap_err(),
        "agent:claude-agent (claude): the session `sesn_1` used its budget of $1.50"
    );
    assert_eq!(
        api.request("create_session")["budget"],
        json!({"type": "limit", "max_list_cost": {"amount": "150", "currency": "USD"}})
    );
}

#[test]
fn stops_at_the_token_budget() {
    let api = MockApi::with_streams(vec![vec![
        event(
            json!({"type": "session.usage", "id": "u1", "usage": {"input_tokens": 60, "output_tokens": 50}}),
        ),
        idle("i1", "end_turn"),
    ]]);
    assert_eq!(
        run("tokens", &api, &["task=x", "token_budget=100"]).unwrap_err(),
        "agent:claude-agent (claude): the session `sesn_1` used 110 tokens, more than its \
         budget of 100"
    );
    assert_eq!(
        api.sent().last().unwrap(),
        &json!({"type": "user.interrupt"})
    );
}

#[test]
fn stops_at_the_time_limit() {
    let api = MockApi::with_streams(vec![vec![Item::Wait(Duration::from_secs(2))]]);
    let start = Instant::now();
    let calls = Rc::new(Cell::new(0u32));
    let (tools, _) = echo();
    let counter = calls.clone();
    let error = run_with("time", &api, &["task=x", "max_time=1m"], "", move |run| {
        run.with_tools(tools).clock(move || {
            counter.set(counter.get() + 1);
            // The first call is the start. Each later call is 2 minutes on.
            start + Duration::from_secs(120 * u64::from(counter.get() - 1))
        })
    })
    .unwrap_err();
    assert_eq!(
        error,
        "agent:claude-agent (claude): the session `sesn_1` stopped after 1m, because the time \
         limit of the run is used"
    );
    assert_eq!(
        api.sent().last().unwrap(),
        &json!({"type": "user.interrupt"})
    );
    assert!(calls.get() >= 2);
}

#[test]
fn opens_the_stream_again_and_skips_the_events_it_handled() {
    let tool_use = event(
        json!({"type": "agent.custom_tool_use", "id": "t1", "name": "local_echo", "input": {"text": "a"}}),
    );
    let Item::Event(tool_event) = tool_use.clone() else {
        unreachable!()
    };
    let api = MockApi::with_streams(vec![
        vec![tool_use, Item::Fail("connection reset".into())],
        vec![message("m1", "late"), idle("i1", "end_turn")],
    ]);
    api.0.borrow_mut().history = vec![tool_event, parse_event(&json!({"type": "agent.message", "id": "m1", "content": [{"type": "text", "text": "late"}]})).unwrap()];
    let (tools, calls) = echo();
    let output = run_with("reconnect", &api, &["task=x"], "", |run| {
        run.with_tools(tools)
    })
    .unwrap();
    assert_eq!(output, "late");
    // The tool ran once, although the history has its call again.
    assert_eq!(calls.get(), 1);
    let names: Vec<String> = api.requests().into_iter().map(|(name, _)| name).collect();
    assert_eq!(names[3..], ["open_stream", "open_stream", "list_events"]);
}

#[test]
fn fails_after_the_stream_ends_too_often() {
    let api = MockApi::with_streams(vec![
        vec![],
        vec![],
        vec![],
        vec![Item::Fail("boom".into())],
    ]);
    assert_eq!(
        run("ends", &api, &["task=x"]).unwrap_err(),
        format!(
            "agent:claude-agent (claude): the stream of the session `sesn_1` ended \
             {MAX_RECONNECTS} times: boom"
        )
    );
}

#[test]
fn uses_an_agent_and_an_environment_that_exist() {
    let api = MockApi::with_streams(vec![vec![message("m1", "ok"), idle("i1", "end_turn")]]);
    api.0.borrow_mut().agent = json!({
        "id": "agent_9",
        "version": 4,
        "tools": [{"type": "custom", "name": "local_echo"}],
    });
    let path = lock_path("existing");
    let (tools, _) = echo();
    let output = ClaudeAgentRun::new("n")
        .api(api.clone())
        .lock_path(&path)
        .with_tools(tools)
        .run_with_env(
            &params(&[
                "task=x",
                "agent_id=agent_9",
                "environment_id=env_7",
                "local_tools=read_file",
            ]),
            Some(&profile()),
            "",
            &env,
        );
    // `local_echo` is not a tool of `local_tools=read_file`.
    assert_eq!(
        output.unwrap_err(),
        "agent:claude-agent (claude): agent `agent_9` has the custom tool `local_echo`, but no \
         tool of `local_tools=` answers it"
    );

    api.0.borrow_mut().agent = json!({"id": "agent_9", "tools": []});
    let (tools, _) = echo();
    let output = ClaudeAgentRun::new("n")
        .api(api.clone())
        .lock_path(&path)
        .with_tools(tools)
        .poll_interval(Duration::from_millis(5))
        .run_with_env(
            &params(&["task=x", "agent_id=agent_9", "environment_id=env_7"]),
            Some(&profile()),
            "",
            &env,
        )
        .unwrap();
    assert_eq!(output, "ok");
    assert_eq!(
        api.request("create_session"),
        json!({"agent": "agent_9", "environment_id": "env_7", "title": "atama: n"})
    );
    // Atama made no agent and no environment, so it wrote no lock file.
    assert!(!path.exists());

    api.0.borrow_mut().agent = json!({"id": "agent_9", "version": 4});
    api.0.borrow_mut().streams = vec![vec![message("m1", "ok"), idle("i1", "end_turn")]].into();
    let (tools, _) = echo();
    run_with(
        "existing-version",
        &api,
        &["task=x", "agent_id=agent_9"],
        "",
        |run| run.with_tools(tools),
    )
    .unwrap();
    let sessions: Vec<Value> = api
        .requests()
        .into_iter()
        .filter(|(name, _)| name == "create_session")
        .map(|(_, body)| body)
        .collect();
    assert_eq!(
        sessions.last().unwrap()["agent"],
        json!({"type": "agent", "id": "agent_9", "version": 4})
    );
}

#[test]
fn makes_the_local_tools_of_local_tools() {
    // Without `with_tools`, the run makes the tools of `local_tools=`, and
    // `on_ask=deny` denies `bash`, because it can change files.
    let api = MockApi::with_streams(vec![vec![
        event(
            json!({"type": "agent.custom_tool_use", "id": "t1", "name": "local_bash", "input": {"command": "echo hi"}}),
        ),
        message("m1", "ok"),
        idle("i1", "end_turn"),
    ]]);
    run_with(
        "local",
        &api,
        &[
            "task=x",
            "local_tools=bash",
            "on_ask=deny",
            "protect=./jobs",
        ],
        "",
        |run| run,
    )
    .unwrap();
    let sent = api.sent();
    let first = sent[0]["content"][0]["text"].as_str().unwrap();
    assert!(first.contains("./jobs"), "{first}");
    assert_eq!(sent[1]["is_error"], true);
    assert!(
        sent[1]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("`on_ask=deny`"),
        "{}",
        sent[1]
    );
}

#[test]
fn writes_the_session_and_the_events_with_debug() {
    let api = MockApi::with_streams(vec![vec![
        event(json!({"type": "session.status_running", "id": "e1"})),
        event(
            json!({"type": "agent.tool_use", "id": "e2", "name": "bash", "input": {}, "evaluated_permission": "ask"}),
        ),
        event(json!({"type": "agent.tool_result", "id": "e3", "is_error": true})),
        event(
            json!({"type": "agent.custom_tool_use", "id": "e4", "name": "local_echo", "input": {"text": "x"}}),
        ),
        event(json!({"type": "session.status_rescheduled", "id": "e5"})),
        event(json!({"type": "session.error", "id": "e6", "error": {"message": "slow"}})),
        event(json!({"type": "span.model_request_start", "id": "e7"})),
        event(
            json!({"type": "session.usage", "id": "e8", "usage": {"input_tokens": 5, "list_cost": {"amount": "12"}}}),
        ),
        message("e9", "Hi"),
        idle("e10", "end_turn"),
    ]]);
    let out = Output::default();
    let asked = Rc::new(RefCell::new(Vec::new()));
    let approver: Rc<RefCell<dyn Approver>> = Rc::new(RefCell::new(Fixed { allow: true, asked }));
    let (tools, _) = echo();
    let debug = out.clone();
    run_with("debug", &api, &["task=x"], "", |run| {
        run.with_tools(tools).approver(approver).debug(debug)
    })
    .unwrap();
    let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
    for expected in [
        "=== agent:claude-agent (claude) agent agent_1 in environment env_1, session sesn_1\n",
        "=== agent:claude-agent (claude) console https://platform.claude.com/workspaces/default/sessions/sesn_1\n",
        "of 30m: session.status_running\n",
        "of 30m: agent.tool_use bash (Ask): {}\n",
        "=== agent:claude-agent (claude) bash: allow\n",
        "of 30m: agent.tool_result (error)\n",
        "of 30m: agent.custom_tool_use local_echo: {\"text\":\"x\"}\n",
        "=== agent:claude-agent (claude) local_echo returned: x\n",
        "of 30m: session.status_rescheduled\n",
        "of 30m: session.error: slow\n",
        "of 30m: span.model_request_start\n",
        "of 30m: session.usage: 5 tokens, $0.12\n",
        "of 30m: agent.message: Hi\n",
        "of 30m: session.status_idle (EndTurn)\n",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
    }
}

#[test]
fn fails_before_the_session_on_invalid_input() {
    let api = MockApi::default();
    assert_eq!(
        run("no-task", &api, &[]).unwrap_err(),
        "agent:claude-agent (claude): the action has no `task=` and no input"
    );
    assert_eq!(
        run("schema-missing", &api, &["task=x", "output_schema=nope"]).unwrap_err(),
        "agent:claude-agent (claude): output schema `nope` is not defined in the job"
    );
    assert_eq!(
        ClaudeAgentRun::new("n")
            .api(api.clone())
            .run_with_env(&[], None, "", &env)
            .unwrap_err(),
        "agent:claude-agent: `agent:claude-agent` needs `model=<profile>`"
    );
    assert!(api.requests().is_empty());
}

#[test]
fn fails_if_an_event_cannot_be_sent() {
    let api = MockApi::with_streams(vec![vec![]]);
    api.0.borrow_mut().send_error = Some((0, "session is busy".into()));
    assert_eq!(
        run("send", &api, &["task=x"]).unwrap_err(),
        "agent:claude-agent (claude): session is busy"
    );

    // A tool result that cannot be sent also fails the run.
    let api = MockApi::with_streams(vec![vec![event(json!({
        "type": "agent.custom_tool_use",
        "id": "t1",
        "name": "local_echo",
        "input": {"text": "a"},
    }))]]);
    api.0.borrow_mut().send_error = Some((1, "session ended".into()));
    assert_eq!(
        run("send-result", &api, &["task=x"]).unwrap_err(),
        "agent:claude-agent (claude): session ended"
    );
}

#[test]
fn waits_for_a_slow_stream_and_uses_the_input_as_the_task() {
    let api = MockApi::with_streams(vec![vec![
        Item::Pause(Duration::from_millis(40)),
        message("m1", "slow"),
        idle("i1", "end_turn"),
    ]]);
    let (tools, _) = echo();
    let output = run_with("slow", &api, &[], "only the input", |run| {
        run.with_tools(tools)
    })
    .unwrap();
    assert_eq!(output, "slow");
    assert_eq!(api.sent(), [user_message("only the input")]);
}

#[test]
fn without_a_mock_the_run_sends_requests_to_the_base_url_of_the_profile() {
    // A port with no server: bind it, then close it.
    let url = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}/v1", listener.local_addr().unwrap())
    };
    let profile = ModelProfile {
        base_url: url.clone(),
        ..profile()
    };
    let error = ClaudeAgentRun::new("n")
        .lock_path(lock_path("http"))
        .run_with_env(&params(&["task=x"]), Some(&profile), "", &env)
        .unwrap_err();
    assert!(
        error.starts_with(&format!(
            "agent:claude-agent (claude): request to `{url}/agents` failed:"
        )),
        "{error}"
    );
}
