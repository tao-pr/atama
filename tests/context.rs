//! Tests for the compaction of the context of an agent (`max_context=`),
//! in `src/agent_loop_openai_compat/context.rs`.

use std::collections::VecDeque;

use serde_json::{Value, json};

use atama::AgentToolLoop;
use atama::actor::AgentType;
use atama::agent_loop_openai_compat::context::{
    SUMMARY_HEADER, SUMMARY_PROMPT, compacted, estimate_tokens, summary_request, transcript,
};
use atama::agent_loop_openai_compat::judge::evidence;
use atama::agent_loop_openai_compat::{ChatClient, Message, Reply, Tool, ToolCall};
use atama::schema::OutputSchema;

fn text(content: &str) -> Reply {
    Reply {
        content: Some(content.into()),
        ..Default::default()
    }
}

fn call(id: &str) -> Reply {
    Reply {
        tool_calls: vec![ToolCall {
            id: id.into(),
            name: "big".into(),
            arguments: "{}".into(),
        }],
        ..Default::default()
    }
}

fn with_tokens(reply: Reply, tokens: u64) -> Reply {
    Reply {
        tokens: Some(tokens),
        ..reply
    }
}

/// One request: its messages, and whether it was a request without tools.
struct Request {
    messages: Vec<Message>,
    without_tools: bool,
}

/// A client with scripted replies that records each request. A reply of
/// `Err` fails the request.
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

    fn summary_requests(&self) -> usize {
        self.requests.iter().filter(|r| r.without_tools).count()
    }
}

impl ChatClient for ScriptedClient {
    fn chat(&mut self, messages: &[Message], _: &[Box<dyn Tool>]) -> Result<Reply, String> {
        self.requests.push(Request {
            messages: messages.to_vec(),
            without_tools: false,
        });
        self.replies.pop_front().expect("no more replies")
    }

    fn chat_without_tools(
        &mut self,
        messages: &[Message],
        format: Option<&OutputSchema>,
    ) -> Result<Reply, String> {
        assert!(format.is_none(), "the summary request has no format");
        self.requests.push(Request {
            messages: messages.to_vec(),
            without_tools: true,
        });
        self.replies.pop_front().expect("no more replies")
    }
}

/// A tool `big` that returns 4,000 characters.
struct Big;

impl Tool for Big {
    fn name(&self) -> &str {
        "big"
    }

    fn description(&self) -> &str {
        "returns a big result"
    }

    fn parameters(&self) -> Value {
        json!({"type": "object"})
    }

    fn call(&mut self, _: &Value) -> Result<String, String> {
        Ok("x".repeat(4_000))
    }
}

fn content(message: &Message) -> String {
    match message {
        Message::System(text) | Message::User(text) => text.clone(),
        other => panic!("expected a system or user message, got {other:?}"),
    }
}

fn coder() -> AgentToolLoop<'static> {
    AgentToolLoop::new(AgentType::Coder).with_tools(vec![Box::new(Big)])
}

#[test]
fn estimates_four_characters_for_each_token() {
    // `{"role":"user","content":"abcd"}` has 32 characters.
    assert_eq!(estimate_tokens(&[Message::User("abcd".into())]), 8);
    // `{"role":"user","content":"abcde"}` has 33 characters, so 9 tokens.
    assert_eq!(estimate_tokens(&[Message::User("abcde".into())]), 9);
    assert_eq!(estimate_tokens(&[]), 0);
}

#[test]
fn the_transcript_has_one_line_for_each_part_of_a_message() {
    let messages = [
        Message::System("ignored".into()),
        Message::User("feedback".into()),
        Message::Assistant(Reply {
            content: Some("I look.".into()),
            ..call("1")
        }),
        Message::Assistant(text("")),
        Message::Tool {
            call_id: "1".into(),
            content: "found".into(),
        },
    ];
    assert_eq!(
        transcript(&messages),
        "user: feedback\nassistant: I look.\ncall big: {}\nresult: found"
    );
}

#[test]
fn the_summary_request_has_the_task_and_the_conversation_after_it() {
    let [system, user] = summary_request("Fix it.", &[Message::User("go on".into())]);
    assert_eq!(content(&system), SUMMARY_PROMPT);
    assert_eq!(
        content(&user),
        "Task of the agent:\nFix it.\n\nConversation after the task:\nuser: go on"
    );
}

