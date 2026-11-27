//! Tests of the SSE reader in `src/agent_loop_claude/sse.rs`.

use atama::agent_loop_claude::sse::{SseMessage, SseReader};

fn messages(text: &str) -> Vec<SseMessage> {
    let mut reader = SseReader::new(text.as_bytes());
    let mut messages = Vec::new();
    while let Some(message) = reader.next_message().unwrap() {
        messages.push(message);
    }
    messages
}

fn message(event: Option<&str>, data: &str) -> SseMessage {
    SseMessage {
        event: event.map(str::to_string),
        data: data.to_string(),
    }
}

#[test]
fn reads_one_message_for_each_blank_line() {
    let text = "event: a\ndata: {\"x\":1}\n\nevent: b\ndata: two\n\n";
    assert_eq!(
        messages(text),
        [message(Some("a"), "{\"x\":1}"), message(Some("b"), "two")]
    );
}

#[test]
fn joins_data_lines_with_a_new_line() {
    assert_eq!(
        messages("data: one\ndata: two\ndata\n\n"),
        [message(None, "one\ntwo\n")]
    );
}

#[test]
fn skips_comments_and_messages_without_data() {
    let text = ": heartbeat\n\nevent: ping\n\n:\ndata: x\n\n";
    assert_eq!(messages(text), [message(None, "x")]);
}

#[test]
fn reads_carriage_returns_and_values_without_a_space() {
    assert_eq!(
        messages("event:a\r\ndata:x\r\nid: 7\r\n\r\n"),
        [message(Some("a"), "x")]
    );
}

#[test]
fn keeps_a_second_space_of_the_value() {
    assert_eq!(messages("data:  x\n\n"), [message(None, " x")]);
}

#[test]
fn gives_the_last_message_without_a_blank_line() {
    assert_eq!(
        messages("data: a\n\ndata: b"),
        [message(None, "a"), message(None, "b")]
    );
}

#[test]
fn an_empty_stream_has_no_messages() {
    assert_eq!(messages(""), []);
    assert_eq!(messages(": only a comment\n\n"), []);
}
