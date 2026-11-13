//! Tests for loop blocks, in `src/loop_block.rs`.

use serde_json::json;

use atama::agent_loop_openai_compat::tools::{DEFAULT_MAX_CALLS, DEFAULT_MAX_FAILURES};
use atama::loop_block::{
    LoopBlock, LoopBlocks, ToolLimits, block_name, check_name, loop_name, parse_block, resolve,
};

fn params(params: &[&str]) -> Vec<String> {
    params.iter().map(|param| param.to_string()).collect()
}

fn parse(lines: &[&str]) -> Result<LoopBlock, (usize, String)> {
    parse_block("fix", lines)
}

#[test]
fn reads_the_name_of_a_loop_fence() {
    assert_eq!(block_name("```loop:fix-tests"), Some("fix-tests"));
    assert_eq!(block_name("  ```loop: fix "), Some("fix"));
    assert_eq!(block_name("```json:fix"), None);
    assert_eq!(check_name("fix-tests_2"), Ok(()));
    assert_eq!(
        check_name("a b"),
        Err(
            "`a b` is not a loop block name, use 1 to 64 letters, digits, `_` or `-`, as in \
             ```loop:fix-tests"
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
        "guardrails:",
        "  - Do not change files in tests/.",
        "  - Do not run: git push.",
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
            "guardrails=- Do not change files in tests/.\n- Do not run: git push.",
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
            "the item `- item` needs a key with no value before it, as in `guardrails:`",
        ),
        (
            &["success: x", "- item"],
            1,
            "the item `- item` needs a key with no value before it, as in `guardrails:`",
        ),
        (
            &["success"],
            0,
            "expected `<key>: <value>`, found `success`",
        ),
        (
            &["model: p"],
            0,
            "unknown key `model` in loop block `fix`, expected `task`, `max_steps`, \
             `token_budget`, `max_context`, `thinking`, `max_iterations`, `max_time`, `success_check`, \
             `success`, `guardrails`, `guard`, `max_violations`, `judge_model` or `tools`",
        ),
        (
            &["success: a", "success: b"],
            1,
            "key `success` is set twice",
        ),
        (
            &["guardrails:"],
            0,
            "key `guardrails` has no value and no items",
        ),
        (&["tools:"], 0, "key `tools` has no value and no items"),
        (
            &["guardrails: x", "", "success:", "- y", "guardrails:"],
            4,
            "key `guardrails` is set twice",
        ),
        (
            &["tools:", "- rm"],
            0,
            "unknown tool `rm`, expected `bash`, `read_file`, `write_file`, `list_files`",
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
        block.merge(&params(&["model=p", "max_time=5m", "loop=fix"])),
        params(&[
            "success=done",
            "max_iterations=3",
            "model=p",
            "max_time=5m",
            "loop=fix"
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
}

#[test]
fn finds_the_block_that_loop_names() {
    let mut blocks = LoopBlocks::new();
    let block = parse(&["success: x"]).unwrap();
    blocks.insert("fix".into(), block.clone());
    assert_eq!(loop_name(&params(&["model=p", "loop=fix"])), Some("fix"));
    assert_eq!(loop_name(&params(&["model=p"])), None);
    assert_eq!(resolve(&params(&["model=p"]), &blocks), Ok(None));
    assert_eq!(resolve(&params(&["loop=fix"]), &blocks), Ok(Some(block)));
    assert_eq!(
        resolve(&params(&["loop=other"]), &blocks),
        Err("loop block `other` is not defined in the job".to_string())
    );
}
