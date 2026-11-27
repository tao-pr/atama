//! Tests of the parts of the job parser for `agent:claude-agent`: the
//! action, its options, the ```` ```claude-env:<name> ```` blocks, `effort=`
//! on a model profile, and the warnings.

use atama::actor::{Actor, CLAUDE_AGENT};
use atama::agent_loop_claude::definition::{EnvironmentDefinition, Networking};
use atama::profile::{EFFORT_LEVELS, ModelProfile, parse_effort};
use atama::{ParseWarning, parse_job, parse_job_with_warnings};

const CONFIG: &str = "```config\n\
claude: model=claude-opus-5-5, base_url=https://api.anthropic.com/v1, api_key=CLAUDE_KEY, effort=high\n\
local: model=qwen, base_url=http://localhost:11434/v1\n\
```\n";

const ENV: &str = "```claude-env:research\nnetworking: limited\nallowed_hosts: arxiv.org\n```\n";

/// A job with the config block, the claude-env block, and one task whose
/// actions are `actions`.
fn job(actions: &str) -> String {
    format!("{CONFIG}{ENV}# My Job\n## Find Papers\n- actions:\n{actions}")
}

fn first_action(text: &str) -> Actor {
    let job = parse_job(text).expect("job should parse");
    job.tasks[0].actions[0].clone()
}

fn error(text: &str) -> (usize, String) {
    let err = parse_job(text).unwrap_err();
    (err.line, err.message)
}

#[test]
fn reads_a_claude_agent_action_and_its_options() {
    let action = first_action(&job(
        "  - bash echo hi\n  - agent:claude-agent\n    - model=claude\n    - environment=research\n    \
         - native_tools=web_search\n    - task=Find the newest papers.\n",
    ));
    assert!(matches!(action, Actor::BashScript { .. }));
    let job = parse_job(&job(
        "  - bash echo hi\n  - agent:claude-agent\n    - model=claude\n    - environment=research\n    \
         - native_tools=web_search\n    - task=Find the newest papers.\n",
    ))
    .unwrap();
    let Actor::ClaudeAgent {
        params,
        profile,
        output_schema,
        environment,
        default_name,
    } = &job.tasks[0].actions[1]
    else {
        panic!("expected a claude agent, got {:?}", job.tasks[0].actions[1]);
    };
    assert_eq!(
        params,
        &[
            "model=claude",
            "environment=research",
            "native_tools=web_search",
            "task=Find the newest papers.",
        ]
    );
    assert_eq!(
        profile
            .as_ref()
            .map(|p| (p.name.as_str(), p.effort.as_deref())),
        Some(("claude", Some("high")))
    );
    assert!(output_schema.is_none());
    assert_eq!(
        environment,
        &Some(EnvironmentDefinition {
            name: "atama-research".into(),
            networking: Networking::Limited {
                allowed_hosts: vec!["arxiv.org".into()],
                allow_package_managers: false,
                allow_mcp_servers: false,
            },
        })
    );
    // The default name has the job, the task and the number of the action.
    assert_eq!(default_name, "atama-my-job-find-papers-2");
    assert_eq!(
        job.tasks[0].actions[1].to_string(),
        "agent:claude-agent model=claude environment=research native_tools=web_search \
         task=\"Find the newest papers.\""
    );
}

#[test]
fn a_foreach_claude_agent_gets_its_profile_and_schema() {
    let text = format!(
        "{CONFIG}```json:paper\n{{\"type\": \"object\"}}\n```\n# Job\n## A\n- actions:\n  \
         - foreach agent:claude-agent\n    - model=claude\n    - output_schema=paper\n"
    );
    let Actor::ForEach(inner) = first_action(&text) else {
        panic!("expected foreach");
    };
    let Actor::ClaudeAgent {
        profile,
        output_schema,
        environment,
        default_name,
        ..
    } = inner.as_ref()
    else {
        panic!("expected a claude agent, got {inner:?}");
    };
    assert_eq!(profile.as_ref().map(|p| p.name.as_str()), Some("claude"));
    assert_eq!(
        output_schema.as_ref().map(|s| s.name.as_str()),
        Some("paper")
    );
    assert!(environment.is_none());
    assert_eq!(default_name, "atama-job-a-1");
}

