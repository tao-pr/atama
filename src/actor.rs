//! Actors: the implementations that carry out a task's actions.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;

use ureq::http::Response;
use ureq::{Agent, Body, RequestBuilder};

use crate::agent_loop_openai_compat::AgentToolLoop;
use crate::passer::Passer;

mod display;

/// One action of a task, and how to carry it out.
///
/// The parser makes an actor from each action statement in the job file. The
/// first word of the statement selects the variant (see the `FromStr` impl in
/// [`crate::parser`]), and [`std::fmt::Display`] writes the statement back
/// (see `src/actor/display.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    /// Runs a command in bash.
    /// Always carry stdout as output
    BashScript {
        /// The command or script to run.
        cmd: String,
        /// Arguments passed to the command, in order.
        params: Vec<String>,
    },
    /// Sends an HTTP request.
    /// ApiCall carries response body as output
    ApiCall {
        verb: ApiVerb,
        url: String,
        input_file: Option<String>,
        payload: Option<String>,
        auth: Option<Auth>,
    },
    /// Downloads a file.
    /// Download from URL carries downloaded file path as output
    DownloadFromUrl {
        url: String,
        auth: Option<Auth>,
        /// Specify MIME type of the output if aware of
        mime: Option<String>,
        /// Output dir it not specified, will write to temp dir
        output_dir: Option<String>,
    },
    /// Runs an agent of the given type.
    AgentTool {
        agent: AgentType,
        /// Parameters for the agent, in order. Each type of agent will
        /// define its own parameters.
        params: Vec<String>,
    },
    /// Does nothing. Carries its input as output unchanged.
    MockPlaceholder,
    /// Always fails, to emulate a failing action.
    MockFail,
}

/// Verbs https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Methods
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiVerb {
    GET,
    POST,
    PUT,
    DELETE,
    PATCH,
    HEAD,
}

/// The type of agent that an [`Actor::AgentTool`] runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentType {
    /// Searches the web.
    WebSearch,
    /// Writes code.
    Coder,
}

/// How an [`Actor::ApiCall`] authenticates with the API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Auth {
    /// Sends `Authorization: Bearer <token>`.
    Bearer {
        /// The opaque access token.
        token: Passer,
    },
    /// Sends a signed JSON Web Token as `Authorization: Bearer <token>`.
    Jwt {
        /// The encoded JWT (`header.payload.signature`).
        token: Passer,
    },
    /// OAuth 2.0 client credentials grant: fetches an access token from
    /// `token_url`, then sends it as a bearer token.
    OAuth2 {
        /// The authorization server's token endpoint.
        token_url: String,
        client_id: Passer,
        client_secret: Passer,
        /// Scopes to request. Empty to use the server's default scopes.
        scopes: Vec<String>,
    },
    /// Sends a static key in a request header.
    ApiKey {
        /// Name of the header that carries the key, e.g. `X-API-Key`.
        header: String,
        /// The key value.
        key: Passer,
    },
}

impl Actor {
    /// Carries out the action and returns its output.
    ///
    /// `input` is the output of the previous action in the task. For the
    /// first action of a task, it is the output of the previous task. It is
    /// empty when there is no previous output.
    ///
    /// - [`Actor::BashScript`] runs `cmd` in bash with `params` as its
    ///   arguments and `input` on stdin, and returns stdout.
    /// - [`Actor::ApiCall`] sends a request and returns the response body.
    ///   The request body is `payload`, else the contents of `input_file`,
    ///   else `input`. `GET`, `DELETE` and `HEAD` send no body. A `HEAD`
    ///   response has no body, so its output is empty.
    /// - [`Actor::DownloadFromUrl`] sends a `GET` request, saves the response
    ///   body to a file, and returns the file path. The file is in
    ///   `output_dir`, or in `atama/` in the system temp directory, and is
    ///   named after the last part of the URL path. It ignores `input`.
    /// - [`Actor::AgentTool`] runs an [`AgentToolLoop`] with `params` and
    ///   `input`. The loop is not implemented yet, and always fails.
    /// - [`Actor::MockPlaceholder`] returns `input` unchanged.
    /// - [`Actor::MockFail`] always fails.
    ///
    /// Fails if the command exits with a non-zero status, the server
    /// responds with a non-2xx status, or a value cannot be read or written.
    pub fn act(&self, input: &str) -> Result<String, String> {
        self.act_with(input, None)
    }

