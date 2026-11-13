//! Tests for the judges of an `autonomous` agent: the success criteria
//! (`success=`) and the guard (`guard=judge`), in
//! `src/agent_loop_openai_compat/judge.rs`.

use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;

use serde_json::{Value, json};

use atama::AgentToolLoop;
use atama::actor::AgentType;
use atama::agent_loop_openai_compat::agent_options::Guard;
use atama::agent_loop_openai_compat::checks::{CheckResult, FinalCheck};
use atama::agent_loop_openai_compat::judge::{
    Judge, MAX_EVIDENCE_CHARS, SuccessJudge, Verdict, evidence, review, verdict_schema,
};
use atama::agent_loop_openai_compat::{ChatClient, Message, Reply, Tool, ToolCall};
use atama::schema::OutputSchema;

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

fn call(id: &str, name: &str, arguments: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments: arguments.into(),
    }
}

fn calls(id: &str, name: &str) -> Reply {
    Reply {
        tool_calls: vec![call(id, name, "{\"command\": \"ls\"}")],
        ..Default::default()
    }
}

/// One request that the client got.
#[derive(Debug)]
struct Request {
    messages: Vec<Message>,
    /// The name of the format of a judge request, or `None` for a request
    /// of the agent.
    format: Option<String>,
}

/// A client that gives scripted replies to the agent and the judges, in
/// one queue, and records each request. A reply of `Err` fails the request.
struct ScriptedClient {
    replies: VecDeque<Result<Reply, String>>,
    requests: Vec<Request>,
}

impl ScriptedClient {
    fn new(replies: Vec<Reply>) -> Self {
        ScriptedClient {
            replies: replies.into_iter().map(Ok).collect(),
            requests: Vec::new(),
        }
    }

    fn next(&mut self) -> Result<Reply, String> {
        self.replies.pop_front().expect("no more replies")
    }

    /// The requests of the judges.
    fn judge_requests(&self) -> Vec<&Request> {
        self.requests
            .iter()
            .filter(|r| r.format.is_some())
            .collect()
    }
}

impl ChatClient for ScriptedClient {
    fn chat(&mut self, messages: &[Message], _: &[Box<dyn Tool>]) -> Result<Reply, String> {
        self.requests.push(Request {
            messages: messages.to_vec(),
            format: None,
        });
        self.next()
    }

    fn chat_with_format(
        &mut self,
        messages: &[Message],
        format: &OutputSchema,
    ) -> Result<Reply, String> {
        self.requests.push(Request {
            messages: messages.to_vec(),
            format: Some(format.name.clone()),
        });
        self.next()
    }
}

/// The text of a message.
fn content(message: &Message) -> String {
    match message {
        Message::System(text) | Message::User(text) => text.clone(),
        Message::Tool { content, .. } => content.clone(),
        Message::Assistant(reply) => reply.content.clone().unwrap_or_default(),
    }
}

/// A tool that counts its calls.
struct Counter {
    calls: Rc<Cell<u32>>,
}

impl Tool for Counter {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "counts"
    }

    fn parameters(&self) -> Value {
        json!({"type": "object"})
    }

    fn call(&mut self, _: &Value) -> Result<String, String> {
        self.calls.set(self.calls.get() + 1);
        Ok("listed".into())
    }
}

fn counter() -> (Box<dyn Tool>, Rc<Cell<u32>>) {
    let calls = Rc::new(Cell::new(0));
    (
        Box::new(Counter {
            calls: calls.clone(),
        }),
        calls,
    )
}

fn autonomous() -> AgentToolLoop<'static> {
    AgentToolLoop::new(AgentType::Autonomous)
        .max_iterations(5)
        .max_steps(50)
}

// Judge

#[test]
fn the_verdict_schema_has_pass_and_reason() {
    let schema = verdict_schema();
    assert_eq!(schema.name, "verdict");
    assert_eq!(schema.schema["required"], json!(["pass", "reason"]));
}

