//! Tests for writing an actor back as an action statement, the `Display`
//! impl in `src/actor/display.rs`.

use atama::Passer;
use atama::actor::{Actor, AgentType, Auth};

#[test]
fn writes_actions_back_as_statements() {
    let statements = [
        "bash npm run build",
        "api_call GET https://x.test",
        "api_call POST https://x.test payload={} input_file=in.json",
        "download https://x.test/a.zip mime=application/zip output_dir=out",
        "mock",
        "mock fail",
        "agent:websearch",
        "agent:websearch rust release notes",
        "agent:coder fix.md",
        "tavily_search",
        "tavily_search topic=news rust release notes",
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
fn writes_agent_tool_with_its_type_and_params() {
    let cases = [
        (AgentType::WebSearch, "websearch"),
        (AgentType::Coder, "coder"),
    ];
    for (agent, name) in cases {
        assert_eq!(agent.to_string(), name);
        let actor = Actor::AgentTool {
            agent,
            params: vec!["a".into(), "b".into()],
        };
        assert_eq!(actor.to_string(), format!("agent:{name} a b"));
    }
}
