//! Tests for the options of `agent:<type>` actions, in
//! `src/agent_loop_openai_compat/agent_options.rs`.

use atama::actor::{Actor, AgentType};
use atama::agent_loop_openai_compat::agent_options::{
    AgentConfig, DEFAULT_EXPERTISE, DEFAULT_MAX_STEPS, DEFAULT_SUMMARIZER_PROMPT, MaxOutput,
    SummarizerOptions,
};
use atama::profile::ModelProfile;

fn params(params: &[&str]) -> Vec<String> {
    params.iter().map(|param| param.to_string()).collect()
}

fn no_env(_: &str) -> Option<String> {
    None
}

/// The model profile `m`, for a server that needs no API key.
fn profile() -> ModelProfile {
    ModelProfile {
        name: "m".into(),
        model: "model-1".into(),
        base_url: "http://x.test/v1".into(),
        api_key_env: None,
        token_budget: None,
    }
}

#[test]
fn reads_options_and_task_from_params_and_the_model_from_the_profile() {
    let config = AgentConfig::from_params(
        AgentType::Coder,
        &params(&["model=m", "task=find the news", "max_steps=3"]),
        Some(&profile()),
        &no_env,
    )
    .unwrap();
    assert_eq!(config.profile, "m");
    assert_eq!(config.model, "model-1");
    assert_eq!(config.base_url, "http://x.test/v1");
    assert_eq!(config.max_steps, 3);
    assert_eq!(config.task, "find the news");
    assert_eq!(config.api_key, None);

    let config = AgentConfig::from_params(
        AgentType::Coder,
        &params(&["model=m"]),
        Some(&profile()),
        &no_env,
    )
    .unwrap();
    assert_eq!(config.max_steps, DEFAULT_MAX_STEPS);
    assert_eq!(config.task, "");
}

#[test]
fn the_token_budget_of_the_action_replaces_the_budget_of_the_profile() {
    let budget = |words: &[&str], profile_budget: Option<u64>| {
        let profile = ModelProfile {
            token_budget: profile_budget,
            ..profile()
        };
        AgentConfig::from_params(AgentType::Coder, &params(words), Some(&profile), &no_env)
            .unwrap()
            .token_budget
    };
    assert_eq!(budget(&["model=m"], None), None);
    assert_eq!(budget(&["model=m"], Some(9000)), Some(9000));
    assert_eq!(budget(&["model=m", "token_budget=500"], None), Some(500));
    assert_eq!(
        budget(&["model=m", "token_budget=500"], Some(9000)),
        Some(500)
    );
}

#[test]
fn reads_thinking_as_true_or_false_and_leaves_it_unset_by_default() {
    let thinking = |words: &[&str]| {
        AgentConfig::from_params(
            AgentType::Summarizer,
            &params(words),
            Some(&profile()),
            &no_env,
        )
        .map(|config| config.thinking)
    };
    assert_eq!(thinking(&["model=m"]), Ok(None));
    assert_eq!(thinking(&["model=m", "thinking=false"]), Ok(Some(false)));
    assert_eq!(thinking(&["model=m", "thinking=true"]), Ok(Some(true)));
    assert_eq!(
        thinking(&["model=m", "thinking=no"]),
        Err("`thinking=no` is not `true` or `false`".to_string())
    );
}

#[test]
fn reads_output_schema_for_every_agent_type() {
    for agent in [AgentType::Coder, AgentType::Summarizer] {
        let output_schema = |words: &[&str]| {
            AgentConfig::from_params(agent, &params(words), Some(&profile()), &no_env)
                .map(|config| config.output_schema)
        };
        assert_eq!(output_schema(&["model=m"]), Ok(None));
        assert_eq!(
            output_schema(&["model=m", "output_schema=article"]),
            Ok(Some("article".to_string()))
        );
        assert_eq!(
            output_schema(&["model=m", "output_schema=a/b.json"]),
            Ok(Some("a/b.json".to_string()))
        );
        assert_eq!(
            output_schema(&["model=m", "output_schema=a", "output_schema=b"]),
            Err("option `output_schema` is set twice".to_string())
        );
    }
}

#[test]
fn reads_the_api_key_from_the_variable_that_the_profile_names() {
    let profile = ModelProfile {
        api_key_env: Some("MY_KEY".into()),
        ..profile()
    };
    let env = |name: &str| (name == "MY_KEY").then(|| "secret".to_string());
    let config = AgentConfig::from_params(
        AgentType::Coder,
        &params(&["model=m"]),
        Some(&profile),
        &env,
    )
    .unwrap();
    assert_eq!(config.api_key.as_deref(), Some("secret"));

    let message = "model profile `m` needs the API key in `MY_KEY`, but `MY_KEY` is not set";
    for env in [no_env, |_: &str| Some(String::new())] {
        assert_eq!(
            AgentConfig::from_params(
                AgentType::Coder,
                &params(&["model=m"]),
                Some(&profile),
                &env
            )
            .err(),
            Some(message.to_string())
        );
    }
}