#[test]
fn a_judge_asks_with_the_verdict_format_and_keeps_the_tokens() {
    let mut client = ScriptedClient::new(vec![with_tokens(verdict(false, "no tests"), 7)]);
    let mut judge = Judge::new(&mut client);
    let result = judge.ask("Decide.", "Is it done?");
    assert_eq!(
        result,
        Ok(Verdict {
            pass: false,
            reason: "no tests".into()
        })
    );
    assert_eq!(judge.tokens, [Some(7)]);
    let request = &client.requests[0];
    assert_eq!(request.format.as_deref(), Some("verdict"));
    assert_eq!(request.messages.len(), 2);
    assert!(content(&request.messages[0]).starts_with("Decide.\n\nYour final reply"));
    assert_eq!(content(&request.messages[1]), "Is it done?");
}

#[test]
fn a_judge_fails_on_an_invalid_reply_or_a_client_error() {
    let mut client = ScriptedClient::new(vec![text("yes")]);
    client.replies.push_back(Err("connection refused".into()));
    let mut judge = Judge::new(&mut client);
    let err = judge.ask("Decide.", "?").unwrap_err();
    assert!(
        err.starts_with("the reply of the judge is not valid: the reply is not valid JSON"),
        "{err}"
    );
    assert_eq!(judge.ask("Decide.", "?"), Err("connection refused".into()));
    assert_eq!(judge.tokens, [None]);
}

#[test]
fn evidence_lists_the_tool_calls_and_results_and_keeps_the_end() {
    assert_eq!(evidence(&[]), "(no tool calls)");
    let messages = [
        Message::System("s".into()),
        Message::User("u".into()),
        Message::Assistant(Reply {
            tool_calls: vec![call("1", "bash", "{\"command\": \"ls\"}")],
            ..Default::default()
        }),
        Message::Tool {
            call_id: "1".into(),
            content: "a.txt".into(),
        },
    ];
    assert_eq!(
        evidence(&messages),
        "call bash: {\"command\": \"ls\"}\nresult: a.txt"
    );
    let long = [Message::Tool {
        call_id: "1".into(),
        content: "x".repeat(MAX_EVIDENCE_CHARS + 10),
    }];
    let text = evidence(&long);
    assert!(text.starts_with("[cut: the output has "), "{}", &text[..60]);
    assert!(text.ends_with("xxx"));
}

#[test]
fn the_success_judge_passes_or_gives_the_reason() {
    let mut check = SuccessJudge::new("Fix it.", "All tests pass.");
    assert!(check.ends_iteration());
    assert_eq!(check.rejection(), "success criteria are not met");
    assert_eq!(check.failure(), "did not meet its success criteria");

    let mut client = ScriptedClient::new(vec![verdict(true, "ok"), verdict(false, "2 fail")]);
    client.replies.push_back(Err("timeout".into()));
    let mut judge = Judge::new(&mut client);
    let messages = [Message::Tool {
        call_id: "1".into(),
        content: "2 tests fail".into(),
    }];
    assert_eq!(
        check.check("done", &mut judge, &messages),
        CheckResult::Done("done".into())
    );
    assert_eq!(
        check.check("done", &mut judge, &messages),
        CheckResult::Retry {
            message: "The task is not done yet, because the work does not meet the success \
                      criteria:\n2 fail\nContinue the work. When you are done, reply without \
                      tool calls."
                .into(),
            error: "2 fail".into(),
        }
    );
    let CheckResult::Retry { error, .. } = check.check("done", &mut judge, &messages) else {
        panic!("a judge that fails does not pass the check");
    };
    assert_eq!(error, "timeout");

    let question = content(&client.requests[0].messages[1]);
    assert_eq!(
        question,
        "Task:\nFix it.\n\nSuccess criteria:\nAll tests pass.\n\nThe tool calls of the agent \
         and their results, the most recent last:\nresult: 2 tests fail\n\nThe final reply of \
         the agent:\ndone\n\nIs \"pass\" true? It is true only if the work meets all the \
         criteria."
    );
}

