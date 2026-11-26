//! Tests of the native tools in `src/agent_loop_claude/native_tools.rs`.

use serde_json::json;

use atama::agent_loop_claude::native_tools::{
    DomainFilter, MAX_DOMAINS, NativeTool, NativeToolset, PermissionPolicy, TOOLSET_TYPE,
    parse_domains,
};
use atama::tools::Autonomy;

#[test]
fn each_tool_has_its_name_in_the_api() {
    let names: Vec<&str> = NativeTool::ALL.iter().map(|tool| tool.name()).collect();
    assert_eq!(
        names,
        [
            "bash",
            "read",
            "write",
            "edit",
            "glob",
            "grep",
            "web_fetch",
            "web_search"
        ]
    );
    for tool in NativeTool::ALL {
        assert_eq!(NativeTool::from_name(tool.name()), Some(tool));
        assert_eq!(
            tool.is_web(),
            matches!(tool, NativeTool::WebFetch | NativeTool::WebSearch)
        );
    }
    assert_eq!(NativeTool::from_name("read_file"), None);
}

#[test]
fn reads_a_list_of_native_tools() {
    assert_eq!(NativeTool::parse_list("all").unwrap(), NativeTool::ALL);
    assert_eq!(
        NativeTool::parse_list("web_search, web_fetch,\n- bash").unwrap(),
        [
            NativeTool::WebSearch,
            NativeTool::WebFetch,
            NativeTool::Bash
        ]
    );
    assert_eq!(
        NativeTool::parse_list("bash, read_file").unwrap_err(),
        "`read_file` is not a native tool, expected `all` or `bash`, `read`, `write`, `edit`, \
         `glob`, `grep`, `web_fetch`, `web_search`"
    );
    assert_eq!(
        NativeTool::parse_list("bash, bash").unwrap_err(),
        "native tool `bash` is in `native_tools=` twice"
    );
    assert_eq!(
        NativeTool::parse_list(" , ").unwrap_err(),
        "`native_tools=` has no tool"
    );
}

#[test]
fn each_level_of_autonomy_sends_a_policy() {
    for (autonomy, policy, name) in [
        (
            Autonomy::NoAsk,
            PermissionPolicy::AlwaysAllow,
            "always_allow",
        ),
        (Autonomy::AskIfRisky, PermissionPolicy::Auto, "auto"),
        (
            Autonomy::AlwaysAsk,
            PermissionPolicy::AlwaysAsk,
            "always_ask",
        ),
    ] {
        assert_eq!(PermissionPolicy::from(autonomy), policy);
        assert_eq!(policy.name(), name);
    }
}

#[test]
fn reads_the_domain_lists() {
    assert_eq!(
        DomainFilter::from_values(None, None).unwrap(),
        DomainFilter::None
    );
    assert_eq!(DomainFilter::default(), DomainFilter::None);
    assert_eq!(
        DomainFilter::from_values(Some("arXiv.org, docs.rs"), None).unwrap(),
        DomainFilter::Allowed(vec!["arxiv.org".into(), "docs.rs".into()])
    );
    assert_eq!(
        DomainFilter::from_values(None, Some("ads.example.com")).unwrap(),
        DomainFilter::Blocked(vec!["ads.example.com".into()])
    );
    assert_eq!(
        DomainFilter::from_values(Some("a.com"), Some("b.com")).unwrap_err(),
        "`allowed_domains=` and `blocked_domains=` cannot be together, use one of them"
    );
    assert!(
        DomainFilter::from_values(Some("x"), None)
            .unwrap_err()
            .starts_with("`x` in `allowed_domains=` is not a host name")
    );
    assert_eq!(
        DomainFilter::from_values(None, Some("x")).unwrap_err(),
        "`x` in `blocked_domains=` is not a host name, use a plain name as in \
         `docs.example.com`, without a scheme, a port, a path or `*`"
    );
}

#[test]
fn a_domain_is_a_plain_host_name() {
    for bad in [
        "https://a.com",
        "a.com:443",
        "*.a.com",
        "a.com/path",
        "localhost",
        ".a.com",
        "a.com.",
    ] {
        let error = parse_domains("allowed_domains", bad).unwrap_err();
        assert!(error.contains("is not a host name"), "{bad}: {error}");
    }
    let long = format!("{}.com", "a".repeat(252));
    assert!(parse_domains("allowed_domains", &long).is_err());
    assert_eq!(
        parse_domains("allowed_domains", "a.com, A.com").unwrap_err(),
        "`a.com` is in `allowed_domains=` twice"
    );
    assert_eq!(
        parse_domains("allowed_domains", "").unwrap_err(),
        "`allowed_domains=` has no domain"
    );
    let many: Vec<String> = (0..=MAX_DOMAINS).map(|n| format!("d{n}.com")).collect();
    assert_eq!(
        parse_domains("allowed_domains", &many.join(",")).unwrap_err(),
        format!("`allowed_domains=` has 65 domains, the most is {MAX_DOMAINS}")
    );
}

#[test]
fn a_toolset_without_tools_has_no_json() {
    let toolset = NativeToolset {
        enabled: vec![],
        permission: PermissionPolicy::Auto,
        web_domains: DomainFilter::None,
    };
    assert_eq!(toolset.to_json(), None);
    assert!(!toolset.has_web());
}

#[test]
fn the_toolset_turns_on_only_the_named_tools() {
    let toolset = NativeToolset {
        enabled: vec![
            NativeTool::Bash,
            NativeTool::WebSearch,
            NativeTool::WebFetch,
        ],
        permission: PermissionPolicy::AlwaysAsk,
        web_domains: DomainFilter::Allowed(vec!["arxiv.org".into()]),
    };
    assert!(toolset.has_web());
    assert_eq!(
        toolset.to_json().unwrap(),
        json!({
            "type": TOOLSET_TYPE,
            "default_config": {"enabled": false, "permission_policy": {"type": "always_ask"}},
            "configs": [
                {"name": "bash", "enabled": true},
                {"name": "web_search", "enabled": true, "allowed_domains": ["arxiv.org"]},
                {"name": "web_fetch", "enabled": true, "allowed_domains": ["arxiv.org"]},
            ],
        })
    );
    let blocked = NativeToolset {
        enabled: vec![NativeTool::WebFetch],
        permission: PermissionPolicy::Auto,
        web_domains: DomainFilter::Blocked(vec!["ads.example.com".into()]),
    };
    assert_eq!(
        blocked.to_json().unwrap()["configs"],
        json!([{"name": "web_fetch", "enabled": true, "blocked_domains": ["ads.example.com"]}])
    );
    assert_eq!(TOOLSET_TYPE, "agent_toolset_20260401");
}
