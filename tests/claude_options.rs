//! Tests of the options of a `claude-agent` action in
//! `src/agent_loop_claude/options.rs`.

use std::time::Duration;

use atama::agent_loop_claude::definition::{AgentDefinition, EnvironmentDefinition, Networking};
use atama::agent_loop_claude::native_tools::{
    DomainFilter, NativeTool, NativeToolset, PermissionPolicy,
};
use atama::agent_loop_claude::options::{
    AgentSource, ClaudeAgentConfig, DEFAULT_AUTONOMY, DEFAULT_MAX_TURNS, EnvironmentSource,
    OPTION_KEYS, OnAsk, parse_usd_cents,
};
use atama::profile::ModelProfile;
use atama::tools::Autonomy;

fn profile() -> ModelProfile {
    ModelProfile {
        name: "claude".into(),
        model: "claude-opus-5-5".into(),
        base_url: "https://api.anthropic.com/v1".into(),
        api_key_env: Some("CLAUDE_KEY".into()),
        token_budget: Some(50_000),
        thinking: None,
    }
}

fn research() -> EnvironmentDefinition {
    EnvironmentDefinition {
        name: "atama-research".into(),
        networking: Networking::Limited {
            allowed_hosts: vec!["arxiv.org".into()],
            allow_package_managers: false,
            allow_mcp_servers: false,
        },
    }
}

fn env(name: &str) -> Option<String> {
    (name == "CLAUDE_KEY").then(|| "sk-test".to_string())
}

fn params(options: &[&str]) -> Vec<String> {
    let mut params = vec!["model=claude".to_string()];
    params.extend(options.iter().map(|option| option.to_string()));
    params
}

fn config(options: &[&str]) -> Result<ClaudeAgentConfig, String> {
    ClaudeAgentConfig::from_params(
        &params(options),
        Some(&profile()),
        Some(&research()),
        "atama-job-task-1",
        &env,
    )
}

fn error(options: &[&str]) -> String {
    config(options).unwrap_err()
}

#[test]
fn a_minimal_action_gets_the_defaults() {
    let config = config(&[]).unwrap();
    assert_eq!(
        config,
        ClaudeAgentConfig {
            profile: "claude".into(),
            base_url: "https://api.anthropic.com/v1".into(),
            api_key: "sk-test".into(),
            task: String::new(),
            agent: AgentSource::Managed(AgentDefinition {
                name: "atama-job-task-1".into(),
                model: "claude-opus-5-5".into(),
                effort: None,
                system: None,
                native: NativeToolset {
                    enabled: vec![],
                    permission: PermissionPolicy::Auto,
                    web_domains: DomainFilter::None,
                },
                local_tools: vec![],
            }),
            environment: EnvironmentSource::Managed(EnvironmentDefinition::default_environment()),
            local_tools: vec![],
            autonomy: DEFAULT_AUTONOMY,
            on_ask: OnAsk::Prompt,
            protect: vec![],
            max_time: Duration::from_secs(30 * 60),
            max_cost_cents: None,
            token_budget: Some(50_000),
            max_turns: DEFAULT_MAX_TURNS,
            output_schema: None,
        }
    );
    assert_eq!(DEFAULT_AUTONOMY, Autonomy::AskIfRisky);
    assert_eq!(DEFAULT_MAX_TURNS, 3);
}

