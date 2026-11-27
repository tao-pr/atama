//! Tests of the local tools, the definitions and the claude-env blocks of
//! `src/agent_loop_claude/`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use serde_json::json;

use atama::agent_loop_claude::custom_tools::{
    LOCAL_PREFIX, LocalTools, api_name, custom_tool_json, parse_names,
};
use atama::agent_loop_claude::definition::{
    AgentDefinition, DEFAULT_ENVIRONMENT, EnvironmentDefinition, Networking, check_existing_agent,
};
use atama::agent_loop_claude::env_block::{
    ENV_KEYS, ENV_NAME_PREFIX, block_name, check_name, parse_block,
};
use atama::agent_loop_claude::native_tools::{
    DomainFilter, NativeTool, NativeToolset, PermissionPolicy,
};
use atama::protect::Protect;
use atama::tools::{Approver, Autonomy, Bash, DenyApprover, ReadFile, Tool};

fn deny() -> Rc<RefCell<dyn Approver>> {
    Rc::new(RefCell::new(DenyApprover))
}

fn no_native() -> NativeToolset {
    NativeToolset {
        enabled: vec![],
        permission: PermissionPolicy::Auto,
        web_domains: DomainFilter::None,
    }
}

#[test]
fn a_local_tool_has_the_local_prefix() {
    assert_eq!(LOCAL_PREFIX, "local_");
    assert_eq!(api_name("bash"), "local_bash");
    assert_eq!(
        custom_tool_json(&Bash::default()),
        json!({
            "type": "custom",
            "name": "local_bash",
            "description": Bash::default().description(),
            "input_schema": Bash::default().parameters(),
        })
    );
}

#[test]
fn reads_the_names_of_the_local_tools() {
    assert_eq!(
        parse_names("bash, read_file").unwrap(),
        ["bash", "read_file"]
    );
    assert_eq!(
        parse_names("bash, web").unwrap_err(),
        "`web` is not a local tool, expected `bash`, `read_file`, `write_file`, `list_files`, `web_search`"
    );
    assert_eq!(
        parse_names("bash,bash").unwrap_err(),
        "local tool `bash` is in `local_tools=` twice"
    );
    assert_eq!(parse_names(",").unwrap_err(), "`local_tools=` has no tool");
}

#[test]
fn local_tools_run_by_their_api_name() {
    let names = vec!["read_file".to_string(), "bash".to_string()];
    let mut tools = LocalTools::from_names(&names, Autonomy::AskIfRisky, deny()).unwrap();
    let definitions = tools.definitions();
    assert_eq!(definitions.len(), 2);
    assert_eq!(definitions[0]["name"], "local_read_file");
    assert_eq!(definitions[1]["name"], "local_bash");
    assert!(tools.has("local_bash"));
    assert!(!tools.has("bash"));
    assert!(!tools.has("local_write_file"));
    tools.set_deadline_time(None);
    tools.set_protect(None::<Arc<Protect>>);

    // `read_file` only reads, so it runs without approval.
    let error = tools
        .call("local_read_file", &json!({"path": "/no/such/file"}))
        .unwrap_err();
    assert!(!error.contains("denied"), "{error}");
    // `bash` can change files, so it asks, and `deny` denies it.
    assert_eq!(
        tools
            .call("local_bash", &json!({"command": "echo hi"}))
            .unwrap_err(),
        "the call of `bash` was denied, because `on_ask=deny` denies each call that waits \
         for approval"
    );
    assert_eq!(
        tools.call("bash", &json!({})).unwrap_err(),
        "unknown tool `bash`"
    );
}

#[test]
fn web_search_is_a_local_tool_that_runs_without_approval() {
    let names = parse_names("web_search").unwrap();
    let mut tools = LocalTools::from_names(&names, Autonomy::AskIfRisky, deny()).unwrap();
    assert_eq!(tools.definitions()[0]["name"], "local_web_search");
    // The tool only reads, so `deny` does not stop it. The empty query
    // fails before a search.
    assert_eq!(
        tools
            .call("local_web_search", &json!({"query": ""}))
            .unwrap_err(),
        "the argument `query` is empty"
    );
}

