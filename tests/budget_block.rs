//! Tests for budget blocks, in `src/budget_block.rs`.

use serde_json::json;

use atama::agent_loop_openai_compat::tools::{DEFAULT_MAX_CALLS, DEFAULT_MAX_FAILURES};
use atama::budget_block::{
    BudgetBlock, BudgetBlocks, ToolLimits, block_name, budget_name, check_name, parse_block,
    resolve,
};

fn params(params: &[&str]) -> Vec<String> {
    params.iter().map(|param| param.to_string()).collect()
}

fn parse(lines: &[&str]) -> Result<BudgetBlock, (usize, String)> {
    parse_block("fix", lines)
}

#[test]
fn rejects_the_options_of_one_action() {
    for key in ["task", "protect", "guardrails"] {
        let line = format!("{key}: x");
        assert_eq!(
            parse(&["max_time: 20m", &line]),
            Err((
                1,
                format!(
                    "`{key}` cannot be in budget block `fix`, set `{key}=` in the options of \
                     the action"
                )
            ))
        );
        let line = format!("{key}:");
        assert!(parse(&[&line, "- y"]).is_err(), "{key}");
    }
}

#[test]
fn reads_the_name_of_a_loop_fence() {
    assert_eq!(block_name("```budget:fix-tests"), Some("fix-tests"));
    assert_eq!(block_name("  ```budget: fix "), Some("fix"));
    assert_eq!(block_name("```json:fix"), None);
    assert_eq!(check_name("fix-tests_2"), Ok(()));
    assert_eq!(
        check_name("a b"),
        Err(
            "`a b` is not a budget block name, use 1 to 64 letters, digits, `_` or `-`, as in \
             ```budget:fix-tests"
                .to_string()
        )
    );
}

#[test]
fn reads_options_lists_and_tools() {
    let block = parse(&[
        "max_time: 20m",
        "",
        "success_check: cargo test --quiet",
        "success:",
        "  - All tests pass.",
        "  - No test is ignored: none.",
        "tools:",
        "  - read_file",
        "  - bash max_calls=40 max_failures=5",
    ])
    .unwrap();
    assert_eq!(block.name, "fix");
    assert_eq!(
        block.params,
        params(&[
            "max_time=20m",
            "success_check=cargo test --quiet",
            "success=- All tests pass.\n- No test is ignored: none.",
        ])
    );
    assert_eq!(
        block.tools,
        Some(vec![
            ToolLimits {
                name: "read_file".into(),
                max_calls: DEFAULT_MAX_CALLS,
                max_failures: DEFAULT_MAX_FAILURES,
            },
            ToolLimits {
                name: "bash".into(),
                max_calls: 40,
                max_failures: 5,
            },
        ])
    );
    assert_eq!(parse(&["success: done"]).unwrap().tools, None);
    assert_eq!(
        parse(&["max_context: 8000"]).unwrap().params,
        params(&["max_context=8000"])
    );
}

#[test]
fn rejects_invalid_lines() {
    let cases: [(&[&str], usize, &str); 11] = [
        (
            &["- item"],
            0,
            "the item `- item` needs a key with no value before it, as in `tools:`",
        ),
        (
            &["success: x", "- item"],
            1,
            "the item `- item` needs a key with no value before it, as in `tools:`",
        ),
        (
            &["success"],
            0,
            "expected `<key>: <value>`, found `success`",
        ),
        (
            &["model: p"],
            0,
            "unknown key `model` in budget block `fix`, expected `max_steps`, `token_budget`, \
             `max_context`, `thinking`, `max_iterations`, `max_time`, `success_check`, `success`, \
             `toolGuard`, `max_violations`, `judge_model` or `tools`",
        ),
        (
            &["success: a", "success: b"],
            1,
            "key `success` is set twice",
        ),
        (&["success:"], 0, "key `success` has no value and no items"),
        (&["tools:"], 0, "key `tools` has no value and no items"),
        (
            &["max_time: 1m", "", "success:", "- y", "max_time:"],
            4,
            "key `max_time` is set twice",
        ),
        (
            &["tools:", "- rm"],
            0,
            "unknown tool `rm`, expected `bash`, `read_file`, `write_file`, `list_files`, `web_search`",
        ),
        (
            &["tools:", "- bash timeout=3"],
            0,
            "unknown limit `timeout=3` of tool `bash`, expected `max_calls=<n>` or \
             `max_failures=<n>`",
        ),
        (
            &["tools:", "- bash max_calls=0"],
            0,
            "`max_calls=0` of tool `bash` is not a positive number",
        ),
    ];
    for (lines, index, message) in cases {
        assert_eq!(parse(lines), Err((index, message.to_string())), "{lines:?}");
    }
}

#[test]
fn an_option_of_the_action_replaces_the_option_of_the_block() {
    let block = parse(&["max_time: 20m", "success: done", "max_iterations: 3"]).unwrap();
    assert_eq!(
        block.merge(&params(&["model=p", "max_time=5m", "budget=fix"])),
        params(&[
            "success=done",
            "max_iterations=3",
            "model=p",
            "max_time=5m",
            "budget=fix"
        ])
    );
    assert_eq!(
        block.merge(&params(&["fix.md"])),
        params(&["max_time=20m", "success=done", "max_iterations=3", "fix.md"])
    );
}

#[test]
fn makes_the_tools_of_the_block_with_their_limits() {
    let block = parse(&["success: x"]).unwrap();
    assert!(block.make_tools().is_none());

    let block = parse(&["tools:", "- read_file max_calls=1", "- list_files"]).unwrap();
    let mut tools = block.make_tools().unwrap();
    let names: Vec<&str> = tools.iter().map(|tool| tool.name()).collect();
    assert_eq!(names, ["read_file", "list_files"]);
    assert!(tools[0].available());
    let _ = tools[0].call(&json!({"path": "Cargo.toml"}));
    assert!(!tools[0].available());

    let block = parse(&["tools:", "- web_search max_calls=3"]).unwrap();
    let tools = block.make_tools().unwrap();
    assert_eq!(tools[0].name(), "web_search");
    assert!(tools[0].read_only());
}

#[test]
fn finds_the_block_that_budget_names() {
    let mut blocks = BudgetBlocks::new();
    let block = parse(&["success: x"]).unwrap();
    blocks.insert("fix".into(), block.clone());
    assert_eq!(
        budget_name(&params(&["model=p", "budget=fix"])),
        Some("fix")
    );
    assert_eq!(budget_name(&params(&["model=p"])), None);
    assert_eq!(resolve(&params(&["model=p"]), &blocks), Ok(None));
    assert_eq!(resolve(&params(&["budget=fix"]), &blocks), Ok(Some(block)));
    assert_eq!(
        resolve(&params(&["budget=other"]), &blocks),
        Err("budget block `other` is not defined in the job".to_string())
    );
}