    /// Carries out the action like [`Actor::act`]. If `debug` is set, an
    /// [`Actor::AgentTool`] writes each request to the model, each reply and
    /// each tool call to it (see [`AgentToolLoop::debug`]). The other actors
    /// ignore `debug`.
    pub fn act_with(&self, input: &str, debug: Option<&mut dyn Write>) -> Result<String, String> {
        match self {
            Actor::BashScript { cmd, params } => run_bash(cmd, params, input),
            Actor::ApiCall {
                verb,
                url,
                input_file,
                payload,
                auth,
            } => {
                let body = match (payload, input_file) {
                    (Some(payload), _) => payload.clone(),
                    (None, Some(path)) => fs::read_to_string(path)
                        .map_err(|err| format!("cannot read input file `{path}`: {err}"))?,
                    (None, None) => input.to_string(),
                };
                call_api(verb, url, &body, auth.as_ref())
            }
            Actor::DownloadFromUrl {
                url,
                auth,
                mime,
                output_dir,
            } => download(url, auth.as_ref(), mime.as_deref(), output_dir.as_deref()),
            Actor::AgentTool { agent, params } => {
                let mut agent_loop = AgentToolLoop::new(*agent);
                match debug {
                    Some(debug) => agent_loop.debug(debug).run_loop(params, input),
                    None => agent_loop.run_loop(params, input),
                }
            }
            Actor::MockPlaceholder => Ok(input.to_string()),
            Actor::MockFail => Err("mock failure".to_string()),
        }
    }
}

fn run_bash(cmd: &str, params: &[String], input: &str) -> Result<String, String> {
    // `"$@"` passes `params` to `cmd` as separate arguments, unchanged.
    let mut child = Command::new("bash")
        .arg("-c")
        .arg(format!("{cmd} \"$@\""))
        .arg("bash")
        .args(params)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("cannot start bash: {err}"))?;

    // Write stdin from another thread, so a command that prints a lot before
    // it reads its input cannot block forever.
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let input = input.to_owned();
    let writer = thread::spawn(move || {
        // A command that does not read stdin closes it early. That is not an error.
        let _ = stdin.write_all(input.as_bytes());
    });
    let output = child
        .wait_with_output()
        .map_err(|err| format!("cannot run `{cmd}`: {err}"))?;
    let _ = writer.join();

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "`{cmd}` failed with {}: {}",
            output.status,
            stderr.trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|_| format!("`{cmd}` printed output that is not UTF-8"))
}

