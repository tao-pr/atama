//! Tests for writing an actor back as an action statement, the `Display`
//! impl in `src/actor/display.rs`.

use atama::Passer;
use atama::actor::{Actor, Auth};

fn web_search() -> Actor {
    Actor::AgentWebSearch {
        research_topic: "rust".to_string(),
        prefer_websites: None,
        prefer_models: Vec::new(),
        output_dir: None,
        output_instructions: String::new(),
        structured_output: String::new(),
    }
}

#[test]
fn writes_actions_back_as_statements() {
    let statements = [
        "bash npm run build",
        "api_call GET https://x.test",
        "api_call POST https://x.test payload={} input_file=in.json",
        "download https://x.test/a.zip mime=application/zip output_dir=out",
        "mock",
        "mock fail",
    ];
    for statement in statements {
        let actor: Actor = statement.parse().expect(statement);
        assert_eq!(actor.to_string(), statement);
    }
}

#[test]
fn hides_auth_secrets_when_written_back() {
    let actor: Actor = "api_call GET https://x.test auth=bearer:secret"
        .parse()
        .unwrap();
    assert_eq!(
        actor.to_string(),
        "api_call GET https://x.test auth=bearer:***"
    );

    let others = [
        (
            Auth::Jwt {
                token: Passer::Data("secret".into()),
            },
            "jwt",
        ),
        (
            Auth::OAuth2 {
                token_url: "https://x.test/token".into(),
                client_id: Passer::Data("id".into()),
                client_secret: Passer::Data("secret".into()),
                scopes: Vec::new(),
            },
            "oauth2",
        ),
        (
            Auth::ApiKey {
                header: "X-API-Key".into(),
                key: Passer::Data("secret".into()),
            },
            "api_key",
        ),
    ];
    for (auth, kind) in others {
        let actor = Actor::DownloadFromUrl {
            url: "https://x.test/a.txt".into(),
            auth: Some(auth),
            mime: None,
            output_dir: None,
        };
        assert_eq!(
            actor.to_string(),
            format!("download https://x.test/a.txt auth={kind}:***")
        );
    }
}

#[test]
fn agent_websearch_writes_only_the_topic_when_options_are_empty() {
    let mut actor = web_search();
    assert_eq!(actor.to_string(), "agent_websearch rust");

    if let Actor::AgentWebSearch {
        prefer_websites, ..
    } = &mut actor
    {
        *prefer_websites = Some(Vec::new());
    }
    assert_eq!(actor.to_string(), "agent_websearch rust");
}

#[test]
fn agent_websearch_writes_every_option() {
    let actor = Actor::AgentWebSearch {
        research_topic: "rust".to_string(),
        prefer_websites: Some(vec!["a.com".to_string(), "b.org".to_string()]),
        prefer_models: vec!["model-1".to_string(), "model-2".to_string()],
        output_dir: Some("./output".to_string()),
        output_instructions: "summary".to_string(),
        structured_output: "{}".to_string(),
    };
    assert_eq!(
        actor.to_string(),
        "agent_websearch rust prefer_websites=a.com,b.org prefer_models=model-1,model-2 \
         output_dir=./output output_instructions=summary structured_output={}"
    );
}
