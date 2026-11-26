//! Tests of the event parser in `src/agent_loop_claude/events.rs`.

use serde_json::json;

use atama::agent_loop_claude::events::{IdleStop, Permission, SessionEvent, Usage, parse_event};

fn parse(event: serde_json::Value) -> SessionEvent {
    parse_event(&event).unwrap()
}

#[test]
fn reads_an_agent_message() {
    let event = parse(json!({
        "type": "agent.message",
        "id": "sevt_1",
        "content": [
            {"type": "text", "text": "Hello, "},
            {"type": "redacted"},
            {"type": "text", "text": "world"},
        ],
    }));
    assert_eq!(
        event,
        SessionEvent::AgentMessage {
            id: "sevt_1".into(),
            text: "Hello, world".into()
        }
    );
    assert_eq!(event.id(), "sevt_1");
    assert_eq!(
        parse(json!({"type": "agent.message", "id": "sevt_2"})),
        SessionEvent::AgentMessage {
            id: "sevt_2".into(),
            text: String::new()
        }
    );
}

#[test]
fn reads_the_permission_of_a_native_tool_call() {
    let tool_use = |permission: Option<&str>| {
        let mut event = json!({
            "type": "agent.tool_use",
            "id": "sevt_3",
            "name": "bash",
            "input": {"command": "ls"},
        });
        if let Some(permission) = permission {
            event["evaluated_permission"] = json!(permission);
        }
        match parse(event) {
            SessionEvent::AgentToolUse {
                id,
                name,
                input,
                permission,
            } => {
                assert_eq!((id.as_str(), name.as_str()), ("sevt_3", "bash"));
                assert_eq!(input, json!({"command": "ls"}));
                permission
            }
            other => panic!("not a tool use: {other:?}"),
        }
    };
    assert_eq!(tool_use(None), Permission::Allow);
    assert_eq!(tool_use(Some("allow")), Permission::Allow);
    assert_eq!(tool_use(Some("ask")), Permission::Ask);
    assert_eq!(tool_use(Some("deny")), Permission::Deny);
    assert_eq!(tool_use(Some("maybe")), Permission::Other("maybe".into()));
}

#[test]
fn reads_tool_results_and_custom_tool_calls() {
    assert_eq!(
        parse(json!({"type": "agent.tool_result", "id": "sevt_4", "is_error": true})),
        SessionEvent::AgentToolResult {
            id: "sevt_4".into(),
            is_error: true
        }
    );
    assert_eq!(
        parse(json!({"type": "agent.tool_result", "id": "sevt_5"})),
        SessionEvent::AgentToolResult {
            id: "sevt_5".into(),
            is_error: false
        }
    );
    assert_eq!(
        parse(json!({
            "type": "agent.custom_tool_use",
            "id": "sevt_6",
            "name": "local_bash",
            "input": {"command": "pwd"},
        })),
        SessionEvent::CustomToolUse {
            id: "sevt_6".into(),
            name: "local_bash".into(),
            input: json!({"command": "pwd"})
        }
    );
    assert_eq!(
        parse(json!({"type": "agent.custom_tool_use", "id": "sevt_7"})),
        SessionEvent::CustomToolUse {
            id: "sevt_7".into(),
            name: String::new(),
            input: serde_json::Value::Null
        }
    );
}

#[test]
fn reads_the_status_events() {
    assert_eq!(
        parse(json!({"type": "session.status_running", "id": "a"})),
        SessionEvent::StatusRunning { id: "a".into() }
    );
    assert_eq!(
        parse(json!({"type": "session.status_rescheduled", "id": "b"})),
        SessionEvent::StatusRescheduled { id: "b".into() }
    );
    assert_eq!(
        parse(json!({"type": "session.status_terminated", "id": "c"})),
        SessionEvent::StatusTerminated { id: "c".into() }
    );
}

