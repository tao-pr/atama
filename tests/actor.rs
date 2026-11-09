//! Tests for running actors.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use atama::Passer;
use atama::actor::{Actor, AgentType, ApiVerb, Auth};
use atama::profile::ModelProfile;

/// Starts a local HTTP server that answers one request per response, in order.
///
/// Returns the server's base URL and a receiver that gets each raw request.
fn serve<B: Into<Vec<u8>>>(responses: Vec<(u16, B)>) -> (String, Receiver<String>) {
    let responses: Vec<(u16, Vec<u8>)> = responses
        .into_iter()
        .map(|(status, body)| (status, body.into()))
        .collect();
    let listener = TcpListener::bind("127.0.0.1:0").expect("cannot bind test server");
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (requests, received) = mpsc::channel();

    thread::spawn(move || {
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().expect("cannot accept connection");
            let mut reader = BufReader::new(stream.try_clone().unwrap());

            let mut request = String::new();
            let mut content_length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                    content_length = value.trim().parse().unwrap();
                }
                request.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut request_body = vec![0; content_length];
            reader.read_exact(&mut request_body).unwrap();
            request.push_str(&String::from_utf8(request_body).unwrap());
            requests.send(request).unwrap();

            write!(
                stream,
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        }
    });

    (url, received)
}

fn bash(cmd: &str, params: &[&str]) -> Actor {
    Actor::BashScript {
        cmd: cmd.into(),
        params: params.iter().map(|param| param.to_string()).collect(),
    }
}

/// Verbs that send a request body, with their HTTP method names.
const BODY_VERBS: [(ApiVerb, &str); 3] = [
    (ApiVerb::POST, "POST"),
    (ApiVerb::PUT, "PUT"),
    (ApiVerb::PATCH, "PATCH"),
];

/// Verbs that send no request body, with their HTTP method names.
const NO_BODY_VERBS: [(ApiVerb, &str); 3] = [
    (ApiVerb::GET, "GET"),
    (ApiVerb::DELETE, "DELETE"),
    (ApiVerb::HEAD, "HEAD"),
];

/// A URL where nothing listens, so connections to it are refused.
fn closed_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    format!("http://{}", listener.local_addr().unwrap())
}

/// A fresh, empty directory under Cargo's temporary directory for tests.
fn temp_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn with_auth(verb: ApiVerb, url: &str, auth: Auth) -> Actor {
    Actor::ApiCall {
        verb,
        url: url.into(),
        input_file: None,
        payload: None,
        auth: Some(auth),
    }
}

fn api_call(verb: ApiVerb, url: &str) -> Actor {
    Actor::ApiCall {
        verb,
        url: url.into(),
        input_file: None,
        payload: None,
        auth: None,
    }
}

#[test]
fn bash_passes_params_as_arguments() {
    let actor = bash("echo", &["hello", "world"]);

    assert_eq!(actor.act(""), Ok("hello world\n".into()));
}

#[test]
fn bash_keeps_params_with_spaces_as_one_argument() {
    let actor = bash("printf '%s|'", &["a b", "c"]);

    assert_eq!(actor.act(""), Ok("a b|c|".into()));
}

#[test]
fn bash_gets_input_on_stdin() {
    let actor = bash("tr a-z A-Z", &[]);

    assert_eq!(actor.act("previous output"), Ok("PREVIOUS OUTPUT".into()));
}

#[test]
fn bash_fails_on_non_zero_exit() {
    let actor = bash("echo broken >&2; exit 4", &[]);

    let err = actor.act("").unwrap_err();
    assert!(err.contains("broken"), "error was: {err}");
}