#[test]
fn rejects_invalid_claude_agent_actions() {
    assert_eq!(
        error(&job("  - agent:claude-agent model=claude\n")),
        (
            12,
            "put the options of `agent:claude-agent` in a list under the action, one \
             `- <key>=<value>` bullet on each line, as in `- model=<profile>`"
                .into()
        )
    );
    assert_eq!(
        error(&job("  - agent:claude-agent\n    - task=x\n")),
        (12, "`agent:claude-agent` needs `model=<profile>`".into())
    );
    let (line, message) = error(&job("  - agent:claude-agent\n    - model=other\n"));
    assert_eq!(line, 12);
    assert!(
        message.starts_with("unknown model profile `other`"),
        "{message}"
    );
    assert_eq!(
        error(&job(
            "  - agent:claude-agent\n    - model=claude\n    - environment=web\n"
        )),
        (
            12,
            "claude-env block `web` is not defined in the job".into()
        )
    );
    assert_eq!(
        error(&job(
            "  - agent:claude-agent\n    - model=claude\n    - native_tools=web_fetch\n"
        ))
        .0,
        12
    );
    assert_eq!(
        error(&job(
            "  - agent:claude-agent\n    - model=claude\n    - output_schema=nope\n"
        ))
        .0,
        12
    );
    assert_eq!(
        error(&job("  - agent:claude-agent\n    - model=local\n")),
        (
            12,
            "model profile `local` needs `api_key=<ENV_VAR>` for a claude-agent action".into()
        )
    );
    let (_, message) = error(&job("  - agent:claudeagent\n"));
    assert_eq!(
        message,
        "unknown agent type `claudeagent`, expected `agent:coder`, `agent:summarizer`, \
         `agent:autonomous` or `agent:claude-agent`"
    );
}

#[test]
fn reads_claude_env_blocks() {
    let text = format!(
        "{CONFIG}```claude-env:open\nnetworking: unrestricted\n```\n# Job\n## A\n- actions:\n  \
         - agent:claude-agent\n    - model=claude\n    - environment=open\n    \
         - native_tools=web_search\n"
    );
    let Actor::ClaudeAgent { environment, .. } = first_action(&text) else {
        panic!("expected a claude agent");
    };
    assert_eq!(
        environment.map(|environment| environment.networking),
        Some(Networking::Unrestricted)
    );
}

#[test]
fn rejects_invalid_claude_env_blocks() {
    assert_eq!(
        error("# Job\n```claude-env:a\n```\n## A\n- actions: mock\n"),
        (
            2,
            "the ```claude-env:<name> block must come before the `# <job name>` heading".into()
        )
    );
    assert_eq!(
        error("```claude-env:a b\n```\n# Job\n"),
        (
            1,
            "`a b` is not a claude-env block name, use 1 to 64 letters, digits, `_` or `-`, as \
             in ```claude-env:research"
                .into()
        )
    );
    assert_eq!(
        error("```claude-env:a\n```\n```claude-env:a\n```\n# Job\n"),
        (3, "claude-env block `a` is defined twice".into())
    );
    assert_eq!(
        error("```claude-env:a\nnetworking: limited\nnetworking: open\n```\n# Job\n"),
        (
            3,
            "`networking: open` is not `limited` or `unrestricted`".into()
        )
    );
    assert_eq!(
        error("```claude-env:a\nnetworking: limited\n"),
        (1, "the ```claude-env:a block has no closing ```".into())
    );
}

#[test]
fn reads_effort_on_a_profile() {
    assert_eq!(EFFORT_LEVELS, ["low", "medium", "high", "xhigh", "max"]);
    for level in EFFORT_LEVELS {
        assert_eq!(parse_effort(level).unwrap(), level);
    }
    assert_eq!(
        parse_effort("huge").unwrap_err(),
        "`effort=huge` is not `low`, `medium`, `high`, `xhigh` or `max`"
    );
    let profile: ModelProfile = "c: model=m, base_url=http://x.test, effort=max"
        .parse()
        .unwrap();
    assert_eq!(profile.effort.as_deref(), Some("max"));
    let profile: ModelProfile = "c: model=m, base_url=http://x.test".parse().unwrap();
    assert_eq!(profile.effort, None);
    assert_eq!(
        "c: model=m, base_url=http://x.test, effort=huge"
            .parse::<ModelProfile>()
            .unwrap_err(),
        "`effort=huge` is not `low`, `medium`, `high`, `xhigh` or `max` in profile `c`"
    );
}

#[test]
fn warns_when_an_agent_ignores_effort() {
    let text = format!(
        "{CONFIG}# Job\n## A\n- actions:\n  - agent:summarizer\n    - model=claude\n  \
         - agent:autonomous\n    - model=local\n    - success_check=true\n    \
         - success=It works.\n    - judge_model=claude\n  - agent:coder\n    - model=local\n  \
         - agent:claude-agent\n    - model=claude\n"
    );
    let (job, warnings) = parse_job_with_warnings(&text).unwrap();
    assert_eq!(job.tasks[0].actions.len(), 4);
    assert_eq!(
        warnings,
        [
            ParseWarning {
                line: 8,
                message: "model profile `claude` sets `effort=high`, but `agent:summarizer` \
                          ignores it. Only `agent:claude-agent` uses `effort=`"
                    .into(),
            },
            ParseWarning {
                line: 10,
                message: "model profile `claude` sets `effort=high`, but the judge of \
                          `agent:autonomous` ignores it. Only `agent:claude-agent` uses \
                          `effort=`"
                    .into(),
            },
        ]
    );
    assert_eq!(
        warnings[0].to_string(),
        format!("line 8: {}", warnings[0].message)
    );
    // `parse_job` drops the warnings, and the job is the same.
    assert_eq!(parse_job(&text).unwrap(), job);
}