#[test]
fn rejects_invalid_configuration() {
    let cases = [
        (
            vec!["task=t"],
            "`agent:coder` needs `model=<profile>`, the name of a model profile in the \
             ```config block at the start of the job",
        ),
        (
            vec!["model=other"],
            "model profile `other` is not defined in the job",
        ),
        (
            vec!["model=m", "find", "it"],
            "unexpected `find`, put the task in `task=\"...\"`",
        ),
        (
            vec!["model=m", "base_url=http://x.test"],
            "unknown option `base_url`, expected `model=`, `max_steps=`, `token_budget=`, `task=`, `thinking=`, `output_schema=`, \
             `expertise=`, `max_output=`",
        ),
        (vec!["model=m", "task="], "option `task` has no value"),
        (vec!["model="], "option `model` has no value"),
        (vec!["model=m", "model=m"], "option `model` is set twice"),
        (
            vec!["model=m", "max_steps=0"],
            "`max_steps=0` is not a positive number",
        ),
        (
            vec!["model=m", "max_steps=x"],
            "`max_steps=x` is not a positive number",
        ),
        (
            vec!["model=m", "token_budget=0"],
            "`token_budget=0` is not a positive number",
        ),
        (
            vec!["model=m", "token_budget=-5"],
            "`token_budget=-5` is not a positive number",
        ),
    ];
    for (input, message) in cases {
        let err =
            AgentConfig::from_params(AgentType::Coder, &params(&input), Some(&profile()), &no_env)
                .err()
                .expect("configuration should be invalid");
        assert_eq!(err, message, "{input:?}");
    }
    // An action parsed on its own has no profile.
    assert_eq!(
        AgentConfig::from_params(AgentType::Coder, &params(&["model=m"]), None, &no_env).err(),
        Some("model profile `m` is not defined in the job".to_string())
    );
}