/// Saves the response body to a file, overwriting any file with the same
/// name, and returns the file path.
fn download(
    url: &str,
    auth: Option<&Auth>,
    mime: Option<&str>,
    output_dir: Option<&str>,
) -> Result<String, String> {
    let agent = agent();
    let mut request = with_auth(&agent, agent.get(url), auth)?;
    if let Some(mime) = mime {
        request = request.header("Accept", mime);
    }
    let mut response = request
        .call()
        .map_err(|err| format!("request to `{url}` failed: {err}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.body_mut().read_to_string().unwrap_or_default();
        return Err(format!("`{url}` responded with {status}: {}", body.trim()));
    }

    let dir = output_dir.map_or_else(|| std::env::temp_dir().join("atama"), PathBuf::from);
    fs::create_dir_all(&dir)
        .map_err(|err| format!("cannot create directory `{}`: {err}", dir.display()))?;
    let path = dir.join(file_name_from_url(url));
    let mut file = File::create(&path)
        .map_err(|err| format!("cannot create file `{}`: {err}", path.display()))?;
    // Stream to the file, so a large download does not have to fit in memory.
    io::copy(&mut response.body_mut().as_reader(), &mut file)
        .map_err(|err| format!("cannot save `{url}` to `{}`: {err}", path.display()))?;
    Ok(path.display().to_string())
}

/// The last part of the URL path, or `download` if it is empty or not a safe
/// file name.
fn file_name_from_url(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let path = path.split_once("://").map_or(path, |(_, rest)| rest);
    let name = path.split_once('/').map_or("", |(_, path)| path);
    let name = name.rsplit('/').next().unwrap_or_default();
    match name {
        "" | "." | ".." => "download".to_string(),
        name => name.to_string(),
    }
}

/// An HTTP client that returns 4xx and 5xx responses instead of an error, so
/// the error message can include the response body.
fn agent() -> Agent {
    Agent::new_with_config(Agent::config_builder().http_status_as_error(false).build())
}

fn call_api(verb: &ApiVerb, url: &str, body: &str, auth: Option<&Auth>) -> Result<String, String> {
    let agent = agent();
    let response = match verb {
        ApiVerb::GET => with_auth(&agent, agent.get(url), auth)?.call(),
        ApiVerb::DELETE => with_auth(&agent, agent.delete(url), auth)?.call(),
        ApiVerb::POST => send(with_auth(&agent, agent.post(url), auth)?, body),
        ApiVerb::PUT => send(with_auth(&agent, agent.put(url), auth)?, body),
        ApiVerb::HEAD => with_auth(&agent, agent.head(url), auth)?.call(),
        ApiVerb::PATCH => send(with_auth(&agent, agent.patch(url), auth)?, body),
    };
    read_response(url, response)
}

/// Sends `body`, as JSON if it is valid JSON, else as plain text.
fn send(
    request: RequestBuilder<ureq::typestate::WithBody>,
    body: &str,
) -> Result<Response<Body>, ureq::Error> {
    let content_type = if serde_json::from_str::<serde_json::Value>(body).is_ok() {
        "application/json"
    } else {
        "text/plain; charset=utf-8"
    };
    request.content_type(content_type).send(body)
}

/// Adds the authentication header for `auth` to the request.
fn with_auth<B>(
    agent: &Agent,
    request: RequestBuilder<B>,
    auth: Option<&Auth>,
) -> Result<RequestBuilder<B>, String> {
    let Some(auth) = auth else {
        return Ok(request);
    };
    Ok(match auth {
        Auth::Bearer { token } | Auth::Jwt { token } => {
            request.header("Authorization", format!("Bearer {}", token.get()?))
        }
        Auth::ApiKey { header, key } => request.header(header.as_str(), key.get()?),
        Auth::OAuth2 {
            token_url,
            client_id,
            client_secret,
            scopes,
        } => {
            let token = fetch_oauth2_token(
                agent,
                token_url,
                &client_id.get()?,
                &client_secret.get()?,
                scopes,
            )?;
            request.header("Authorization", format!("Bearer {token}"))
        }
    })
}

/// Gets an access token with the OAuth 2.0 client credentials grant.
fn fetch_oauth2_token(
    agent: &Agent,
    token_url: &str,
    client_id: &str,
    client_secret: &str,
    scopes: &[String],
) -> Result<String, String> {
    let scope = scopes.join(" ");
    let mut form = vec![
        ("grant_type", "client_credentials"),
        ("client_id", client_id),
        ("client_secret", client_secret),
    ];
    if !scopes.is_empty() {
        form.push(("scope", &scope));
    }

    let body = read_response(token_url, agent.post(token_url).send_form(form))?;
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|err| format!("token response from `{token_url}` is not JSON: {err}"))?;
    json.get("access_token")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("token response from `{token_url}` has no `access_token`"))
}

/// Returns the response body, or an error for a failed request or a non-2xx status.
fn read_response(
    url: &str,
    response: Result<Response<Body>, ureq::Error>,
) -> Result<String, String> {
    let mut response = response.map_err(|err| format!("request to `{url}` failed: {err}"))?;
    let status = response.status();
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|err| format!("cannot read response from `{url}`: {err}"))?;
    if !status.is_success() {
        return Err(format!("`{url}` responded with {status}: {}", body.trim()));
    }
    Ok(body)
}