#[test]
fn local_tools_without_approval_run() {
    let mut tools = LocalTools::from_names(&["bash".to_string()], Autonomy::NoAsk, deny()).unwrap();
    assert_eq!(
        tools
            .call("local_bash", &json!({"command": "echo hi"}))
            .unwrap()
            .trim(),
        "hi"
    );
    let mut mock = LocalTools::with_tools(vec![Box::new(ReadFile)]);
    assert!(mock.has("local_read_file"));
    assert!(mock.call("local_read_file", &json!({})).is_err());
    assert_eq!(
        LocalTools::from_names(&["web".to_string()], Autonomy::NoAsk, deny())
            .err()
            .unwrap(),
        "`web` is not a local tool"
    );
}

#[test]
fn an_agent_definition_is_the_body_of_a_create() {
    let agent = AgentDefinition {
        name: "atama-job-task-1".into(),
        model: "claude-opus-5-5".into(),
        effort: None,
        system: None,
        native: no_native(),
        local_tools: vec![],
    };
    assert_eq!(
        agent.to_json(),
        json!({"name": "atama-job-task-1", "model": "claude-opus-5-5", "tools": []})
    );

    let agent = AgentDefinition {
        effort: Some("high".into()),
        system: Some("Be brief.".into()),
        native: NativeToolset {
            enabled: vec![NativeTool::WebSearch],
            permission: PermissionPolicy::Auto,
            web_domains: DomainFilter::None,
        },
        local_tools: vec!["write_file".into()],
        ..agent
    };
    let body = agent.to_json();
    assert_eq!(
        body["model"],
        json!({"id": "claude-opus-5-5", "effort": "high"})
    );
    assert_eq!(body["system"], "Be brief.");
    let tools = body["tools"].as_array().unwrap();
    assert_eq!(tools[0]["type"], "agent_toolset_20260401");
    assert_eq!(tools[1]["name"], "local_write_file");
    // The order of the keys is fixed, so the text of two bodies is equal.
    let keys: Vec<&String> = body.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["name", "model", "system", "tools"]);
    assert_eq!(body.to_string(), agent.to_json().to_string());
}

#[test]
fn an_existing_agent_needs_a_local_tool_for_each_custom_tool() {
    let agent = json!({
        "id": "agent_1",
        "tools": [
            {"type": "agent_toolset_20260401"},
            {"type": "custom", "name": "local_bash"},
        ],
    });
    check_existing_agent(&agent, &["bash".into()]).unwrap();
    assert_eq!(
        check_existing_agent(&agent, &[]).unwrap_err(),
        "agent `agent_1` has the custom tool `local_bash`, but no tool of `local_tools=` \
         answers it"
    );
    let other = json!({"tools": [{"type": "custom", "name": "get_weather"}]});
    assert_eq!(
        check_existing_agent(&other, &["bash".into()]).unwrap_err(),
        "agent `the agent` has the custom tool `get_weather`, but no tool of `local_tools=` \
         answers it"
    );
    check_existing_agent(&json!({"id": "agent_2"}), &[]).unwrap();
}

#[test]
fn an_environment_definition_is_the_body_of_a_create() {
    let default = EnvironmentDefinition::default_environment();
    assert_eq!(default.name, DEFAULT_ENVIRONMENT);
    assert!(default.blocks_the_web());
    assert_eq!(
        default.to_json(),
        json!({
            "name": "atama-default",
            "config": {
                "type": "cloud",
                "networking": {
                    "type": "limited",
                    "allowed_hosts": [],
                    "allow_package_managers": false,
                    "allow_mcp_servers": false,
                },
            },
        })
    );
    let open = EnvironmentDefinition {
        name: "atama-open".into(),
        networking: Networking::Unrestricted,
    };
    assert!(!open.blocks_the_web());
    assert_eq!(
        open.to_json()["config"]["networking"],
        json!({"type": "unrestricted"})
    );
}