#[test]
fn reads_quoted_option_values_with_spaces() {
    // The parser removes the quotes, so test the whole path.
    let actor: Actor = r#"agent:coder model="m" task="List \"the\"  changes""#
        .parse()
        .unwrap();
    let Actor::AgentTool {
        agent,
        params: words,
        ..
    } = actor
    else {
        panic!("expected an agent action");
    };
    let config = AgentConfig::from_params(agent, &words, Some(&profile()), &no_env).unwrap();
    // Spaces inside the quotes do not change.
    assert_eq!(config.task, r#"List "the"  changes"#);
}

#[test]
fn summary_shows_whether_the_api_key_is_set_but_not_the_key() {
    let mut config = AgentConfig {
        profile: "p".into(),
        base_url: "http://x.test/v1".into(),
        model: "m".into(),
        api_key: Some("secret-key".into()),
        max_steps: 3,
        token_budget: None,
        summarizer: None,
        task: "task".into(),
        thinking: None,
        output_schema: None,
    };
    assert_eq!(
        config.summary(),
        "profile: p\nmodel: m\nbase_url: http://x.test/v1\nmax_steps: 3\napi key: set"
    );
    config.token_budget = Some(800);
    assert!(
        config
            .summary()
            .ends_with("\napi key: set\ntoken_budget: 800")
    );
    config.token_budget = None;
    config.thinking = Some(false);
    assert!(
        config
            .summary()
            .ends_with("\napi key: set\nthinking: false")
    );
    config.output_schema = Some("article".into());
    assert!(
        config
            .summary()
            .ends_with("\napi key: set\nthinking: false\noutput_schema: article")
    );
    config.output_schema = None;
    config.thinking = None;
    config.api_key = None;
    assert!(config.summary().ends_with("\napi key: not set"));

    config.summarizer = Some(SummarizerOptions {
        expertise: "law".into(),
        max_output: None,
    });
    assert!(
        config
            .summary()
            .ends_with("\napi key: not set\nexpertise: law\nmax_output: not set"),
        "{}",
        config.summary()
    );
    config.summarizer = Some(SummarizerOptions {
        expertise: "law".into(),
        max_output: Some(MaxOutput::Percent(60)),
    });
    assert!(config.summary().ends_with("\nmax_output: 60%"));
}

#[test]
fn reads_expertise_and_max_output_for_summarizer() {
    let actor: Actor =
        r#"agent:summarizer model=m expertise="molecular biology" max_output="3 sentences""#
            .parse()
            .unwrap();
    let Actor::AgentTool {
        agent,
        params: words,
        ..
    } = actor
    else {
        panic!("expected an agent action");
    };
    assert_eq!(agent, AgentType::Summarizer);
    let config = AgentConfig::from_params(agent, &words, Some(&profile()), &no_env).unwrap();
    assert_eq!(
        config.summarizer,
        Some(SummarizerOptions {
            expertise: "molecular biology".into(),
            max_output: Some(MaxOutput::Sentences(3)),
        })
    );

    let config = AgentConfig::from_params(
        AgentType::Summarizer,
        &params(&["model=m"]),
        Some(&profile()),
        &no_env,
    )
    .unwrap();
    assert_eq!(
        config.summarizer,
        Some(SummarizerOptions {
            expertise: DEFAULT_EXPERTISE.into(),
            max_output: None,
        })
    );

    let config = AgentConfig::from_params(
        AgentType::Coder,
        &params(&["model=m"]),
        Some(&profile()),
        &no_env,
    )
    .unwrap();
    assert_eq!(config.summarizer, None);
}

#[test]
fn reads_and_writes_max_output_sizes() {
    let cases = [
        ("60%", MaxOutput::Percent(60), "60%"),
        ("100%", MaxOutput::Percent(100), "100%"),
        ("3 sentences", MaxOutput::Sentences(3), "3 sentences"),
        ("1 sentence", MaxOutput::Sentences(1), "1 sentence"),
        ("100 words", MaxOutput::Words(100), "100 words"),
        ("1 word", MaxOutput::Words(1), "1 word"),
        ("2 paragraphs", MaxOutput::Paragraphs(2), "2 paragraphs"),
        ("1 paragraph", MaxOutput::Paragraphs(1), "1 paragraph"),
    ];
    for (text, size, written) in cases {
        assert_eq!(text.parse::<MaxOutput>(), Ok(size), "{text}");
        assert_eq!(size.to_string(), written);
    }
}

#[test]
fn rejects_invalid_summarizer_options() {
    let size_error = |value: &str| {
        format!(
            "`max_output={value}` is not a size, expected a percent from 1 to 100 as in \
             `60%`, or a number of sentences, words or paragraphs as in `3 sentences`"
        )
    };
    let mut cases: Vec<(AgentType, Vec<String>, String)> =
        ["0%", "101%", "x%", "3", "0 words", "x words", "3 pages"]
            .into_iter()
            .map(|value| {
                (
                    AgentType::Summarizer,
                    vec![format!("max_output={value}")],
                    size_error(value),
                )
            })
            .collect();
    cases.extend([
        (
            AgentType::Summarizer,
            vec!["expertise=a".into(), "expertise=b".into()],
            "option `expertise` is set twice".into(),
        ),
        (
            AgentType::Summarizer,
            vec!["topic=rust".into()],
            "unknown option `topic`, expected `model=`, `max_steps=`, `token_budget=`, `task=`, `thinking=`, `output_schema=`, \
             `expertise=`, `max_output=`"
                .into(),
        ),
        (
            AgentType::Coder,
            vec!["expertise=law".into()],
            "option `expertise` works only with `agent:summarizer`".into(),
        ),
        (
            AgentType::Coder,
            vec!["max_output=60%".into()],
            "option `max_output` works only with `agent:summarizer`".into(),
        ),
    ]);
    for (agent, mut words, message) in cases {
        words.insert(0, "model=m".into());
        let err = AgentConfig::from_params(agent, &words, Some(&profile()), &no_env)
            .err()
            .expect("configuration should be invalid");
        assert_eq!(err, message, "{agent} {words:?}");
    }
}

#[test]
fn summarizer_task_has_the_expertise_and_the_length_limit() {
    let options = |max_output| SummarizerOptions {
        expertise: "law".into(),
        max_output,
    };
    let input = "one two three four five six seven eight nine ten";
    let cases = [
        (
            None,
            "",
            format!(
                "{DEFAULT_SUMMARIZER_PROMPT}\n\nExpertise: law\nLength of the summary: \
                 as short as it can be without losing key information"
            ),
        ),
        (
            Some(MaxOutput::Sentences(3)),
            "List the risks.",
            "List the risks.\n\nExpertise: law\nLength of the summary: at most 3 sentences"
                .to_string(),
        ),
        (
            Some(MaxOutput::Percent(60)),
            "",
            format!(
                "{DEFAULT_SUMMARIZER_PROMPT}\n\nExpertise: law\nLength of the summary: \
                 at most 60% of the input, about 6 words of 10"
            ),
        ),
        // The number of words is rounded up.
        (
            Some(MaxOutput::Percent(15)),
            "",
            format!(
                "{DEFAULT_SUMMARIZER_PROMPT}\n\nExpertise: law\nLength of the summary: \
                 at most 15% of the input, about 2 words of 10"
            ),
        ),
    ];
    for (max_output, prompt, task) in cases {
        assert_eq!(options(max_output).task(prompt, input), Ok(task));
    }
    assert_eq!(
        options(None).task("", " \n"),
        Err("`agent:summarizer` needs an input to summarize".to_string())
    );
}
