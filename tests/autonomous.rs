//! Tests for the `autonomous` agent: iterations, the success check, the
//! guardrails, the time limit and the tool limits of the agent loop.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use serde_json::Value;

use atama::AgentToolLoop;
use atama::actor::AgentType;
use atama::agent_loop_openai_compat::openai::request_body;
use atama::agent_loop_openai_compat::tools::{
    AUTONOMOUS_DEFAULT_TOOLS, DEFAULT_MAX_CALLS, DEFAULT_MAX_FAILURES, Limited, default_tools,
};
use atama::agent_loop_openai_compat::{ChatClient, Message, Reply, Tool, ToolCall};
use atama::schema::OutputSchema;

fn text(content: &str) -> Reply {
    Reply {
        content: Some(content.into()),
        ..Default::default()
    }
}

fn call(id: &str, name: &str) -> Reply {
    calls(&[(id, name)])
}

fn calls(calls: &[(&str, &str)]) -> Reply {
    Reply {
        tool_calls: calls
            .iter()
            .map(|(id, name)| ToolCall {
                id: id.to_string(),
                name: name.to_string(),
                arguments: "{}".into(),
            })
            .collect(),
        ..Default::default()
    }
}

/// A client that gives scripted replies and records each conversation.
#[derive(Default)]
struct ScriptedClient {
    replies: VecDeque<Reply>,
    conversations: Vec<Vec<Message>>,
}

impl ScriptedClient {
    fn new(replies: Vec<Reply>) -> Self {
        ScriptedClient {
            replies: replies.into(),
            ..Default::default()
        }
    }