#[test]
fn api_call_sends_input_as_body_for_verbs_with_a_body() {
    for (verb, method) in BODY_VERBS {
        let (url, requests) = serve(vec![(200, "created")]);
        let actor = api_call(verb, &format!("{url}/items"));

        assert_eq!(
            actor.act(r#"{"name":"atama"}"#),
            Ok("created".into()),
            "verb: {method}"
        );
        let request = requests.recv().unwrap();
        assert!(
            request.starts_with(&format!("{method} /items ")),
            "request was:\n{request}"
        );
        assert!(
            request
                .to_lowercase()
                .contains("content-type: application/json"),
            "request was:\n{request}"
        );
        assert!(
            request.ends_with(r#"{"name":"atama"}"#),
            "request was:\n{request}"
        );
    }
}

#[test]
fn api_call_prefers_payload_over_input() {
    let (url, requests) = serve(vec![(200, "ok")]);
    let actor = Actor::ApiCall {
        verb: ApiVerb::PUT,
        url: url.clone(),
        input_file: None,
        payload: Some("from payload".into()),
        auth: None,
    };

    actor.act("from input").unwrap();
    let request = requests.recv().unwrap();
    assert!(request.starts_with("PUT / "), "request was:\n{request}");
    assert!(
        request.ends_with("\r\n\r\nfrom payload"),
        "request was:\n{request}"
    );
}

#[test]
fn api_call_reads_body_from_input_file() {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("actor_body.txt");
    std::fs::write(&path, "from file").unwrap();
    let (url, requests) = serve(vec![(200, "ok")]);
    let actor = Actor::ApiCall {
        verb: ApiVerb::POST,
        url,
        input_file: Some(path.display().to_string()),
        payload: None,
        auth: None,
    };

    actor.act("from input").unwrap();
    let request = requests.recv().unwrap();
    assert!(
        request.ends_with("\r\n\r\nfrom file"),
        "request was:\n{request}"
    );
}

#[test]
fn api_call_get_sends_bearer_token_and_no_body() {
    let (url, requests) = serve(vec![(200, "[1, 2]")]);
    let actor = Actor::ApiCall {
        verb: ApiVerb::GET,
        url,
        input_file: None,
        payload: None,
        auth: Some(Auth::Bearer {
            token: Passer::Data("secret-token".into()),
        }),
    };

    assert_eq!(actor.act("ignored input"), Ok("[1, 2]".into()));
    let request = requests.recv().unwrap();
    assert!(request.starts_with("GET / "), "request was:\n{request}");
    assert!(
        request
            .to_lowercase()
            .contains("authorization: bearer secret-token"),
        "request was:\n{request}"
    );
    assert!(request.ends_with("\r\n\r\n"), "request was:\n{request}");
}

#[test]
fn api_call_sends_api_key_header() {
    let (url, requests) = serve(vec![(200, "ok")]);
    let actor = Actor::ApiCall {
        verb: ApiVerb::DELETE,
        url,
        input_file: None,
        payload: None,
        auth: Some(Auth::ApiKey {
            header: "X-API-Key".into(),
            key: Passer::Data("key-123".into()),
        }),
    };

    actor.act("").unwrap();
    let request = requests.recv().unwrap();
    assert!(request.starts_with("DELETE / "), "request was:\n{request}");
    assert!(
        request.to_lowercase().contains("x-api-key: key-123"),
        "request was:\n{request}"
    );
}

#[test]
fn api_call_gets_oauth2_token_first() {
    let (url, requests) = serve(vec![
        (
            200,
            r#"{"access_token":"oauth-token","token_type":"Bearer"}"#,
        ),
        (200, "data"),
    ]);
    let actor = Actor::ApiCall {
        verb: ApiVerb::GET,
        url: format!("{url}/data"),
        input_file: None,
        payload: None,
        auth: Some(Auth::OAuth2 {
            token_url: format!("{url}/token"),
            client_id: Passer::Data("my-client".into()),
            client_secret: Passer::Data("my-secret".into()),
            scopes: vec!["read".into(), "write".into()],
        }),
    };

    assert_eq!(actor.act(""), Ok("data".into()));
    let token_request = requests.recv().unwrap();
    assert!(
        token_request.starts_with("POST /token "),
        "request was:\n{token_request}"
    );
    assert!(
        token_request.contains("grant_type=client_credentials"),
        "request was:\n{token_request}"
    );
    assert!(
        token_request.contains("client_id=my-client"),
        "request was:\n{token_request}"
    );
    let data_request = requests.recv().unwrap();
    assert!(
        data_request
            .to_lowercase()
            .contains("authorization: bearer oauth-token"),
        "request was:\n{data_request}"
    );
}

#[test]
fn api_call_fails_on_error_status_for_every_verb() {
    for (verb, method) in BODY_VERBS.into_iter().chain(NO_BODY_VERBS) {
        // A response to `HEAD` has no body, so there is no text to report.
        let body = if method == "HEAD" { "" } else { "no such item" };
        let (url, _requests) = serve(vec![(404, body)]);
        let actor = api_call(verb, &url);

        let err = actor.act("").unwrap_err();
        assert!(err.contains("404"), "verb: {method}, error was: {err}");
        assert!(err.contains(body), "verb: {method}, error was: {err}");
    }
}

#[test]
fn download_saves_file_and_returns_its_path() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("downloads");
    let (url, requests) = serve(vec![(200, "a,b\n1,2\n")]);
    let actor = Actor::DownloadFromUrl {
        url: format!("{url}/files/report.csv?version=2"),
        auth: None,
        mime: Some("text/csv".into()),
        output_dir: Some(dir.display().to_string()),
    };

    let path = actor.act("ignored input").unwrap();
    assert_eq!(PathBuf::from(&path), dir.join("report.csv"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "a,b\n1,2\n");
    let request = requests.recv().unwrap();
    assert!(
        request.starts_with("GET /files/report.csv?version=2 "),
        "request was:\n{request}"
    );
    assert!(
        request.to_lowercase().contains("accept: text/csv"),
        "request was:\n{request}"
    );
}

#[test]
fn download_uses_temp_dir_and_default_name() {
    let (url, _requests) = serve(vec![(200, "data")]);
    let actor = Actor::DownloadFromUrl {
        url: format!("{url}/"),
        auth: None,
        mime: None,
        output_dir: None,
    };

    let path = actor.act("").unwrap();
    assert_eq!(
        PathBuf::from(&path),
        std::env::temp_dir().join("atama").join("download")
    );
}

#[test]
fn download_fails_on_error_status() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("failed_downloads");
    let (url, _requests) = serve(vec![(500, "server broke")]);
    let actor = Actor::DownloadFromUrl {
        url: format!("{url}/never.bin"),
        auth: None,
        mime: None,
        output_dir: Some(dir.display().to_string()),
    };

    let err = actor.act("").unwrap_err();
    assert!(err.contains("server broke"), "error was: {err}");
    assert!(!dir.join("never.bin").exists());
}

// BashScript: more cases.

#[test]
fn bash_runs_cmd_as_a_script() {
    let actor = bash("printf 'b\\na\\n' | sort", &[]);

    assert_eq!(actor.act(""), Ok("a\nb\n".into()));
}

#[test]
fn bash_without_params_ends_a_heredoc_on_the_last_line() {
    let actor = bash("cat <<'EOF'\nline 1\nline 2\nEOF", &[]);

    assert_eq!(actor.act(""), Ok("line 1\nline 2\n".into()));
}

#[test]
fn bash_does_not_quote_special_characters_in_params() {
    let actor = bash("printf '%s\\n'", &["$HOME", "*", "a;b"]);

    assert_eq!(actor.act(""), Ok("$HOME\n*\na;b\n".into()));
}

#[test]
fn bash_ignores_large_input_it_does_not_read() {
    let actor = bash("echo done", &[]);
    let input = "x".repeat(1024 * 1024);

    assert_eq!(actor.act(&input), Ok("done\n".into()));
}

#[test]
fn bash_does_not_block_when_it_prints_a_lot_before_reading_input() {
    let actor = bash(
        "head -c 1000000 /dev/zero | tr '\\0' a; wc -c | tr -d ' '",
        &[],
    );
    let input = "y".repeat(1024 * 1024);

    let output = actor.act(&input).unwrap();
    assert_eq!(output.len(), 1_000_000 + "1048576\n".len());
    assert!(output.ends_with("1048576\n"));
}

#[test]
fn bash_error_includes_exit_status() {
    let actor = bash("exit 7", &[]);

    let err = actor.act("").unwrap_err();
    assert!(err.contains('7'), "error was: {err}");
}

#[test]
fn bash_fails_on_output_that_is_not_utf8() {
    let actor = bash("printf '\\377'", &[]);

    let err = actor.act("").unwrap_err();
    assert!(err.contains("not UTF-8"), "error was: {err}");
}

#[test]
fn bash_fails_on_unknown_command() {
    let actor = bash("atama-command-that-does-not-exist", &[]);

    assert!(actor.act("").is_err());
}

// ApiCall: more cases.

#[test]
fn api_call_sends_text_that_is_not_json_as_plain_text() {
    let (url, requests) = serve(vec![(200, "ok")]);
    let actor = api_call(ApiVerb::POST, &url);

    actor.act("hello {not json").unwrap();
    let request = requests.recv().unwrap().to_lowercase();
    assert!(
        request.contains("content-type: text/plain; charset=utf-8"),
        "request was:\n{request}"
    );
}

#[test]
fn api_call_prefers_payload_over_input_file() {
    let (url, requests) = serve(vec![(200, "ok")]);
    let actor = Actor::ApiCall {
        verb: ApiVerb::POST,
        url,
        input_file: Some("does/not/exist.txt".into()),
        payload: Some("from payload".into()),
        auth: None,
    };

    actor.act("from input").unwrap();
    let request = requests.recv().unwrap();
    assert!(
        request.ends_with("\r\n\r\nfrom payload"),
        "request was:\n{request}"
    );
}

#[test]
fn api_call_fails_on_missing_input_file() {
    let actor = Actor::ApiCall {
        verb: ApiVerb::POST,
        url: closed_url(),
        input_file: Some("does/not/exist.txt".into()),
        payload: None,
        auth: None,
    };

    let err = actor.act("").unwrap_err();
    assert!(err.contains("does/not/exist.txt"), "error was: {err}");
}

#[test]
fn api_call_sends_no_body_for_verbs_without_a_body() {
    for (verb, method) in NO_BODY_VERBS {
        let (url, requests) = serve(vec![(200, "")]);
        let actor = Actor::ApiCall {
            verb,
            url,
            input_file: None,
            payload: Some("not sent".into()),
            auth: None,
        };

        actor.act("ignored input").unwrap();
        let request = requests.recv().unwrap();
        assert!(
            request.starts_with(&format!("{method} / ")),
            "request was:\n{request}"
        );
        assert!(request.ends_with("\r\n\r\n"), "request was:\n{request}");
        assert!(
            !request.to_lowercase().contains("content-length"),
            "request was:\n{request}"
        );
    }
}

#[test]
fn api_call_sends_jwt_as_bearer_token() {
    let (url, requests) = serve(vec![(200, "ok")]);
    let actor = with_auth(
        ApiVerb::GET,
        &url,
        Auth::Jwt {
            token: Passer::Data("aaa.bbb.ccc".into()),
        },
    );

    actor.act("").unwrap();
    let request = requests.recv().unwrap().to_lowercase();
    assert!(
        request.contains("authorization: bearer aaa.bbb.ccc"),
        "request was:\n{request}"
    );
}

#[test]
fn api_call_fails_before_sending_when_auth_value_is_missing() {
    let missing = || Passer::FromEnvVar("ATAMA_TEST_TOKEN_THAT_IS_NOT_SET".into());
    let given = || Passer::Data("value".into());
    let auths = [
        Auth::Bearer { token: missing() },
        Auth::Jwt { token: missing() },
        Auth::ApiKey {
            header: "X-API-Key".into(),
            key: missing(),
        },
        Auth::OAuth2 {
            token_url: closed_url(),
            client_id: missing(),
            client_secret: given(),
            scopes: Vec::new(),
        },
        Auth::OAuth2 {
            token_url: closed_url(),
            client_id: given(),
            client_secret: missing(),
            scopes: Vec::new(),
        },
    ];
    // The URL is closed, so an error about the variable proves that no
    // request was sent.
    for (verb, method) in BODY_VERBS.into_iter().chain(NO_BODY_VERBS) {
        for auth in auths.clone() {
            let err = with_auth(verb.clone(), &closed_url(), auth.clone())
                .act("")
                .unwrap_err();
            assert!(
                err.contains("ATAMA_TEST_TOKEN_THAT_IS_NOT_SET"),
                "{method} with {auth:?}: error was: {err}"
            );
        }
    }
}

#[test]
fn api_call_oauth2_sends_scopes_space_separated() {
    let (url, requests) = serve(vec![(200, r#"{"access_token":"t"}"#), (200, "ok")]);
    let actor = with_auth(
        ApiVerb::GET,
        &url,
        Auth::OAuth2 {
            token_url: format!("{url}/token"),
            client_id: Passer::Data("id".into()),
            client_secret: Passer::Data("shh".into()),
            scopes: vec!["read".into(), "write".into()],
        },
    );

    actor.act("").unwrap();
    let token_request = requests.recv().unwrap();
    assert!(
        token_request.contains("client_secret=shh"),
        "request was:\n{token_request}"
    );
    assert!(
        token_request.contains("scope=read+write") || token_request.contains("scope=read%20write"),
        "request was:\n{token_request}"
    );
}

#[test]
fn api_call_oauth2_leaves_out_scope_when_empty() {
    let (url, requests) = serve(vec![(200, r#"{"access_token":"t"}"#), (200, "ok")]);
    let actor = with_auth(
        ApiVerb::GET,
        &url,
        Auth::OAuth2 {
            token_url: format!("{url}/token"),
            client_id: Passer::Data("id".into()),
            client_secret: Passer::Data("shh".into()),
            scopes: vec![],
        },
    );

    actor.act("").unwrap();
    let token_request = requests.recv().unwrap();
    assert!(
        !token_request.contains("scope="),
        "request was:\n{token_request}"
    );
}

#[test]
fn api_call_oauth2_fails_without_access_token() {
    let (url, _requests) = serve(vec![(200, r#"{"error":"nope"}"#)]);
    let actor = with_auth(
        ApiVerb::GET,
        &url,
        Auth::OAuth2 {
            token_url: format!("{url}/token"),
            client_id: Passer::Data("id".into()),
            client_secret: Passer::Data("shh".into()),
            scopes: vec![],
        },
    );

    let err = actor.act("").unwrap_err();
    assert!(err.contains("access_token"), "error was: {err}");
}

#[test]
fn api_call_oauth2_fails_on_token_response_that_is_not_json() {
    let (url, _requests) = serve(vec![(200, "<html>login</html>")]);
    let actor = with_auth(
        ApiVerb::GET,
        &url,
        Auth::OAuth2 {
            token_url: format!("{url}/token"),
            client_id: Passer::Data("id".into()),
            client_secret: Passer::Data("shh".into()),
            scopes: vec![],
        },
    );

    let err = actor.act("").unwrap_err();
    assert!(err.contains("not JSON"), "error was: {err}");
}

#[test]
fn api_call_oauth2_fails_on_token_error_status() {
    let (url, _requests) = serve(vec![(401, "bad client")]);
    let actor = with_auth(
        ApiVerb::GET,
        &url,
        Auth::OAuth2 {
            token_url: format!("{url}/token"),
            client_id: Passer::Data("id".into()),
            client_secret: Passer::Data("wrong".into()),
            scopes: vec![],
        },
    );

    let err = actor.act("").unwrap_err();
    assert!(err.contains("401"), "error was: {err}");
    assert!(err.contains("bad client"), "error was: {err}");
}

#[test]
fn api_call_fails_when_server_is_unreachable() {
    let url = closed_url();
    let actor = api_call(ApiVerb::GET, &url);

    let err = actor.act("").unwrap_err();
    assert!(err.contains(&url), "error was: {err}");
}

// DownloadFromUrl: more cases.

#[test]
fn download_sends_auth_header() {
    let dir = temp_dir("download_auth");
    let (url, requests) = serve(vec![(200, "data")]);
    let actor = Actor::DownloadFromUrl {
        url: format!("{url}/private.txt"),
        auth: Some(Auth::ApiKey {
            header: "X-API-Key".into(),
            key: Passer::Data("key-123".into()),
        }),
        mime: None,
        output_dir: Some(dir.display().to_string()),
    };

    actor.act("").unwrap();
    let request = requests.recv().unwrap().to_lowercase();
    assert!(
        request.contains("x-api-key: key-123"),
        "request was:\n{request}"
    );
}

#[test]
fn download_keeps_binary_content() {
    let dir = temp_dir("download_binary");
    let bytes: Vec<u8> = (0..=255).collect();
    let (url, _requests) = serve(vec![(200, bytes.clone())]);
    let actor = Actor::DownloadFromUrl {
        url: format!("{url}/image.png"),
        auth: None,
        mime: Some("image/png".into()),
        output_dir: Some(dir.display().to_string()),
    };

    let path = actor.act("").unwrap();
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn download_saves_files_larger_than_10_mb() {
    let dir = temp_dir("download_large");
    let size = 11 * 1024 * 1024;
    let (url, _requests) = serve(vec![(200, vec![b'z'; size])]);
    let actor = Actor::DownloadFromUrl {
        url: format!("{url}/large.bin"),
        auth: None,
        mime: None,
        output_dir: Some(dir.display().to_string()),
    };

    let path = actor.act("").unwrap();
    assert_eq!(std::fs::metadata(path).unwrap().len(), size as u64);
}

#[test]
fn download_creates_missing_output_dir() {
    let dir = temp_dir("download_nested").join("a").join("b");
    let (url, _requests) = serve(vec![(200, "data")]);
    let actor = Actor::DownloadFromUrl {
        url: format!("{url}/file.txt"),
        auth: None,
        mime: None,
        output_dir: Some(dir.display().to_string()),
    };

    let path = actor.act("").unwrap();
    assert_eq!(PathBuf::from(path), dir.join("file.txt"));
}

#[test]
fn download_overwrites_file_with_same_name() {
    let dir = temp_dir("download_overwrite");
    let (url, _requests) = serve(vec![(200, "first version"), (200, "second")]);
    let actor = Actor::DownloadFromUrl {
        url: format!("{url}/notes.txt"),
        auth: None,
        mime: None,
        output_dir: Some(dir.display().to_string()),
    };

    actor.act("").unwrap();
    let path = actor.act("").unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), "second");
}

#[test]
fn download_uses_default_name_for_unsafe_or_missing_names() {
    let dir = temp_dir("download_names");
    for path in ["", "/", "/a/..", "/a/.?x=1"] {
        let (url, _requests) = serve(vec![(200, "data")]);
        let actor = Actor::DownloadFromUrl {
            url: format!("{url}{path}"),
            auth: None,
            mime: None,
            output_dir: Some(dir.display().to_string()),
        };

        let saved = actor.act("").unwrap();
        assert_eq!(PathBuf::from(saved), dir.join("download"), "path: {path:?}");
    }
}

#[test]
fn download_fails_when_auth_value_is_missing() {
    let actor = Actor::DownloadFromUrl {
        url: closed_url(),
        auth: Some(Auth::Bearer {
            token: Passer::FromFile("does/not/exist.token".into()),
        }),
        mime: None,
        output_dir: None,
    };

    let err = actor.act("").unwrap_err();
    assert!(err.contains("does/not/exist.token"), "error was: {err}");
}

#[test]
fn api_call_head_sends_no_body_and_outputs_nothing() {
    let (url, requests) = serve(vec![(200, "")]);
    let actor = Actor::ApiCall {
        verb: ApiVerb::HEAD,
        url: format!("{url}/items"),
        input_file: None,
        payload: Some("not sent".into()),
        auth: Some(Auth::Bearer {
            token: Passer::Data("head-token".into()),
        }),
    };

    assert_eq!(actor.act("ignored input"), Ok(String::new()));
    let request = requests.recv().unwrap();
    assert!(
        request.starts_with("HEAD /items "),
        "request was:\n{request}"
    );
    assert!(
        request
            .to_lowercase()
            .contains("authorization: bearer head-token"),
        "request was:\n{request}"
    );
    assert!(request.ends_with("\r\n\r\n"), "request was:\n{request}");
}

#[test]
fn mock_placeholder_passes_input_through() {
    assert_eq!(
        Actor::MockPlaceholder.act("from before\n").unwrap(),
        "from before\n"
    );
}

#[test]
fn mock_placeholder_passes_empty_input_through() {
    assert_eq!(Actor::MockPlaceholder.act("").unwrap(), "");
}

#[test]
fn mock_fail_always_fails() {
    for input in ["", "some input"] {
        assert_eq!(Actor::MockFail.act(input).unwrap_err(), "mock failure");
    }
}

#[test]
fn mock_actors_chain_like_real_actors() {
    let output = Actor::MockPlaceholder.act("data").unwrap();
    let output = bash("tr a-z A-Z", &[]).act(&output).unwrap();
    assert_eq!(Actor::MockPlaceholder.act(&output).unwrap(), "DATA");
}

fn download_to(url: &str, output_dir: &std::path::Path) -> Actor {
    Actor::DownloadFromUrl {
        url: url.into(),
        auth: None,
        mime: None,
        output_dir: Some(output_dir.display().to_string()),
    }
}

#[test]
fn download_fails_when_server_is_unreachable() {
    let dir = temp_dir("download_unreachable");
    let err = download_to(&format!("{}/a.txt", closed_url()), &dir)
        .act("")
        .unwrap_err();

    assert!(err.contains("request to"), "error was: {err}");
    assert!(!dir.exists(), "no directory is made before the response");
}

#[test]
fn download_fails_when_output_dir_is_a_file() {
    let dir = temp_dir("download_output_dir_is_file");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("not_a_dir");
    std::fs::write(&file, "x").unwrap();
    let (url, _requests) = serve(vec![(200, "content")]);

    let err = download_to(&format!("{url}/a.txt"), &file)
        .act("")
        .unwrap_err();
    assert!(err.contains("cannot create directory"), "error was: {err}");
}

#[test]
fn download_fails_when_target_path_is_a_directory() {
    let dir = temp_dir("download_target_is_dir");
    std::fs::create_dir_all(dir.join("a.txt")).unwrap();
    let (url, _requests) = serve(vec![(200, "content")]);

    let err = download_to(&format!("{url}/a.txt"), &dir)
        .act("")
        .unwrap_err();
    assert!(err.contains("cannot create file"), "error was: {err}");
}

/// The model profile `p`: the model `test-model` on the server at
/// `base_url`, without an API key.
fn profile(base_url: &str) -> ModelProfile {
    ModelProfile {
        name: "p".into(),
        model: "test-model".into(),
        base_url: base_url.into(),
        api_key_env: None,
        token_budget: None,
    }
}

#[test]
fn agent_tool_runs_the_agent_loop_with_its_params_and_input() {
    let (url, requests) = serve(vec![(
        200,
        r#"{"choices":[{"message":{"role":"assistant","content":"done"}}]}"#,
    )]);
    let actor = Actor::AgentTool {
        agent: AgentType::Coder,
        params: vec!["model=p".into(), "task=find it".into()],
        profile: Some(profile(&url)),
        output_schema: None,
    };
    assert_eq!(actor.act("the input"), Ok("done".to_string()));
    let request = requests.recv().unwrap();
    assert!(request.starts_with("POST /chat/completions "), "{request}");
    assert!(
        request.contains("find it\\n\\nInput:\\nthe input"),
        "{request}"
    );
}

#[test]
fn act_with_debug_writes_agent_debug_output_only() {
    let url = closed_url();
    let actor = Actor::AgentTool {
        agent: AgentType::Coder,
        params: vec!["model=p".into(), "task=t".into()],
        profile: Some(profile(&url)),
        output_schema: None,
    };
    let mut out = Vec::new();
    assert!(actor.act_with("", Some(&mut out)).is_err());
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("=== agent:coder (p) configuration ==="), "{out}");
    assert!(out.contains("=== agent:coder (p) step 1: request"), "{out}");

    let mut out = Vec::new();
    assert_eq!(
        bash("echo", &["hi"]).act_with("", Some(&mut out)),
        Ok("hi\n".to_string())
    );
    assert!(out.is_empty());
}

#[test]
fn agent_tool_fails_when_the_model_server_is_unreachable() {
    let url = closed_url();
    let actor = Actor::AgentTool {
        agent: AgentType::Coder,
        params: vec!["model=p".into(), "task=t".into()],
        profile: Some(profile(&url)),
        output_schema: None,
    };
    let err = actor.act("").unwrap_err();
    assert!(
        err.starts_with(&format!("request to `{url}/chat/completions` failed")),
        "error was: {err}"
    );
}

#[test]
fn api_call_fails_on_response_that_is_not_utf8() {
    let (url, _requests) = serve(vec![(200, vec![0xff, 0xfe])]);
    let err = api_call(ApiVerb::GET, &url).act("").unwrap_err();
    assert!(
        err.starts_with(&format!("cannot read response from `{url}`")),
        "error was: {err}"
    );
}

#[test]
fn download_fails_when_the_connection_closes_early() {
    // The server promises 100 bytes, sends 5, and closes the connection.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/file.txt", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        while line != "\r\n" {
            line.clear();
            reader.read_line(&mut line).unwrap();
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort")
            .unwrap();
    });

    let dir = temp_dir("download_closes_early");
    let err = Actor::DownloadFromUrl {
        url: url.clone(),
        auth: None,
        mime: None,
        output_dir: Some(dir.display().to_string()),
    }
    .act("")
    .unwrap_err();
    server.join().unwrap();
    assert!(
        err.starts_with(&format!("cannot save `{url}` to `")),
        "error was: {err}"
    );
}

#[test]
fn tavily_search_fails_on_invalid_params() {
    // The test with a real request is in `tests/cli.rs`, because it needs
    // `TAVILY_API_KEY` and only a child process can set it safely.
    let actor = Actor::TavilySearch {
        params: vec!["max_results=x".into(), "q".into()],
    };
    assert_eq!(actor.act(""), Err("`max_results=x` is not a number".into()));
}

// Foreach

fn for_each(action: Actor) -> Actor {
    Actor::ForEach(Box::new(action))
}

#[test]
fn foreach_runs_the_action_once_for_each_line_and_joins_the_outputs() {
    // `tr` gets one line without its newline, so its output has none. Each
    // output gets a newline, so each item starts on a new line.
    let actor = for_each(bash("tr", &["a-z", "A-Z"]));
    assert_eq!(actor.act("apple\nbanana\n"), Ok("APPLE\nBANANA\n".into()));

    // An output that ends with a newline gets no second one. Blank lines
    // are not items.
    let actor = for_each(bash("awk", &["{print\"item:\",$0}"]));
    assert_eq!(
        actor.act("a\n\n   \nb c"),
        Ok("item: a\nitem: b c\n".into())
    );
}

#[test]
fn foreach_runs_nothing_for_an_input_without_items() {
    // `mock fail` would fail if it ran.
    let actor = for_each(Actor::MockFail);
    assert_eq!(actor.act(""), Ok(String::new()));
    assert_eq!(actor.act("\n \n"), Ok(String::new()));
}

#[test]
fn foreach_fails_at_the_first_item_that_fails() {
    let actor = for_each(bash("grep", &["a"]));
    let err = actor.act("a\nb\nc").unwrap_err();
    assert!(
        err.starts_with("`foreach` failed at item 2 (`b`): "),
        "error was: {err}"
    );
}

#[test]
fn foreach_gives_the_debug_output_to_each_run() {
    let url = closed_url();
    let actor = for_each(Actor::AgentTool {
        agent: AgentType::Coder,
        params: vec!["model=p".into()],
        profile: Some(profile(&url)),
        output_schema: None,
    });
    let mut out = Vec::new();
    assert!(actor.act_with("one\ntwo", Some(&mut out)).is_err());
    let out = String::from_utf8(out).unwrap();
    // The first run fails, so only it writes debug output.
    assert_eq!(
        out.matches("=== agent:coder (p) configuration ===").count(),
        1,
        "{out}"
    );
    assert!(out.contains("\"content\": \"one\""), "{out}");
}

// read_file

/// Writes `content` to `<name>` in the fresh directory `temp_dir(dir)`, and
/// returns the path. Each call deletes the earlier files in `dir`.
fn write_temp(dir: &str, name: &str, content: &[u8]) -> String {
    let dir = temp_dir(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn read_file_returns_the_content_with_all_its_lines() {
    let content = "first line\n\n  indented line\r\nlast line without newline";
    let path = write_temp("read_file_lines", "notes.txt", content.as_bytes());
    let actor = Actor::ReadFile {
        path: Some(path.clone()),
    };
    // The path of the action wins over the input.
    assert_eq!(actor.act("ignored input"), Ok(content.to_string()));

    let content = "a\nb\n";
    let path = write_temp("read_file_newline", "with newline.txt", content.as_bytes());
    let actor = Actor::ReadFile { path: Some(path) };
    assert_eq!(actor.act(""), Ok(content.to_string()));
}

#[test]
fn read_file_reads_the_path_from_the_input() {
    let path = write_temp("read_file_input", "a.txt", b"one\ntwo\n");
    let actor = Actor::ReadFile { path: None };
    // The newline at the end of the previous output is not part of the path.
    assert_eq!(
        actor.act(&format!("  {path}\n")),
        Ok("one\ntwo\n".to_string())
    );

    // `foreach` reads one file for each line.
    let other = write_temp("read_file_input_other", "b.txt", b"three\n");
    let actor: Actor = "foreach read_file".parse().unwrap();
    assert_eq!(
        actor.act(&format!("{path}\n{other}\n")),
        Ok("one\ntwo\nthree\n".to_string())
    );
}

#[test]
fn read_file_fails_without_a_single_readable_path() {
    let actor = Actor::ReadFile { path: None };
    for input in ["", " \n "] {
        assert_eq!(
            actor.act(input),
            Err("`read_file` needs a path in its params or an input".to_string()),
            "{input:?}"
        );
    }
    assert_eq!(
        actor.act("a.txt\nb.txt\n"),
        Err(
            "`read_file` reads one path, but its input has 2 lines. To read one file for \
             each line, use `foreach read_file`"
                .to_string()
        )
    );

    let missing = temp_dir("read_file_missing").join("no-such-file.txt");
    let missing = missing.to_string_lossy();
    let err = actor.act(&missing).unwrap_err();
    assert!(
        err.starts_with(&format!("cannot read file `{missing}`: ")),
        "{err}"
    );

    let path = write_temp("read_file_binary", "data.bin", &[0xff, 0xfe, 0x00]);
    let err = Actor::ReadFile { path: Some(path) }.act("").unwrap_err();
    assert!(err.starts_with("cannot read file `"), "{err}");
}

// write_file

#[test]
fn write_file_writes_the_input_and_passes_it_on() {
    let dir = temp_dir("write_file");
    // The directories of the path do not exist yet.
    let path = dir.join("a").join("b").join("out.txt");
    let actor = Actor::WriteFile {
        path: path.to_string_lossy().into_owned(),
    };
    let input = "first\n\n  indented\r\nlast without newline";
    assert_eq!(actor.act(input), Ok(input.to_string()));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), input);

    // A second write replaces the file.
    assert_eq!(actor.act(""), Ok(String::new()));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
}

#[test]
fn write_file_writes_a_file_in_the_current_directory() {
    // A relative path. The test runs in the package root, so it writes to
    // target/.
    let name = format!("target/atama-write-file-{}.txt", std::process::id());
    let actor = Actor::WriteFile { path: name.clone() };
    assert_eq!(actor.act("x"), Ok("x".to_string()));
    assert_eq!(std::fs::read_to_string(&name).unwrap(), "x");
    std::fs::remove_file(&name).unwrap();
}

#[test]
fn write_file_fails_if_it_cannot_write() {
    let dir = temp_dir("write_file_fails");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("file.txt");
    std::fs::write(&file, "x").unwrap();

    // The parent of the path is a file, not a directory.
    let path = file.join("out.txt");
    let err = Actor::WriteFile {
        path: path.to_string_lossy().into_owned(),
    }
    .act("y")
    .unwrap_err();
    assert!(
        err.starts_with(&format!(
            "cannot make directory `{}` for file `{}`: ",
            file.display(),
            path.display()
        )),
        "{err}"
    );

    // The path is a directory. `/` also has no parent.
    for path in [dir.to_string_lossy().into_owned(), "/".to_string()] {
        let err = Actor::WriteFile { path: path.clone() }
            .act("y")
            .unwrap_err();
        assert!(
            err.starts_with(&format!("cannot write file `{path}`: ")),
            "{err}"
        );
    }
}

// bash-sandbox

fn sandbox(cmd: &str, params: &[&str]) -> Actor {
    Actor::BashSandbox {
        cmd: cmd.into(),
        params: params.iter().map(|param| param.to_string()).collect(),
    }
}

#[cfg(target_os = "macos")]
#[test]
fn bash_sandbox_runs_a_command_with_the_input_and_the_params() {
    assert_eq!(
        sandbox("tr", &["a-z", "A-Z"]).act("abc\n"),
        Ok("ABC\n".to_string())
    );
    assert_eq!(
        sandbox("printenv", &["PATH"]).act(""),
        Ok(format!("{}\n", atama::actor::SANDBOX_PATH))
    );
    let err = sandbox("false", &[]).act("").unwrap_err();
    assert!(
        err.starts_with("`false` failed with exit status: 1"),
        "{err}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn bash_sandbox_without_params_ends_a_heredoc_on_the_last_line() {
    assert_eq!(
        sandbox("cat <<'EOF'\nline 1\nEOF", &[]).act(""),
        Ok("line 1\n".to_string())
    );
}

#[cfg(target_os = "macos")]
#[test]
fn bash_sandbox_reads_and_writes_in_the_working_directory() {
    // The tests run in the package root, so `target/` is in the working
    // directory.
    let dir = format!("target/bash-sandbox-{}", std::process::id());
    let path = format!("{dir}/out.txt");
    assert_eq!(sandbox("mkdir", &["-p", &dir]).act(""), Ok(String::new()));
    assert_eq!(
        sandbox("tee", &[&path]).act("data\n"),
        Ok("data\n".to_string())
    );
    assert_eq!(sandbox("cat", &[&path]).act(""), Ok("data\n".to_string()));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn bash_sandbox_denies_files_outside_the_working_directory() {
    // `temp_dir` is in `target/`, so this test uses the temp directory of
    // the system.
    let outside = std::env::temp_dir().join(format!("atama-sandbox-{}", std::process::id()));
    let outside = outside.to_string_lossy().into_owned();
    std::fs::create_dir_all(&outside).unwrap();
    let file = format!("{outside}/out.txt");
    let err = sandbox("tee", &[&file]).act("data").unwrap_err();
    assert!(err.contains("Operation not permitted"), "{err}");
    assert!(!std::path::Path::new(&file).exists());
    std::fs::remove_dir_all(&outside).unwrap();

    // The home directory has secrets, such as SSH keys.
    let home = std::env::var("HOME").unwrap();
    let err = sandbox("ls", &[&home]).act("").unwrap_err();
    assert!(err.contains("Operation not permitted"), "{err}");
}

#[cfg(target_os = "macos")]
#[test]
fn bash_sandbox_denies_the_network_and_sudo() {
    // A local server that the command cannot reach.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    let err = sandbox("nc", &["-z", "-w", "2", "127.0.0.1", &port])
        .act("")
        .unwrap_err();
    assert!(err.starts_with("`nc` failed"), "{err}");
    listener.set_nonblocking(true).unwrap();
    assert!(listener.accept().is_err(), "the sandbox let `nc` connect");

    let err = sandbox("sudo", &["-n", "true"]).act("").unwrap_err();
    assert!(
        err.contains("/usr/bin/sudo: Operation not permitted"),
        "{err}"
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn bash_sandbox_fails_without_macos() {
    assert_eq!(
        sandbox("true", &[]).act(""),
        Err("`bash-sandbox` needs `sandbox-exec`, which only macOS has".to_string())
    );
}