#[test]
fn the_compacted_conversation_keeps_the_system_prompt_and_the_task() {
    let conversation = [
        Message::System("system".into()),
        Message::User("task".into()),
        Message::Assistant(call("1")),
        Message::Tool {
            call_id: "1".into(),
            content: "result".into(),
        },
    ];
    let messages = compacted(&conversation, "did a lot");
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[..2], conversation[..2]);
    assert_eq!(
        content(&messages[2]),
        format!("{SUMMARY_HEADER}\ndid a lot\n\nGo on with the task.")
    );
}

#[test]
fn the_evidence_of_the_judge_has_the_summary() {
    let messages = compacted(
        &[Message::System("s".into()), Message::User("t".into())],
        "ran the tests",
    );
    assert_eq!(
        evidence(&messages),
        format!("{SUMMARY_HEADER}\nran the tests\n\nGo on with the task.")
    );
}

#[test]
fn without_max_context_the_loop_never_summarizes() {
    let mut client = ScriptedClient::new(vec![call("1"), call("2"), call("3"), text("done")]);
    let output = coder().run_with(&mut client, "task", "").unwrap();
    assert_eq!(output, "done");
    assert_eq!(client.summary_requests(), 0);
    assert_eq!(client.requests[3].messages.len(), 8);
}

#[test]
fn summarizes_before_a_request_that_is_over_the_limit() {
    let mut client = ScriptedClient::new(vec![
        call("1"),
        text("the agent read a big file"),
        call("2"),
        text("the agent read two big files"),
        text("done"),
    ]);
    let mut out = Vec::new();
    let output = coder()
        .max_context(Some(500))
        .debug(&mut out)
        .run_with(&mut client, "task", "")
        .unwrap();
    assert_eq!(output, "done");
    assert_eq!(client.summary_requests(), 2);

    // The first request is under the limit. The second one is over it, so
    // the summary request comes before it.
    assert!(!client.requests[0].without_tools);
    let summary = &client.requests[1];
    assert!(summary.without_tools);
    assert_eq!(content(&summary.messages[0]), SUMMARY_PROMPT);
    assert!(
        content(&summary.messages[1]).contains("call big: {}\nresult: xxxx"),
        "{}",
        content(&summary.messages[1])
    );
    let second = &client.requests[2].messages;
    assert_eq!(second.len(), 3);
    assert_eq!(
        content(&second[2]),
        format!("{SUMMARY_HEADER}\nthe agent read a big file\n\nGo on with the task.")
    );
    // The second summary has the first one in its transcript.
    assert!(content(&client.requests[3].messages[1]).contains("user: Summary of the work so far:"),);

    let out = String::from_utf8(out).unwrap();
    assert!(
        out.contains("=== agent:coder step 2: context compacted (4 messages, about "),
        "{out}"
    );
    assert!(out.contains("to 3 messages, about "), "{out}");
    assert!(
        out.contains("tokens) ===\nthe agent read a big file"),
        "{out}"
    );
    assert!(
        out.contains("=== agent:coder step 2: request (3 messages, 1 new"),
        "{out}"
    );
}

#[test]
fn the_tokens_of_the_summary_count_in_the_budget() {
    let mut client = ScriptedClient::new(vec![
        with_tokens(call("1"), 40),
        with_tokens(text("summary"), 70),
        with_tokens(call("2"), 10),
    ]);
    let err = coder()
        .max_context(Some(500))
        .token_budget(Some(100))
        .run_with(&mut client, "task", "")
        .unwrap_err();
    // Step 2 starts with 40 tokens. The summary before its request adds 70,
    // and its reply 10, so step 3 stops at 120.
    assert_eq!(
        err,
        "`agent:coder` used 120 tokens in 2 steps, and its token budget is 100"
    );
}

#[test]
fn fails_if_the_summary_request_fails_or_has_no_token_count() {
    let mut client = ScriptedClient::new(vec![call("1")]);
    client.replies.push_back(Err("connection refused".into()));
    let err = coder()
        .max_context(Some(500))
        .run_with(&mut client, "task", "")
        .unwrap_err();
    assert_eq!(
        err,
        "`agent:coder` could not summarize its context: connection refused"
    );

    let mut client = ScriptedClient::new(vec![with_tokens(call("1"), 40), text("summary")]);
    let err = coder()
        .max_context(Some(500))
        .token_budget(Some(1000))
        .run_with(&mut client, "task", "")
        .unwrap_err();
    assert!(err.contains("did not send `usage.total_tokens`"), "{err}");
}