#[test]
fn reads_the_stop_reason_of_an_idle_session() {
    let stop = |stop_reason: serde_json::Value| match parse(json!({
        "type": "session.status_idle",
        "id": "sevt_8",
        "stop_reason": stop_reason,
    })) {
        SessionEvent::StatusIdle { id, stop } => {
            assert_eq!(id, "sevt_8");
            stop
        }
        other => panic!("not an idle event: {other:?}"),
    };
    assert_eq!(stop(json!({"type": "end_turn"})), IdleStop::EndTurn);
    assert_eq!(
        stop(json!({"type": "requires_action", "event_ids": ["sevt_1", 2, "sevt_3"]})),
        IdleStop::RequiresAction {
            event_ids: vec!["sevt_1".into(), "sevt_3".into()]
        }
    );
    assert_eq!(
        stop(json!({"type": "requires_action"})),
        IdleStop::RequiresAction { event_ids: vec![] }
    );
    assert_eq!(
        stop(json!({"type": "retries_exhausted"})),
        IdleStop::RetriesExhausted
    );
    assert_eq!(
        stop(json!({"type": "budget_reached"})),
        IdleStop::BudgetReached
    );
    assert_eq!(stop(json!({"type": "new"})), IdleStop::Other("new".into()));
    assert_eq!(stop(json!(null)), IdleStop::Other(String::new()));
}

#[test]
fn reads_the_usage_of_a_session() {
    let event = parse(json!({
        "type": "session.usage",
        "id": "sevt_9",
        "usage": {
            "input_tokens": 5000,
            "output_tokens": 3200,
            "cache_read_input_tokens": 20000,
            "cache_creation": {
                "ephemeral_5m_input_tokens": 2000,
                "ephemeral_1h_input_tokens": 1,
            },
            "list_cost": {"amount": "187", "currency": "USD"},
            "active_seconds": 342.5,
        },
    }));
    let expected = Usage {
        input_tokens: 5000,
        output_tokens: 3200,
        cache_read_input_tokens: 20000,
        cache_creation_input_tokens: 2001,
        list_cost_cents: Some(187),
    };
    assert_eq!(
        event,
        SessionEvent::Usage {
            id: "sevt_9".into(),
            usage: expected.clone()
        }
    );
    assert_eq!(expected.total_tokens(), 30201);
    assert_eq!(Usage::from_json(&json!({})), Usage::default());
    assert_eq!(
        Usage::from_json(&json!({"cache_creation": 5, "list_cost": {"amount": "x"}})),
        Usage::default()
    );
}

#[test]
fn reads_the_message_of_an_error() {
    assert_eq!(
        parse(json!({
            "type": "session.error",
            "id": "e1",
            "error": {
                "type": "model_error",
                "message": "the model failed",
                "retry_status": {"type": "retrying"},
            },
        })),
        SessionEvent::Error {
            id: "e1".into(),
            message: "the model failed".into(),
            retry_status: Some("retrying".into())
        }
    );
    assert_eq!(
        parse(json!({
            "type": "session.error",
            "id": "e2",
            "error": {"type": "mcp_error", "retry_status": "exhausted"},
        })),
        SessionEvent::Error {
            id: "e2".into(),
            message: "mcp_error".into(),
            retry_status: Some("exhausted".into())
        }
    );
    assert_eq!(
        parse(json!({"type": "session.error", "id": "e3", "error": {"code": 1}})),
        SessionEvent::Error {
            id: "e3".into(),
            message: "{\"code\":1}".into(),
            retry_status: None
        }
    );
}

#[test]
fn keeps_other_types_and_events_without_an_id() {
    assert_eq!(
        parse(json!({"type": "user.message", "id": "sevt_10"})),
        SessionEvent::Other {
            id: "sevt_10".into(),
            kind: "user.message".into()
        }
    );
    let interrupt = parse(json!({"type": "user.interrupt"}));
    assert_eq!(interrupt.id(), "");
}

#[test]
fn fails_on_an_event_without_a_type() {
    assert_eq!(
        parse_event(&json!({"id": "x"})).unwrap_err(),
        "event without a type: {\"id\":\"x\"}"
    );
}

#[test]
fn each_event_has_its_id() {
    let events = [
        json!({"type": "agent.message"}),
        json!({"type": "agent.tool_use"}),
        json!({"type": "agent.tool_result"}),
        json!({"type": "agent.custom_tool_use"}),
        json!({"type": "session.status_running"}),
        json!({"type": "session.status_idle"}),
        json!({"type": "session.status_rescheduled"}),
        json!({"type": "session.status_terminated"}),
        json!({"type": "session.usage"}),
        json!({"type": "session.error"}),
        json!({"type": "span.model_request_end"}),
    ];
    for (index, mut event) in events.into_iter().enumerate() {
        event["id"] = json!(format!("sevt_{index}"));
        assert_eq!(parse(event).id(), format!("sevt_{index}"));
    }
}