#[test]
fn finds_the_fence_of_a_block() {
    assert_eq!(block_name("```claude-env:research"), Some("research"));
    assert_eq!(block_name("  ```claude-env: web "), Some("web"));
    assert_eq!(block_name("```budget:research"), None);
    check_name("research-1").unwrap();
    assert_eq!(
        check_name("a b").unwrap_err(),
        "`a b` is not a claude-env block name, use 1 to 64 letters, digits, `_` or `-`, as \
         in ```claude-env:research"
    );
}

#[test]
fn reads_a_block() {
    let block = parse_block(
        "research",
        &[
            "networking: limited",
            "",
            "allowed_hosts: arXiv.org, *.docs.rs",
            "allow_package_managers: true",
            "allow_mcp_servers: false",
        ],
    )
    .unwrap();
    assert_eq!(ENV_NAME_PREFIX, "atama-");
    assert_eq!(
        block,
        EnvironmentDefinition {
            name: "atama-research".into(),
            networking: Networking::Limited {
                allowed_hosts: vec!["arxiv.org".into(), "*.docs.rs".into()],
                allow_package_managers: true,
                allow_mcp_servers: false,
            },
        }
    );
    assert_eq!(
        parse_block("open", &["networking: unrestricted"]).unwrap(),
        EnvironmentDefinition {
            name: "atama-open".into(),
            networking: Networking::Unrestricted,
        }
    );
    assert_eq!(
        parse_block("empty", &[]).unwrap(),
        EnvironmentDefinition {
            name: "atama-empty".into(),
            ..EnvironmentDefinition::default_environment()
        }
    );
}

#[test]
fn rejects_an_invalid_block() {
    let error = |lines: &[&str]| parse_block("b", lines).unwrap_err();
    assert_eq!(
        error(&["networking limited"]),
        (
            0,
            "expected `<key>: <value>`, found `networking limited`".into()
        )
    );
    assert_eq!(
        error(&["networking: open"]),
        (
            0,
            "`networking: open` is not `limited` or `unrestricted`".into()
        )
    );
    assert_eq!(
        error(&["networking: limited", "networking: limited"]),
        (
            1,
            "`networking` is set twice in claude-env block `b`".into()
        )
    );
    assert_eq!(
        error(&["allowed_hosts: a.com", "allowed_hosts: b.com"]),
        (
            1,
            "`allowed_hosts` is set twice in claude-env block `b`".into()
        )
    );
    assert_eq!(
        error(&["allow_mcp_servers: true", "allow_mcp_servers: true"]),
        (
            1,
            "`allow_mcp_servers` is set twice in claude-env block `b`".into()
        )
    );
    assert_eq!(
        error(&["allow_package_managers: yes"]),
        (
            0,
            "`allow_package_managers: yes` is not `true` or `false`".into()
        )
    );
    assert_eq!(
        error(&["tools: bash"]),
        (
            0,
            format!(
                "unknown key `tools` in claude-env block `b`, expected {}",
                ENV_KEYS.map(|key| format!("`{key}`")).join(", ")
            )
        )
    );
    assert_eq!(
        error(&["allowed_hosts: https://a.com"]),
        (
            0,
            "`https://a.com` in `allowed_hosts` is not a host name, use a name as in \
             `api.example.com` or `*.example.com`, without a scheme, a port or a path"
                .into()
        )
    );
    assert_eq!(
        error(&["allowed_hosts: a.com, A.com"]),
        (0, "`a.com` is in `allowed_hosts` twice".into())
    );
    assert_eq!(
        error(&["allowed_hosts: ,"]),
        (0, "`allowed_hosts` has no host".into())
    );
    assert_eq!(
        error(&["allow_package_managers: true", "networking: unrestricted"]),
        (
            0,
            "`allow_package_managers` needs `networking: limited`, not `unrestricted`".into()
        )
    );
    assert_eq!(
        error(&[
            "networking: unrestricted",
            "allow_mcp_servers: false",
            "allowed_hosts: a.com"
        ]),
        (
            1,
            "`allow_mcp_servers` needs `networking: limited`, not `unrestricted`".into()
        )
    );
}