#[test]
fn the_guard_allows_or_blocks_a_call() {
    let mut client = ScriptedClient::new(vec![verdict(true, "fine"), verdict(false, "pushes")]);
    client.replies.push_back(Err("timeout".into()));
    let mut judge = Judge::new(&mut client);
    let push = call("1", "bash", "{\"command\": \"git push\"}");
    assert_eq!(review(&mut judge, "Do not push.", &push), None);
    assert_eq!(
        review(&mut judge, "Do not push.", &push),
        Some("pushes".into())
    );
    assert_eq!(
        review(&mut judge, "Do not push.", &push),
        Some("timeout".into())
    );
    assert_eq!(
        content(&client.requests[0].messages[1]),
        "Guardrails:\nDo not push.\n\nTool: bash\nArguments:\n{\"command\": \"git push\"}"
    );
}

// Loop

#[test]
fn the_loop_runs_until_the_judge_says_that_the_criteria_are_met() {
    let mut client = ScriptedClient::new(vec![
        text("done 1"),
        verdict(false, "no test ran"),
        text("done 2"),
        verdict(true, "tests pass"),
    ]);
    let mut out = Vec::new();
    let output = autonomous()
        .success(Some("All tests pass.".into()))
        .debug(&mut out)
        .run_with(&mut client, "Fix it.", "")
        .unwrap();
    assert_eq!(output, "done 2");
    let system = content(&client.requests[0].messages[0]);
    assert!(
        system.contains(
            "The task is done when your work meets these success criteria:\nAll tests pass.\n"
        ),
        "{system}"
    );
    let third = &client.requests[2].messages;
    assert!(
        content(&third[3]).ends_with("criteria:\nno test ran\nContinue the work. When you are done, reply without tool calls.\nThis is iteration 2 of 5."),
        "{}",
        content(&third[3])
    );
    let out = String::from_utf8(out).unwrap();
    assert!(
        out.contains(
            "=== agent:autonomous iteration 1 step 1: success criteria are not met ===\nno test ran"
        ),
        "{out}"
    );
}

#[test]
fn the_command_check_runs_before_the_judge() {
    let mut client = ScriptedClient::new(vec![text("done"), text("done")]);
    let err = autonomous()
        .max_iterations(2)
        .success_check(Some("echo red; exit 1".into()))
        .success(Some("All tests pass.".into()))
        .run_with(&mut client, "Fix it.", "")
        .unwrap_err();
    assert!(client.judge_requests().is_empty());
    assert_eq!(
        err,
        "`agent:autonomous` did not pass its success check `echo red; exit 1` in 2 \
         iterations, the last error: exit status: 1\nred"
    );
}

#[test]
fn fails_after_all_iterations_with_the_last_reason_of_the_judge() {
    let mut client = ScriptedClient::new(vec![text("done"), verdict(false, "no tests")]);
    let err = autonomous()
        .max_iterations(1)
        .success(Some("All tests pass.".into()))
        .run_with(&mut client, "Fix it.", "")
        .unwrap_err();
    assert_eq!(
        err,
        "`agent:autonomous` did not meet its success criteria in 1 iterations, the last \
         error: no tests"
    );
}

