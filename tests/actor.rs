//! Tests for running actors.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use atama::Passer;
use atama::actor::{Actor, ApiVerb, Auth};

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
fn api_call_sends_input_as_body() {
    let (url, requests) = serve(vec![(200, "created")]);
    let actor = api_call(ApiVerb::POST, &format!("{url}/items"));

    assert_eq!(actor.act(r#"{"name":"atama"}"#), Ok("created".into()));
    let request = requests.recv().unwrap();
    assert!(
        request.starts_with("POST /items "),
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
fn api_call_fails_on_error_status() {
    let (url, _requests) = serve(vec![(404, "no such item")]);
    let actor = api_call(ApiVerb::GET, &url);

    let err = actor.act("").unwrap_err();
    assert!(err.contains("404"), "error was: {err}");
    assert!(err.contains("no such item"), "error was: {err}");
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
fn api_call_get_ignores_payload() {
    let (url, requests) = serve(vec![(200, "ok")]);
    let actor = Actor::ApiCall {
        verb: ApiVerb::GET,
        url,
        input_file: None,
        payload: Some("not sent".into()),
        auth: None,
    };

    actor.act("").unwrap();
    let request = requests.recv().unwrap();
    assert!(!request.contains("not sent"), "request was:\n{request}");
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
    let actor = with_auth(
        ApiVerb::GET,
        &closed_url(),
        Auth::Bearer {
            token: Passer::FromEnvVar("ATAMA_TEST_TOKEN_THAT_IS_NOT_SET".into()),
        },
    );

    let err = actor.act("").unwrap_err();
    assert!(
        err.contains("ATAMA_TEST_TOKEN_THAT_IS_NOT_SET"),
        "error was: {err}"
    );
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