#[test]
fn a_claude_agent_runs_with_the_key_of_its_profile() {
    // The test process has no `CLAUDE_KEY`, so the run stops before it
    // sends a request.
    let action = first_action(&job("  - agent:claude-agent\n    - model=claude\n"));
    assert_eq!(
        action.act("input").unwrap_err(),
        "agent:claude-agent (claude): model profile `claude` needs the API key in `CLAUDE_KEY`, \
         but `CLAUDE_KEY` is not set"
    );
    let mut debug = Vec::new();
    assert!(
        action
            .act_with("input", Some(&mut debug))
            .unwrap_err()
            .contains("`CLAUDE_KEY` is not set")
    );
    assert_eq!(CLAUDE_AGENT, "agent:claude-agent");
    // An action parsed on its own has no profile and no default name.
    let alone: Actor = "agent:claude-agent model=claude".parse().unwrap();
    assert!(matches!(
        alone,
        Actor::ClaudeAgent { profile: None, ref default_name, .. } if default_name.is_empty()
    ));
}

#[test]
fn sample_claude_agent_job_searches_results_and_reports_the_form_of_the_winners() {
    let job = parse_job(include_str!("../jobs/sample_claude_agent.md")).expect("job should parse");
    let names: Vec<&str> = job.tasks.iter().map(|task| task.id.as_str()).collect();
    assert_eq!(
        names,
        ["Prepare", "Search", "Last Results", "Form of the Winners"]
    );
    let actions = |task: usize| -> Vec<String> {
        job.tasks[task]
            .actions
            .iter()
            .map(|action| action.to_string())
            .collect()
    };
    assert_eq!(
        actions(1),
        [
            "websearch topic=\"Premier League results this week final scores\" max_results=10",
            "write_file ./output/claude_agent/search.json",
        ]
    );

    let agent = |task: usize| match &job.tasks[task].actions[1] {
        Actor::ClaudeAgent {
            params,
            profile,
            output_schema,
            environment,
            ..
        } => (
            params.clone(),
            profile.clone().unwrap(),
            output_schema.as_ref().map(|schema| schema.name.clone()),
            environment.clone(),
        ),
        other => panic!("expected a claude agent, got {other:?}"),
    };
    for task in [2, 3] {
        let (params, profile, _, environment) = agent(task);
        assert_eq!(profile.model, "claude-opus-5-5");
        assert_eq!(profile.api_key_env.as_deref(), Some("CLAUDE_API_KEY"));
        assert_eq!(profile.effort.as_deref(), Some("high"));
        // The agents only read the search results: no native tools, the
        // default environment.
        assert!(
            !params.iter().any(|param| param.starts_with("native_tools=")),
            "{params:?}"
        );
        assert!(environment.is_none());
        assert!(params.contains(&"max_cost_usd=0.50".to_string()));
    }
    assert_eq!(
        actions(0)[1],
        "bash rm -f ./output/claude_agent/matches.json"
    );
    // The first agent writes `matches.json` with the local `write_file`
    // tool, and a success check makes sure that it is valid JSON.
    let (params, _, schema, _) = agent(2);
    assert_eq!(schema.as_deref(), Some("matches"));
    for option in [
        "local_tools=write_file",
        "autonomy=no_ask",
        "protect=./output/claude_agent/search.json",
        "success_check=test -s ./output/claude_agent/matches.json && python3 -m json.tool \
         ./output/claude_agent/matches.json > /dev/null",
    ] {
        assert!(params.contains(&option.to_string()), "{option} in {params:?}");
    }
    assert_eq!(job.tasks[2].actions.len(), 2);
    let (params, _, schema, _) = agent(3);
    assert_eq!(schema, None);
    assert!(
        !params.iter().any(|param| param.starts_with("local_tools=")),
        "{params:?}"
    );
    assert_eq!(actions(2)[0], "read_file ./output/claude_agent/search.json");
    assert_eq!(
        actions(3)[0],
        "bash cat ./output/claude_agent/matches.json ./output/claude_agent/search.json"
    );
    assert_eq!(
        actions(3)[2],
        "write_file ./output/claude_agent/winners_form.md"
    );
}