#[test]
fn reads_every_option() {
    let config = config(&[
        "task=Find papers.",
        "system=Be brief.",
        "name=papers",
        "native_tools=web_search, bash",
        "autonomy=always_ask",
        "on_ask=prompt",
        "allowed_domains=arxiv.org",
        "local_tools=write_file",
        "protect=./jobs",
        "environment=research",
        "max_time=20m",
        "max_cost_usd=2.5",
        "token_budget=100_000",
        "max_turns=4",
        "output_schema=papers",
    ])
    .unwrap();
    assert_eq!(config.task, "Find papers.");
    assert_eq!(
        config.agent,
        AgentSource::Managed(AgentDefinition {
            name: "papers".into(),
            model: "claude-opus-5-5".into(),
            effort: None,
            system: Some("Be brief.".into()),
            native: NativeToolset {
                enabled: vec![NativeTool::WebSearch, NativeTool::Bash],
                permission: PermissionPolicy::AlwaysAsk,
                web_domains: DomainFilter::Allowed(vec!["arxiv.org".into()]),
            },
            local_tools: vec!["write_file".into()],
        })
    );
    assert_eq!(config.environment, EnvironmentSource::Managed(research()));
    assert_eq!(config.local_tools, ["write_file"]);
    assert_eq!(config.autonomy, Autonomy::AlwaysAsk);
    assert_eq!(config.protect, ["./jobs"]);
    assert_eq!(config.max_time, Duration::from_secs(20 * 60));
    assert_eq!(config.max_cost_cents, Some(250));
    assert_eq!(config.token_budget, Some(100_000));
    assert_eq!(config.max_turns, 4);
    assert_eq!(config.output_schema.as_deref(), Some("papers"));
    assert_eq!(OPTION_KEYS.len(), 19);
}

#[test]
fn uses_an_agent_and_an_environment_that_exist() {
    let config = config(&[
        "agent_id=agent_1",
        "environment_id=env_1",
        "local_tools=bash",
        "autonomy=no_ask",
        "on_ask=deny",
    ])
    .unwrap();
    assert_eq!(config.agent, AgentSource::Existing("agent_1".into()));
    assert_eq!(
        config.environment,
        EnvironmentSource::Existing("env_1".into())
    );
    assert_eq!(config.on_ask, OnAsk::Deny);
    assert_eq!(
        error(&["agent_id=agent_1", "name=x", "native_tools=bash"]),
        "`agent_id=` uses an agent that exists, so it cannot be with `name=`, `native_tools=`"
    );
}

#[test]
fn rejects_invalid_options() {
    assert_eq!(
        error(&["Fix it"]),
        "unexpected `Fix it`, put the task in `task=\"...\"`"
    );
    assert_eq!(
        error(&["max_steps=3"]),
        format!(
            "unknown option `max_steps` of `agent:claude-agent`, expected {}",
            OPTION_KEYS.map(|key| format!("`{key}`")).join(", ")
        )
    );
    assert_eq!(error(&["task="]), "option `task` has no value");
    assert_eq!(error(&["task=a", "task=b"]), "option `task` is set twice");
    assert_eq!(
        error(&["autonomy=often"]),
        "`autonomy=often` is not `no_ask`, `ask_if_risky` or `always_ask`"
    );
    assert_eq!(
        error(&["on_ask=judge"]),
        "`on_ask=judge` is not available yet, use `prompt` or `deny`"
    );
    assert_eq!(
        error(&["on_ask=maybe"]),
        "`on_ask=maybe` is not `prompt` or `deny`"
    );
    assert_eq!(
        error(&["max_time=soon"]),
        "`max_time=soon` is not a duration, expected a positive number and a unit as in \
         `90s`, `20m` or `1h`"
    );
    assert_eq!(
        error(&["max_turns=0"]),
        "`max_turns=0` is not a positive number"
    );
    assert_eq!(
        error(&["token_budget=many"]),
        "`token_budget=many` is not a positive number"
    );
    assert_eq!(
        error(&["native_tools=read_file"]),
        "`read_file` is not a native tool, expected `all` or `bash`, `read`, `write`, `edit`, \
         `glob`, `grep`, `web_fetch`, `web_search`"
    );
    assert_eq!(
        error(&["local_tools=web"]),
        "`web` is not a local tool, expected `bash`, `read_file`, `write_file`, `list_files`"
    );
    assert!(error(&["local_tools=bash", "protect=a\"b"]).contains("protect"));
}

