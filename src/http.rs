//! HTTP helpers that the actors, the Tavily client and the OpenAI client
//! share.

use ureq::http::Response;
use ureq::{Agent, Body};

/// An HTTP agent that returns a non-2xx response instead of an error, so
/// that [`read_response`] can put the response body in the error.
pub(crate) fn agent() -> Agent {
    Agent::new_with_config(Agent::config_builder().http_status_as_error(false).build())
}

/// Reads the body of the response to a request to `url`.
///
/// Fails if the request failed, the body cannot be read, or the status is
/// not 2xx. The error of a non-2xx status has the body.
pub(crate) fn read_response(
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
