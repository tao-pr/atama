//! Tests for the job config markdown parser.

use atama::Passer;
use atama::actor::{Actor, AgentType, ApiVerb, Auth};
use atama::agent_loop_openai_compat::agent_options::AgentConfig;
use atama::profile::ModelProfile;
use atama::tavily::DEFAULT_BASE_URL;
use atama::websearch::{DEFAULT_MAX_RESULTS, WebSearchOptions};
use atama::{Event, Hook, ParseError, Trigger, parse_job};

const EXAMPLE_JOB: &str = include_str!("fixtures/example_job.md");

#[test]
fn parses_example_job() {
    let job = parse_job(EXAMPLE_JOB).expect("example job should parse");

    assert_eq!(job.name, "Build and release");
    assert_eq!(
        job.description,
        "Builds the project, runs the tests and publishes a release."
    );
    let ids: Vec<&str> = job.tasks.iter().map(|task| task.id.as_str()).collect();
    assert_eq!(ids, ["Build", "Test", "Lint", "Release", "Cleanup"]);

    let build = &job.tasks[0];
    assert_eq!(build.description, "Compile the release binary.");
    assert_eq!(build.begin_if, Trigger::PrevTask(Event::Succeed));
    assert_eq!(build.actions, [bash("cargo", &["build", "--release"])]);
    assert_eq!(build.if_fail, Some(Hook::Retry(2)));
    assert_eq!(build.if_succeed, Some(Hook::WriteFile("build.log".into())));

    let lint = &job.tasks[2];
    assert_eq!(
        lint.begin_if,
        Trigger::Task {
            id: "Test".into(),
            on: Event::Begin
        }
    );
    assert_eq!(lint.if_fail, Some(Hook::Die));

    let cleanup = &job.tasks[4];
    assert_eq!(cleanup.begin_if, Trigger::PrevTask(Event::SucceedOrFail));
    assert!(cleanup.actions.is_empty());
}

/// Builds the expected [`Actor::BashScript`] for a command and its parameters.
fn bash(cmd: &str, params: &[&str]) -> Actor {
    Actor::BashScript {
        cmd: cmd.into(),
        params: params.iter().map(|param| param.to_string()).collect(),
    }
}

fn api_call(verb: ApiVerb, url: &str, payload: Option<&str>, input_file: Option<&str>) -> Actor {
    Actor::ApiCall {
        verb,
        url: url.into(),
        input_file: input_file.map(Into::into),
        payload: payload.map(Into::into),
        auth: None,
    }
}

fn download(url: &str, mime: Option<&str>, output_dir: Option<&str>) -> Actor {
    Actor::DownloadFromUrl {
        url: url.into(),
        auth: None,
        mime: mime.map(Into::into),
        output_dir: output_dir.map(Into::into),
    }
}

#[test]
fn parses_actions() {
    let cases = [
        ("bash npm run build", bash("npm", &["run", "build"])),
        ("bash make", bash("make", &[])),
        (
            "  bash \t  ./deploy.sh   staging  ",
            bash("./deploy.sh", &["staging"]),
        ),
        (
            "api_call GET https://x.test",
            api_call(ApiVerb::GET, "https://x.test", None, None),
        ),
        (
            "api_call post https://x.test payload={\"a\":1}",
            api_call(ApiVerb::POST, "https://x.test", Some("{\"a\":1}"), None),
        ),
        (
            "api_call Patch https://x.test input_file=body.json",
            api_call(ApiVerb::PATCH, "https://x.test", None, Some("body.json")),
        ),
        (
            "api_call PUT https://x.test input_file=in.txt payload=text",
            api_call(ApiVerb::PUT, "https://x.test", Some("text"), Some("in.txt")),
        ),
        (
            "download https://x.test/a.zip",
            download("https://x.test/a.zip", None, None),
        ),
        (
            "download https://x.test/a.zip output_dir=out mime=application/zip",
            download("https://x.test/a.zip", Some("application/zip"), Some("out")),
        ),
        ("mock", Actor::MockPlaceholder),
        ("mock build step", Actor::MockPlaceholder),
        ("mock fail", Actor::MockFail),
        ("mock fail with a note", Actor::MockFail),
    ];
    for (input, expected) in cases {
        assert_eq!(input.parse::<Actor>(), Ok(expected), "input: {input:?}");
    }
}