#[test]
fn enforces_the_rules_between_options() {
    assert_eq!(
        error(&["autonomy=always_ask", "on_ask=deny"]),
        "`autonomy=always_ask` and `on_ask=deny` cannot be together, because Atama then \
         denies each tool call"
    );
    assert_eq!(
        error(&["protect=./jobs"]),
        "`protect=` needs `local_tools=`, because it protects the files of the local tools \
         only"
    );
    assert_eq!(
        error(&["environment=research", "environment_id=env_1"]),
        "`environment=` and `environment_id=` cannot be together, use one of them"
    );
    assert_eq!(
        error(&["environment=other"]),
        "claude-env block `other` is not defined in the job"
    );
    assert_eq!(
        error(&["allowed_domains=a.com"]),
        "`allowed_domains=` and `blocked_domains=` need `web_search` or `web_fetch` in \
         `native_tools=`"
    );
    assert_eq!(
        error(&[
            "native_tools=web_fetch",
            "allowed_domains=a.com",
            "blocked_domains=b.com"
        ]),
        "`allowed_domains=` and `blocked_domains=` cannot be together, use one of them"
    );
    assert_eq!(
        error(&["native_tools=web_search"]),
        "the web tools return nothing in environment `atama-default`, because it has \
         `limited` networking with no `allowed_hosts`. Name a ```claude-env:<name>``` block \
         with `allowed_hosts` in `environment=`"
    );
    // The web tools work in an environment with hosts, or one that exists.
    config(&["native_tools=web_search", "environment=research"]).unwrap();
    config(&["native_tools=web_search", "environment_id=env_1"]).unwrap();
}

#[test]
fn needs_a_profile_that_a_claude_agent_can_use() {
    assert_eq!(
        ClaudeAgentConfig::from_params(&[], Some(&profile()), None, "n", &env).unwrap_err(),
        "`agent:claude-agent` needs `model=<profile>`"
    );
    assert_eq!(
        ClaudeAgentConfig::from_params(&params(&[]), None, None, "n", &env).unwrap_err(),
        "model profile `claude` is not defined in the job"
    );
    let thinking = ModelProfile {
        thinking: Some(false),
        ..profile()
    };
    assert_eq!(
        ClaudeAgentConfig::from_params(&params(&[]), Some(&thinking), None, "n", &env).unwrap_err(),
        "model profile `claude` sets `thinking=`, but a claude-agent action cannot turn off \
         thinking, remove `thinking=` from the profile"
    );
    let no_key = ModelProfile {
        api_key_env: None,
        ..profile()
    };
    assert_eq!(
        ClaudeAgentConfig::from_params(&params(&[]), Some(&no_key), None, "n", &env).unwrap_err(),
        "model profile `claude` needs `api_key=<ENV_VAR>` for a claude-agent action"
    );
    let no_env = |_: &str| None;
    assert_eq!(
        ClaudeAgentConfig::from_params(&params(&[]), Some(&profile()), None, "n", &no_env)
            .unwrap_err(),
        "model profile `claude` needs the API key in `CLAUDE_KEY`, but `CLAUDE_KEY` is not set"
    );
}

#[test]
fn check_does_not_read_the_api_key() {
    ClaudeAgentConfig::check(&params(&["task=x"]), Some(&profile()), None).unwrap();
    assert_eq!(
        ClaudeAgentConfig::check(&params(&["task="]), Some(&profile()), None).unwrap_err(),
        "option `task` has no value"
    );
}

#[test]
fn reads_an_amount_of_dollars_as_cents() {
    for (value, cents) in [
        ("15", 1500),
        ("2.5", 250),
        ("2.00", 200),
        ("0.01", 1),
        ("007", 700),
    ] {
        assert_eq!(parse_usd_cents(value).unwrap(), cents, "{value}");
    }
    for bad in [
        "0",
        "0.00",
        "",
        ".5",
        "2.",
        "2.555",
        "-1",
        "+1",
        "1e3",
        "2,50",
        " 2",
        "1.2.3",
        "99999999999999999999",
    ] {
        assert_eq!(
            parse_usd_cents(bad).unwrap_err(),
            format!(
                "`max_cost_usd={bad}` is not an amount of dollars, use a number more than 0 \
                 with two decimal places at most, as in `15`, `2.5` or `2.00`"
            ),
            "{bad}"
        );
    }
}