    /// The text of each user message of the last conversation.
    fn user_messages(&self) -> Vec<String> {
        self.conversations
            .last()
            .unwrap()
            .iter()
            .filter_map(|message| match message {
                Message::User(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    fn system_prompt(&self) -> String {
        match &self.conversations[0][0] {
            Message::System(text) => text.clone(),
            other => panic!("the first message is not the system prompt: {other:?}"),
        }
    }
}

impl ChatClient for ScriptedClient {
    fn chat(&mut self, messages: &[Message], _: &[Box<dyn Tool>]) -> Result<Reply, String> {
        self.conversations.push(messages.to_vec());
        Ok(self
            .replies
            .pop_front()
            .expect("the loop asked for more replies than the script has"))
    }
}

/// A tool that counts its calls, moves the fake clock by `advance`, and
/// gives `result`.
struct Counter {
    name: &'static str,
    result: Result<String, String>,
    calls: Rc<Cell<u32>>,
    clock: Option<(Now, Duration)>,
}

impl Tool for Counter {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "counts"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({"type": "object"})
    }

    fn call(&mut self, _: &Value) -> Result<String, String> {
        self.calls.set(self.calls.get() + 1);
        if let Some((now, advance)) = &self.clock {
            now.set(now.get() + *advance);
        }
        self.result.clone()
    }
}

fn counter(name: &'static str, result: Result<&str, &str>) -> (Box<dyn Tool>, Rc<Cell<u32>>) {
    let calls = Rc::new(Cell::new(0));
    let tool = Counter {
        name,
        result: result.map(str::to_string).map_err(str::to_string),
        calls: calls.clone(),
        clock: None,
    };
    (Box::new(tool), calls)
}

/// A fake clock, and a tool `wait` that moves it by `advance`.
/// The time of the fake clock.
type Now = Rc<Cell<Instant>>;

fn fake_clock(advance: Duration) -> (Now, Box<dyn Tool>, Rc<Cell<u32>>) {
    let now = Rc::new(Cell::new(Instant::now()));
    let calls = Rc::new(Cell::new(0));
    let tool = Counter {
        name: "wait",
        result: Ok("waited".into()),
        calls: calls.clone(),
        clock: Some((now.clone(), advance)),
    };
    (now, Box::new(tool), calls)
}

/// A success check that fails `failures` times, then passes. It counts its
/// runs in a fresh file of the target directory.
fn check_that_passes_after(name: &str, failures: u32) -> String {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.count"));
    let _ = std::fs::remove_file(&path);
    let path = path.display();
    format!(
        "n=$(cat '{path}' 2>/dev/null || echo 0); echo $((n+1)) > '{path}'; \
         [ \"$n\" -ge {failures} ] || {{ echo \"run $n fails\"; exit 1; }}"
    )
}

fn autonomous() -> AgentToolLoop<'static> {
    AgentToolLoop::new(AgentType::Autonomous)
        .max_iterations(5)
        .max_steps(50)
}

#[test]
fn stops_at_the_first_reply_that_passes_the_success_check() {
    let mut client = ScriptedClient::new(vec![text("all done")]);
    let output = autonomous()
        .success_check(Some("true".into()))
        .guardrails(Some("Do not push.".into()))
        .with_tools(Vec::new())
        .run_with(&mut client, "fix it", "")
        .unwrap();
    assert_eq!(output, "all done");
    assert_eq!(client.conversations.len(), 1);
    let system = client.system_prompt();
    assert!(
        system.starts_with("You are an autonomous agent"),
        "{system}"
    );
    assert!(
        system.contains(
            "When you reply without tool calls, the success check `true` runs. If it exits \
             with status 0, the task is done."
        ),
        "{system}"
    );
    assert!(
        system.ends_with("Guardrails. You must obey these rules in all your work:\nDo not push."),
        "{system}"
    );
}

#[test]
fn a_failed_success_check_starts_the_next_iteration() {
    let check = check_that_passes_after("next_iteration", 2);
    let (tool, tool_calls) = counter("edit", Ok("edited"));
    let mut client = ScriptedClient::new(vec![
        text("done 1"),
        call("c1", "edit"),
        text("done 2"),
        text("done 3"),
    ]);
    let mut out = Vec::new();
    let output = autonomous()
        .success_check(Some(check))
        .with_tools(vec![tool])
        .debug(&mut out)
        .run_with(&mut client, "fix it", "")
        .unwrap();
    assert_eq!(output, "done 3");
    assert_eq!(tool_calls.get(), 1);

    let users = client.user_messages();
    assert_eq!(users.len(), 3);
    assert!(
        users[1].starts_with("The task is not done yet, because the success check `"),
        "{}",
        users[1]
    );
    assert!(
        users[1].ends_with(
            "exit status: 1\nrun 0 fails\nContinue the work. When you are done, reply without \
             tool calls.\nThis is iteration 2 of 5."
        ),
        "{}",
        users[1]
    );
    assert!(
        users[2].ends_with("This is iteration 3 of 5."),
        "{}",
        users[2]
    );

    let out = String::from_utf8(out).unwrap();
    assert!(
        out.contains("=== agent:autonomous iteration 1 step 1: request ("),
        "{out}"
    );
    assert!(
        out.contains("=== agent:autonomous iteration 2 step 2: tool call 1 of 1: edit"),
        "{out}"
    );
    assert!(
        out.contains("=== agent:autonomous iteration 1 step 1: success check `"),
        "{out}"
    );
}

#[test]
fn fails_with_the_last_check_error_after_all_iterations() {
    let mut client = ScriptedClient::new(vec![text("done"), text("done again")]);
    let err = autonomous()
        .max_iterations(2)
        .success_check(Some("echo nope; exit 1".into()))
        .run_with(&mut client, "fix it", "")
        .unwrap_err();
    assert_eq!(
        err,
        "`agent:autonomous` did not pass its success check `echo nope; exit 1` in 2 \
         iterations, the last error: exit status: 1\nnope"
    );
}

#[test]
fn max_steps_counts_the_steps_of_all_iterations() {
    let (tool, _) = counter("edit", Ok("edited"));
    let mut client =
        ScriptedClient::new(vec![text("done"), call("c1", "edit"), call("c2", "edit")]);
    let err = autonomous()
        .max_steps(3)
        .success_check(Some("exit 1".into()))
        .with_tools(vec![tool])
        .run_with(&mut client, "fix it", "")
        .unwrap_err();
    assert_eq!(
        err,
        "`agent:autonomous` did not pass its success check `exit 1` in 3 steps, the last \
         error: exit status: 1\n"
    );
}

#[test]
fn stops_before_a_request_when_the_time_is_used() {
    let (now, tool, waits) = fake_clock(Duration::from_secs(40));
    let clock = now.clone();
    let mut client = ScriptedClient::new(vec![call("c1", "wait"), call("c2", "wait")]);
    let mut out = Vec::new();
    let err = autonomous()
        .max_time(Some(Duration::from_secs(60)))
        .clock(move || clock.get())
        .with_tools(vec![tool])
        .debug(&mut out)
        .run_with(&mut client, "fix it", "")
        .unwrap_err();
    assert_eq!(
        err,
        "`agent:autonomous` did not finish in its time limit of 1m"
    );
    assert_eq!(waits.get(), 2);
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("tools: wait, 0s of 1m) ==="), "{out}");
    assert!(out.contains("tools: wait, 40s of 1m) ==="), "{out}");
}

#[test]
fn stops_before_a_tool_call_when_the_time_is_used() {
    let (now, tool, waits) = fake_clock(Duration::from_secs(70));
    let clock = now.clone();
    let mut client =
        ScriptedClient::new(vec![text("done"), calls(&[("c1", "wait"), ("c2", "wait")])]);
    let err = autonomous()
        .max_time(Some(Duration::from_secs(60)))
        .clock(move || clock.get())
        .success_check(Some("echo nope; exit 1".into()))
        .with_tools(vec![tool])
        .run_with(&mut client, "fix it", "")
        .unwrap_err();
    assert_eq!(
        err,
        "`agent:autonomous` did not finish in its time limit of 1m, the last error: \
         exit status: 1\nnope"
    );
    assert_eq!(waits.get(), 1);
}

#[test]
fn the_token_error_adds_the_last_error_of_the_success_check() {
    let mut client = ScriptedClient::new(vec![Reply {
        tokens: Some(120),
        ..text("done")
    }]);
    let err = autonomous()
        .token_budget(Some(100))
        .success_check(Some("echo nope; exit 1".into()))
        .run_with(&mut client, "fix it", "")
        .unwrap_err();
    assert_eq!(
        err,
        "`agent:autonomous` used 120 tokens in 1 steps, and its token budget is 100, the last \
         error: exit status: 1\nnope"
    );
}

#[test]
fn the_token_error_does_not_add_a_schema_error() {
    let schema = OutputSchema {
        name: "article".into(),
        schema: serde_json::json!({"type": "object"}),
    };
    let mut client = ScriptedClient::new(vec![Reply {
        tokens: Some(120),
        ..text("not json")
    }]);
    let err = AgentToolLoop::new(AgentType::Coder)
        .token_budget(Some(100))
        .output_schema(Some(schema))
        .run_with(&mut client, "task", "")
        .unwrap_err();
    assert_eq!(
        err,
        "`agent:coder` used 120 tokens in 1 steps, and its token budget is 100"
    );
}

#[test]
fn the_output_schema_check_runs_before_the_success_check() {
    let schema = OutputSchema {
        name: "article".into(),
        schema: serde_json::json!({"type": "object"}),
    };
    let check = check_that_passes_after("schema_first", 0);
    let mut client = ScriptedClient::new(vec![text("not json"), text("{\"a\": 1}")]);
    let output = autonomous()
        .output_schema(Some(schema))
        .success_check(Some(check))
        .run_with(&mut client, "task", "")
        .unwrap();
    assert_eq!(output, "{\"a\": 1}");
    let users = client.user_messages();
    assert!(
        users[1].starts_with("Your reply is not valid: "),
        "{}",
        users[1]
    );
    assert!(!users[1].contains("This is iteration"), "{}", users[1]);
}

// Tool limits

#[test]
fn autonomous_gets_the_default_tools_with_limits() {
    let tools = default_tools(AgentType::Autonomous);
    let names: Vec<&str> = tools.iter().map(|tool| tool.name()).collect();
    assert_eq!(names, AUTONOMOUS_DEFAULT_TOOLS);
    assert!(tools.iter().all(|tool| tool.available()));
    assert_eq!(DEFAULT_MAX_CALLS, 50);
    assert_eq!(DEFAULT_MAX_FAILURES, 10);
}

#[test]
fn a_tool_at_its_call_limit_fails_and_leaves_the_tool_list() {
    let (inner, inner_calls) = counter("edit", Ok("edited"));
    let mut tool = Limited::new(inner, Some(2), None);
    assert_eq!(tool.description(), "counts");
    assert_eq!(tool.parameters()["type"], "object");
    assert_eq!(tool.call(&Value::Null), Ok("edited".into()));
    assert!(tool.available());
    assert_eq!(tool.call(&Value::Null), Ok("edited".into()));
    assert!(!tool.available());
    assert_eq!(
        tool.call(&Value::Null),
        Err("tool `edit` used its limit of 2 calls".into())
    );
    assert_eq!(inner_calls.get(), 2);

    let tools: Vec<Box<dyn Tool>> = vec![Box::new(tool)];
    let body = request_body("m", &[Message::User("x".into())], &tools, None, None);
    assert!(body.get("tools").is_none(), "{body}");
}

#[test]
fn a_tool_at_its_failure_limit_fails_and_leaves_the_tool_list() {
    let (inner, inner_calls) = counter("edit", Err("broken"));
    let mut tool = Limited::new(inner, None, Some(2));
    assert_eq!(tool.call(&Value::Null), Err("broken".into()));
    assert_eq!(tool.call(&Value::Null), Err("broken".into()));
    assert!(!tool.available());
    assert_eq!(
        tool.call(&Value::Null),
        Err("tool `edit` failed 2 times, which is its limit".into())
    );
    assert_eq!(inner_calls.get(), 2);
}

#[test]
fn the_loop_offers_only_available_tools() {
    let (inner, _) = counter("edit", Ok("edited"));
    let (other, _) = counter("read", Ok("text"));
    let limited: Box<dyn Tool> = Box::new(Limited::new(inner, Some(1), None));
    let mut client =
        ScriptedClient::new(vec![call("c1", "edit"), call("c2", "edit"), text("done")]);
    let mut out = Vec::new();
    let mut agent_loop = AgentToolLoop::new(AgentType::Coder)
        .with_tools(vec![limited, other])
        .debug(&mut out);
    let output = agent_loop.run_with(&mut client, "task", "").unwrap();
    assert_eq!(output, "done");
    assert_eq!(agent_loop.tool_names(), ["read"]);
    drop(agent_loop);
    let out = String::from_utf8(out).unwrap();
    assert!(
        out.contains("step 1: request (2 messages, 2 new, tools: edit, read) ==="),
        "{out}"
    );
    assert!(
        out.contains("step 2: request (4 messages, 2 new, tools: read) ==="),
        "{out}"
    );
    let Message::Tool { content, .. } = &client.conversations[2][5] else {
        panic!("the second call has a tool result");
    };
    assert_eq!(content, "error: tool `edit` used its limit of 1 calls");
}

#[test]
fn the_steps_of_coder_have_no_iteration_in_debug_headers() {
    let mut client = ScriptedClient::new(vec![text("done")]);
    let mut out = Vec::new();
    AgentToolLoop::new(AgentType::Coder)
        .debug(&mut out)
        .run_with(&mut client, "task", "")
        .unwrap();
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("=== agent:coder step 1: request ("), "{out}");
    assert!(!out.contains("iteration"), "{out}");
}

#[test]
fn a_run_without_a_time_limit_does_not_use_the_clock_for_debug() {
    let used = Rc::new(RefCell::new(0));
    let counted = used.clone();
    let mut client = ScriptedClient::new(vec![text("done")]);
    let mut out = Vec::new();
    autonomous()
        .clock(move || {
            *counted.borrow_mut() += 1;
            Instant::now()
        })
        .debug(&mut out)
        .run_with(&mut client, "task", "")
        .unwrap();
    assert_eq!(*used.borrow(), 1);
    let out = String::from_utf8(out).unwrap();
    assert!(!out.contains(" of 30m"), "{out}");
}