#[test]
fn the_guard_blocks_a_call_that_breaks_a_guardrail() {
    let (tool, tool_calls) = counter();
    let mut client = ScriptedClient::new(vec![
        calls("c1", "bash"),
        verdict(false, "it pushes"),
        calls("c2", "bash"),
        verdict(true, "fine"),
        text("done"),
    ]);
    let mut out = Vec::new();
    let output = autonomous()
        .success_check(Some("true".into()))
        .guardrails(Some("Do not push.".into()))
        .guard(Guard::Judge)
        .with_tools(vec![tool])
        .debug(&mut out)
        .run_with(&mut client, "Fix it.", "")
        .unwrap();
    assert_eq!(output, "done");
    assert_eq!(tool_calls.get(), 1);
    assert_eq!(client.judge_requests().len(), 2);
    let second = &client.requests[2].messages;
    assert_eq!(
        content(&second[3]),
        "error: blocked by guardrail: it pushes"
    );
    let out = String::from_utf8(out).unwrap();
    assert!(
        out.contains("tool call 1 of 1: bash (id c1) blocked by the guard ===\narguments:\n"),
        "{out}"
    );
    assert!(out.contains("reason: it pushes"), "{out}");
}

#[test]
fn the_run_fails_after_max_violations_blocked_calls() {
    let (tool, tool_calls) = counter();
    let mut client = ScriptedClient::new(vec![
        calls("c1", "bash"),
        verdict(false, "first"),
        calls("c2", "bash"),
        verdict(false, "second"),
    ]);
    let err = autonomous()
        .success_check(Some("true".into()))
        .guardrails(Some("Do not push.".into()))
        .guard(Guard::Judge)
        .max_violations(2)
        .with_tools(vec![tool])
        .run_with(&mut client, "Fix it.", "")
        .unwrap_err();
    assert_eq!(
        err,
        "`agent:autonomous` made 2 tool calls that break its guardrails, the last reason: second"
    );
    assert_eq!(tool_calls.get(), 0);
}

#[test]
fn the_prompt_guard_asks_no_judge() {
    let (tool, tool_calls) = counter();
    let mut client = ScriptedClient::new(vec![calls("c1", "bash"), text("done")]);
    autonomous()
        .success_check(Some("true".into()))
        .guardrails(Some("Do not push.".into()))
        .with_tools(vec![tool])
        .run_with(&mut client, "Fix it.", "")
        .unwrap();
    assert_eq!(tool_calls.get(), 1);
    assert!(client.judge_requests().is_empty());
}

#[test]
fn the_tokens_of_the_judges_count_in_the_budget() {
    let (tool, _) = counter();
    let mut client = ScriptedClient::new(vec![
        with_tokens(calls("c1", "bash"), 30),
        with_tokens(verdict(true, "fine"), 20),
        with_tokens(text("done"), 30),
        with_tokens(verdict(false, "no tests"), 30),
    ]);
    let err = autonomous()
        .token_budget(Some(100))
        .success(Some("All tests pass.".into()))
        .guardrails(Some("Do not push.".into()))
        .guard(Guard::Judge)
        .with_tools(vec![tool])
        .run_with(&mut client, "Fix it.", "")
        .unwrap_err();
    assert_eq!(
        err,
        "`agent:autonomous` used 110 tokens in 2 steps, and its token budget is 100, the last \
         error: no tests"
    );
}

#[test]
fn a_judge_reply_without_tokens_fails_a_run_with_a_budget() {
    let (tool, _) = counter();
    let mut client = ScriptedClient::new(vec![
        with_tokens(calls("c1", "bash"), 30),
        verdict(true, "fine"),
    ]);
    let err = autonomous()
        .token_budget(Some(100))
        .success_check(Some("true".into()))
        .guardrails(Some("Do not push.".into()))
        .guard(Guard::Judge)
        .with_tools(vec![tool])
        .run_with(&mut client, "Fix it.", "")
        .unwrap_err();
    assert!(err.contains("did not send `usage.total_tokens`"), "{err}");

    let mut client = ScriptedClient::new(vec![with_tokens(text("done"), 30), verdict(true, "ok")]);
    let err = autonomous()
        .token_budget(Some(100))
        .success(Some("All tests pass.".into()))
        .run_with(&mut client, "Fix it.", "")
        .unwrap_err();
    assert!(err.contains("did not send `usage.total_tokens`"), "{err}");
}
