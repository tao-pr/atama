//! Tests for the options of `agent:<type>` actions, in
//! `src/agent_loop_openai_compat/agent_options.rs`.

use atama::actor::{Actor, AgentType};
use atama::agent_loop_openai_compat::agent_options::{
    API_KEY_ENV, AgentConfig, BASE_URL_ENV, DEFAULT_BASE_URL, DEFAULT_EXPERTISE, DEFAULT_MAX_STEPS,
    DEFAULT_SUMMARIZER_PROMPT, MODEL_ENV, MaxOutput, SummarizerOptions,
};

fn params(params: &[&str]) -> Vec<String> {
    params.iter().map(|param| param.to_string()).collect()
}

fn no_env(_: &str) -> Option<String> {
    None
}

#[test]
fn reads_options_and_task_from_params() {
    let config = AgentConfig::from_params(
        AgentType::Coder,
        &params(&[
            "model=m1",
            "task=find the news",
            "base_url=http://x.test/v1",
            "max_steps=3",
        ]),
        no_env,
    )
    .unwrap();
    assert_eq!(config.model, "m1");
    assert_eq!(config.base_url, "http://x.test/v1");
    assert_eq!(config.max_steps, 3);
    assert_eq!(config.task, "find the news");
    assert_eq!(config.api_key, None);
}

#[test]
fn reads_missing_options_and_the_api_key_from_the_environment() {
    let env = |name: &str| match name {
        MODEL_ENV => Some("env-model".to_string()),
        BASE_URL_ENV => Some("http://env.test/v1".to_string()),
        API_KEY_ENV => Some("secret".to_string()),
        _ => None,
    };
    let config = AgentConfig::from_params(AgentType::Coder, &params(&["task=t"]), env).unwrap();
    assert_eq!(config.model, "env-model");
    assert_eq!(config.base_url, "http://env.test/v1");
    assert_eq!(config.api_key.as_deref(), Some("secret"));
    assert_eq!(config.max_steps, DEFAULT_MAX_STEPS);

    let config = AgentConfig::from_params(
        AgentType::Coder,
        &params(&["model=m", "base_url=http://p.test"]),
        env,
    )
    .unwrap();
    assert_eq!(config.model, "m");
    assert_eq!(config.base_url, "http://p.test");
}

#[test]
fn uses_defaults_and_ignores_empty_environment_values() {
    let env = |name: &str| (name != MODEL_ENV).then(String::new);
    let config = AgentConfig::from_params(AgentType::Coder, &params(&["model=m"]), env).unwrap();
    assert_eq!(config.base_url, DEFAULT_BASE_URL);
    assert_eq!(config.api_key, None);
    assert_eq!(config.task, "");
}

#[test]
fn rejects_invalid_configuration() {
    let cases = [
        (
            vec!["task=t"],
            "no model: add `model=<id>` or set `ATAMA_AGENT_MODEL`",
        ),
        (
            vec!["model=m", "find", "it"],
            "unexpected `find`, put the task in `task=\"...\"`",
        ),
        (
            vec!["model=m", "a=b"],
            "unknown option `a`, expected `base_url=`, `model=`, `max_steps=`, \
             `task=`, `expertise=`, `max_output=`",
        ),
        (vec!["model=m", "task="], "option `task` has no value"),
        (vec!["model="], "option `model` has no value"),
        (vec!["model=a", "model=b"], "option `model` is set twice"),
        (
            vec!["model=a", "max_steps=0"],
            "`max_steps=0` is not a positive number",
        ),
        (
            vec!["model=a", "max_steps=x"],
            "`max_steps=x` is not a positive number",
        ),
    ];
    for (input, message) in cases {
        let err = AgentConfig::from_params(AgentType::Coder, &params(&input), no_env)
            .err()
            .expect("configuration should be invalid");
        assert_eq!(err, message, "{input:?}");
    }
    let env = |name: &str| (name == MODEL_ENV).then(String::new);
    assert!(AgentConfig::from_params(AgentType::Coder, &params(&["task=t"]), env).is_err());
}

#[test]
fn reads_quoted_option_values_with_spaces() {
    // The parser removes the quotes, so test the whole path.
    let actor: Actor = r#"agent:coder model="m 1" task="List \"the\"  changes""#
        .parse()
        .unwrap();
    let Actor::AgentTool {
        agent,
        params: words,
    } = actor
    else {
        panic!("expected an agent action");
    };
    let config = AgentConfig::from_params(agent, &words, no_env).unwrap();
    assert_eq!(config.model, "m 1");
    // Spaces inside the quotes do not change.
    assert_eq!(config.task, r#"List "the"  changes"#);
}

#[test]
fn summary_shows_whether_the_api_key_is_set_but_not_the_key() {
    let mut config = AgentConfig {
        base_url: "http://x.test/v1".into(),
        model: "m".into(),
        api_key: Some("secret-key".into()),
        max_steps: 3,
        summarizer: None,
        task: "task".into(),
    };
    assert_eq!(
        config.summary(),
        "model: m\nbase_url: http://x.test/v1\nmax_steps: 3\napi key: set"
    );
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
    } = actor
    else {
        panic!("expected an agent action");
    };
    assert_eq!(agent, AgentType::Summarizer);
    let config = AgentConfig::from_params(agent, &words, no_env).unwrap();
    assert_eq!(
        config.summarizer,
        Some(SummarizerOptions {
            expertise: "molecular biology".into(),
            max_output: Some(MaxOutput::Sentences(3)),
        })
    );

    let config =
        AgentConfig::from_params(AgentType::Summarizer, &params(&["model=m"]), no_env).unwrap();
    assert_eq!(
        config.summarizer,
        Some(SummarizerOptions {
            expertise: DEFAULT_EXPERTISE.into(),
            max_output: None,
        })
    );

    let config = AgentConfig::from_params(AgentType::Coder, &params(&["model=m"]), no_env).unwrap();
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
            "unknown option `topic`, expected `base_url=`, `model=`, `max_steps=`, \
             `task=`, `expertise=`, `max_output=`"
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
        let err = AgentConfig::from_params(agent, &words, no_env)
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