#[test]
fn parses_every_api_verb() {
    let verbs = [
        ("GET", ApiVerb::GET),
        ("POST", ApiVerb::POST),
        ("PUT", ApiVerb::PUT),
        ("DELETE", ApiVerb::DELETE),
        ("PATCH", ApiVerb::PATCH),
        ("HEAD", ApiVerb::HEAD),
    ];
    for (text, verb) in verbs {
        assert_eq!(text.parse::<ApiVerb>(), Ok(verb.clone()));
        assert_eq!(text.to_lowercase().parse::<ApiVerb>(), Ok(verb));
    }
}

#[test]
fn parses_agent_tool_actions() {
    let cases = [
        ("agent:summarizer", AgentType::Summarizer, vec![]),
        (
            "agent:coder max_steps=3 task=\"fix the  tests\"",
            AgentType::Coder,
            vec!["max_steps=3", "task=fix the  tests"],
        ),
        ("agent:coder   fix.md ", AgentType::Coder, vec!["fix.md"]),
        (
            "agent:summarizer max_output=60%",
            AgentType::Summarizer,
            vec!["max_output=60%"],
        ),
        (
            r#"agent:coder task="say \"hi\" \\ a\b" "" "a b"c"#,
            AgentType::Coder,
            vec![r#"task=say "hi" \ a\b"#, "", "a bc"],
        ),
    ];
    for (statement, agent, params) in cases {
        let actor: Actor = statement.parse().expect(statement);
        assert_eq!(
            actor,
            Actor::AgentTool {
                agent,
                params: params.into_iter().map(Into::into).collect(),
                profile: None,
            },
            "{statement}"
        );
    }
}

#[test]
fn reads_model_profiles_from_the_config_block() {
    let job = parse_job(
        "```config\n\
         local: model=qwen3.5:0.8b, base_url=http://localhost:11434/v1\n\
         \n\
         cloud: model=gpt-5,base_url=https://api.openai.com/v1 , api_key=OPENAI_API_KEY\n\
         ```\n\
         \n\
         # Job\n\
         ## A\n\
         - actions:\n  \
           - agent:coder model=local task=t\n  \
           - agent:summarizer model=cloud\n",
    )
    .expect("job should parse");
    let profiles: Vec<ModelProfile> = job.tasks[0]
        .actions
        .iter()
        .map(|actor| match actor {
            Actor::AgentTool {
                profile: Some(profile),
                ..
            } => profile.clone(),
            other => panic!("expected an agent with a profile, got {other:?}"),
        })
        .collect();
    assert_eq!(
        profiles,
        [
            ModelProfile {
                name: "local".into(),
                model: "qwen3.5:0.8b".into(),
                base_url: "http://localhost:11434/v1".into(),
                api_key_env: None,
            },
            ModelProfile {
                name: "cloud".into(),
                model: "gpt-5".into(),
                base_url: "https://api.openai.com/v1".into(),
                api_key_env: Some("OPENAI_API_KEY".into()),
            },
        ]
    );
    // The printed action keeps the profile name.
    assert_eq!(
        job.tasks[0].actions[0].to_string(),
        "agent:coder model=local task=t"
    );
}

#[test]
fn rejects_invalid_model_profiles() {
    let job =
        |config: &str, action: &str| format!("{config}# Job\n## A\n- actions:\n  - {action}\n");
    let profile = "```config\np: model=m, base_url=http://x.test\n```\n";
    let cases = [
        (
            job("", "agent:coder model=p"),
            4,
            "unknown model profile `p`: the job has no ```config block with model profiles",
        ),
        (
            job(profile, "agent:coder model=q"),
            7,
            "unknown model profile `q`, expected `p`",
        ),
        (
            job(profile, "agent:coder task=t"),
            7,
            "`agent:coder` needs `model=<profile>`, the name of a model profile in the \
             ```config block at the start of the job",
        ),
        (
            job("```config\np model=m\n```\n", "mock"),
            2,
            "`p model=m` is not a profile, expected `<name>: model=<id>, base_url=<url>`",
        ),
        (
            job("```config\nmy p: model=m\n```\n", "mock"),
            2,
            "`my p` is not a profile name, use one word as in `local-qwen`",
        ),
        (
            job("```config\n: model=m\n```\n", "mock"),
            2,
            "`` is not a profile name, use one word as in `local-qwen`",
        ),
        (
            job("```config\np: model\n```\n", "mock"),
            2,
            "unexpected `model` in profile `p`, expected `key=value`",
        ),
        (
            job("```config\np: key=k\n```\n", "mock"),
            2,
            "unknown setting `key` in profile `p`, expected `model=`, `base_url=` or `api_key=`",
        ),
        (
            job("```config\np: model=\n```\n", "mock"),
            2,
            "setting `model` in profile `p` has no value",
        ),
        (
            job("```config\np: model=a, model=b\n```\n", "mock"),
            2,
            "setting `model` in profile `p` is set twice",
        ),
        (
            job("```config\np: base_url=http://x.test\n```\n", "mock"),
            2,
            "profile `p` needs `model=`",
        ),
        (
            job("```config\np: model=m\n```\n", "mock"),
            2,
            "profile `p` needs `base_url=`",
        ),
        (
            job(
                "```config\np: model=m, base_url=u\n\np: model=n, base_url=u\n```\n",
                "mock",
            ),
            4,
            "profile `p` is defined twice",
        ),
        (
            job(&format!("{profile}```config\n```\n"), "mock"),
            4,
            "more than one ```config block",
        ),
        (
            job("```config\np: model=m, base_url=u\n", "mock"),
            1,
            "the ```config block has no closing ```",
        ),
        (
            "# Job\n```config\n```\n## A\n- actions: mock\n".to_string(),
            2,
            "the ```config block must come before the `# <job name>` heading",
        ),
    ];
    for (text, line, message) in cases {
        assert_eq!(
            parse_job(&text),
            Err(ParseError {
                line,
                message: message.to_string()
            }),
            "{text}"
        );
    }
}

#[test]
fn parses_websearch_actions() {
    let cases = [
        (
            "websearch topic=rust",
            WebSearchOptions {
                topic: "rust".into(),
                max_results: DEFAULT_MAX_RESULTS,
                fetch_content: false,
                base_url: DEFAULT_BASE_URL.into(),
            },
        ),
        (
            "websearch  base_url=http://x.test fetch_content=true max_results=20 topic=\"rust  release\"",
            WebSearchOptions {
                topic: "rust  release".into(),
                max_results: 20,
                fetch_content: true,
                base_url: "http://x.test".into(),
            },
        ),
        (
            "websearch topic=rust fetch_content=false",
            WebSearchOptions {
                topic: "rust".into(),
                max_results: DEFAULT_MAX_RESULTS,
                fetch_content: false,
                base_url: DEFAULT_BASE_URL.into(),
            },
        ),
    ];
    for (statement, options) in cases {
        let actor: Actor = statement.parse().expect(statement);
        assert_eq!(actor, Actor::WebSearch(options), "{statement}");
    }
}

#[test]
fn rejects_invalid_websearch_actions() {
    let cases = [
        (
            "websearch",
            "`websearch` needs a topic, as in `topic=\"latest Rust release\"`",
        ),
        (
            "websearch rust",
            "unexpected `rust`, expected `topic=` or `max_results=` or `fetch_content=` or `base_url=`",
        ),
        (
            "websearch topic=a model=m",
            "unknown option `model`, expected `topic=` or `max_results=` or `fetch_content=` or `base_url=`",
        ),
        ("websearch topic=a topic=b", "duplicate option `topic`"),
        ("websearch topic=", "option `topic` has no value"),
        (
            "websearch topic=a max_results=0",
            "`max_results=0` is not a number from 1 to 20",
        ),
        (
            "websearch topic=a max_results=21",
            "`max_results=21` is not a number from 1 to 20",
        ),
        (
            "websearch topic=a max_results=x",
            "`max_results=x` is not a number from 1 to 20",
        ),
        (
            "websearch topic=a fetch_content=yes",
            "`fetch_content=yes` is not `true` or `false`",
        ),
        (
            "websearch topic=\"rust",
            "a quote in `topic=\"rust` has no closing quote",
        ),
        (
            "agent:websearch topic=rust",
            "`agent:websearch` is now the `websearch` action, because it asks no model",
        ),
    ];
    for (statement, message) in cases {
        assert_eq!(
            statement.parse::<Actor>(),
            Err(message.to_string()),
            "{statement}"
        );
    }
}

#[test]
fn parses_tavily_search_actions() {
    let cases = [
        ("tavily_search", vec![]),
        (
            "tavily_search max_results=3 rust release notes",
            vec!["max_results=3", "rust", "release", "notes"],
        ),
    ];
    for (statement, params) in cases {
        let actor: Actor = statement.parse().expect(statement);
        assert_eq!(
            actor,
            Actor::TavilySearch {
                params: params.into_iter().map(Into::into).collect(),
            },
            "{statement}"
        );
    }
}

#[test]
fn rejects_invalid_actions() {
    let cases = [
        ("", "action is empty"),
        ("   ", "action is empty"),
        ("npm ci", "unknown action `npm`"),
        ("Bash ls", "unknown action `Bash`"),
        ("bash", "`bash` needs a command"),
        ("agent search", "unknown action `agent`"),
        ("agent:", "unknown agent type ``"),
        ("agent:browser", "unknown agent type `browser`"),
        ("agent:WebSearch", "unknown agent type `WebSearch`"),
        (
            "agent:coder task=\"fix it",
            "a quote in `task=\"fix it` has no closing quote",
        ),
        (
            "agent:coder task=\"fix it\\",
            "a quote in `task=\"fix it\\` has no closing quote",
        ),
        ("api_call", "`api_call` needs a verb and a URL"),
        ("api_call GET", "`api_call` needs a verb and a URL"),
        ("api_call FETCH https://x.test", "unknown API verb `FETCH`"),
        ("api_call GET https://x.test extra", "unexpected `extra`"),
        (
            "api_call GET https://x.test mime=a/b",
            "unknown option `mime`",
        ),
        (
            "api_call GET https://x.test payload=",
            "option `payload` has no value",
        ),
        (
            "api_call GET https://x.test payload=a payload=b",
            "duplicate option `payload`",
        ),
        ("download", "`download` needs a URL"),
        (
            "api_call GET https://x.test auth=bearer:",
            "unsupported auth",
        ),
        ("api_call GET https://x.test auth=token", "unsupported auth"),
        ("download https://x.test auth=basic:a:b", "unsupported auth"),
        (
            "download https://x.test payload=a",
            "unknown option `payload`",
        ),
    ];
    for (input, message) in cases {
        let err = input.parse::<Actor>().expect_err(input);
        assert!(err.contains(message), "input: {input:?}, error: {err}");
    }
}

#[test]
fn parses_inline_action() {
    let job = parse_job("# Job\n## A\n- actions: mock fail\n").expect("job should parse");

    assert_eq!(job.tasks[0].actions, [Actor::MockFail]);
}

#[test]
fn reports_line_of_invalid_action() {
    assert_error(
        "# Job\n## A\n- actions: npm ci\n",
        3,
        "unknown action `npm`",
    );
    assert_error(
        "# Job\n## A\n- actions:\n  - mock\n  - bash\n",
        5,
        "`bash` needs a command",
    );
}

#[test]
fn parses_trigger_conditions() {
    let cases = [
        ("prev task succeed", Trigger::PrevTask(Event::Succeed)),
        ("prev task fail", Trigger::PrevTask(Event::Fail)),
        (
            "prev task succeed or fail",
            Trigger::PrevTask(Event::SucceedOrFail),
        ),
        (
            "task(Build) succeed or fail",
            Trigger::Task {
                id: "Build".into(),
                on: Event::SucceedOrFail,
            },
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(input.parse::<Trigger>(), Ok(expected), "input: {input}");
    }

    assert!("prev task begin".parse::<Trigger>().is_err());
    assert!("task() succeed".parse::<Trigger>().is_err());
    assert!("whenever".parse::<Trigger>().is_err());
}

#[test]
fn parses_hooks() {
    let cases = [
        ("retry=3", Hook::Retry(3)),
        ("die", Hook::Die),
        ("restart", Hook::Restart),
        (
            "call_api(https://x.test)",
            Hook::CallApi("https://x.test".into()),
        ),
        ("write_file(out.txt)", Hook::WriteFile("out.txt".into())),
        (
            "write_error_file(err.txt)",
            Hook::WriteErrorFile("err.txt".into()),
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(input.parse::<Hook>(), Ok(expected), "input: {input}");
    }

    assert!("retry=many".parse::<Hook>().is_err());
    assert!("write_file(out.txt".parse::<Hook>().is_err());
    assert!("explode".parse::<Hook>().is_err());
}

#[test]
fn ignores_headings_inside_code_blocks() {
    let input = "# Job\n\n## Task\n\n```sh\n# not a heading\n## not a task\n```\n";
    let job = parse_job(input).expect("job should parse");

    assert_eq!(job.tasks.len(), 1);
    assert!(job.tasks[0].description.contains("## not a task"));
}

/// Asserts that parsing fails on `line` with a message containing `message`.
fn assert_error(input: &str, line: usize, message: &str) {
    let err = parse_job(input).expect_err("input should not parse");
    assert_eq!(err.line, line, "error was: {err}");
    assert!(err.message.contains(message), "error was: {err}");
}

#[test]
fn rejects_invalid_jobs() {
    assert_error("", 1, "missing `# <job name>`");
    assert_error("some text\n# Job\n", 1, "expected the `# <job name>`");
    assert_error("# Job\n# Other\n", 2, "more than one");
    assert_error("# Job\n", 1, "no `## <task>`");
    assert_error("# Job\n## A\n## A\n", 3, "duplicate task id `A`");
    assert_error("# Job\n## A\n- begin_if: prev task fail\n", 3, "first task");
    assert_error(
        "# Job\n## A\n## B\n- begin_if: task(C) fail\n",
        4,
        "unknown task id `C`",
    );
    assert_error(
        "# Job\n## A\n## B\n- begin_if: task(B) fail\n",
        4,
        "depends on itself",
    );
    assert_error(
        "# Job\n## A\n- if_fail: retry=1\n- if_fail: die\n",
        4,
        "duplicate `if_fail`",
    );
    assert_error("# Job\n## A\n- if_succeed:\n", 3, "has no value");
    assert_error("## A\n# Job\n", 1, "before the `# <job name>`");
    assert_error("# Job\n## \n", 2, "task heading is empty");
    assert_error("#  \n## A\n", 1, "job name heading is empty");
    assert_error(
        "# Job\n## A\n- actions: mock\n- actions: mock\n",
        4,
        "duplicate `actions`",
    );
    assert_error(
        "# Job\n## A\n## B\n- begin_if: prev task fail\n- begin_if: prev task succeed\n",
        5,
        "duplicate `begin_if`",
    );
    assert_error(
        "# Job\n## A\n## B\n- begin_if: task(A) explode\n",
        4,
        "unknown event `explode`",
    );
    assert_error(
        "# Job\n## A\n- if_fail: notify(me)\n",
        3,
        "unknown hook `notify`",
    );
}

#[test]
fn skips_blank_lines_before_the_job_name() {
    let job = parse_job("\n  \n# Job\n## A\n").expect("job should parse");

    assert_eq!(job.name, "Job");
}

#[test]
fn shows_parse_error_with_its_line() {
    let err = parse_job("# Job\n## A\n- actions: npm ci\n").unwrap_err();

    assert_eq!(
        err.to_string(),
        "line 3: unknown action `npm`, expected `bash`, `api_call`, `download`, \
         `tavily_search`, `websearch`, `mock` or `agent:<type>`"
    );
}

fn bearer(token: &str) -> Option<Auth> {
    Some(Auth::Bearer {
        token: Passer::Data(token.into()),
    })
}

#[test]
fn parses_bearer_auth_option() {
    let actor: Actor = "api_call PUT https://x.test auth=bearer:my-token"
        .parse()
        .unwrap();
    assert_eq!(
        actor,
        Actor::ApiCall {
            verb: ApiVerb::PUT,
            url: "https://x.test".into(),
            input_file: None,
            payload: None,
            auth: bearer("my-token"),
        }
    );

    let actor: Actor = "download https://x.test/a.txt auth=bearer:a:b"
        .parse()
        .unwrap();
    assert_eq!(
        actor,
        Actor::DownloadFromUrl {
            url: "https://x.test/a.txt".into(),
            auth: bearer("a:b"),
            mime: None,
            output_dir: None,
        }
    );
}

#[test]
fn sample_api_calls_job_uses_every_action_type() {
    let job = parse_job(include_str!("../jobs/sample_api_calls.md")).expect("job should parse");
    let actions: Vec<&Actor> = job.tasks.iter().flat_map(|task| &task.actions).collect();

    let uses = |matches: fn(&Actor) -> bool| actions.iter().any(|actor| matches(actor));
    assert!(uses(|actor| matches!(actor, Actor::BashScript { .. })));
    assert!(uses(|actor| matches!(
        actor,
        Actor::ApiCall { auth: None, .. }
    )));
    assert!(uses(|actor| matches!(
        actor,
        Actor::ApiCall { auth: Some(_), .. }
    )));
    assert!(uses(|actor| matches!(actor, Actor::DownloadFromUrl { .. })));
    assert!(uses(|actor| matches!(actor, Actor::MockPlaceholder)));
    assert!(uses(|actor| matches!(actor, Actor::MockFail)));
}

#[test]
fn sample_agents_job_searches_and_then_summarizes() {
    let job = parse_job(include_str!("../jobs/sample_agents.md")).expect("job should parse");
    let actions: Vec<&Actor> = job.tasks.iter().flat_map(|task| &task.actions).collect();

    let searches: Vec<&WebSearchOptions> = actions
        .iter()
        .filter_map(|actor| match actor {
            Actor::WebSearch(options) => Some(options),
            _ => None,
        })
        .collect();
    let [research] = searches.as_slice() else {
        panic!("expected one websearch action, got {searches:?}");
    };
    assert!(research.fetch_content);
    assert!(
        research.topic.contains("agent loop patterns"),
        "{}",
        research.topic
    );

    let agents: Vec<(AgentType, &Vec<String>, &Option<ModelProfile>)> = actions
        .iter()
        .filter_map(|actor| match actor {
            Actor::AgentTool {
                agent,
                params,
                profile,
            } => Some((*agent, params, profile)),
            _ => None,
        })
        .collect();
    let [(AgentType::Summarizer, summarizer, profile)] = agents.as_slice() else {
        panic!("expected one summarizer agent, got {agents:?}");
    };
    // The summarizer uses the local Ollama profile, which needs no API key.
    assert_eq!(
        profile.as_ref(),
        Some(&ModelProfile {
            name: "local-qwen".into(),
            model: "qwen3.5:0.8b".into(),
            base_url: "http://localhost:11434/v1".into(),
            api_key_env: None,
        })
    );
    let no_env = |_: &str| None;
    let config =
        AgentConfig::from_params(AgentType::Summarizer, summarizer, profile.as_ref(), &no_env)
            .expect("valid options");
    assert_eq!(config.model, "qwen3.5:0.8b");
    assert_eq!(config.api_key, None);
    assert!(
        !summarizer
            .iter()
            .any(|param| param.split('=').next().unwrap().contains("key")),
        "the job file must not have an API key: {summarizer:?}"
    );
    // The task tells the model what to do. Its wording can change.
    assert!(!config.task.is_empty());
}

#[test]
fn sample_agents_job_saves_the_research_and_its_summary() {
    let job = parse_job(include_str!("../jobs/sample_agents.md")).expect("job should parse");
    let ids: Vec<&str> = job.tasks.iter().map(|task| task.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "Prepare",
            "Research",
            "Extract results",
            "Save raw research",
            "Summarize research",
            "Save summary",
            "Print summary",
        ]
    );
    let actions: Vec<String> = job
        .tasks
        .iter()
        .flat_map(|task| &task.actions)
        .map(|actor| actor.to_string())
        .collect();
    for action in [
        "bash jq .results",
        "bash ./jobs/scripts/split_json_array.sh ./output/agents/research-raw",
        "bash xargs jq -s .",
        "bash tee ./output/agents/research-summary.md",
        "bash cat ./output/agents/research-summary.md",
    ] {
        assert!(actions.iter().any(|written| written == action), "{action}");
    }
}

#[test]
fn treats_deeper_headings_as_description() {
    let job = parse_job("# Job\n## A\n### Notes\nsome text\n").expect("job should parse");

    assert_eq!(job.tasks.len(), 1);
    assert_eq!(job.tasks[0].description, "### Notes\nsome text");
}
