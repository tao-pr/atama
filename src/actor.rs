//! Actors: the implementations that carry out a task's actions.

use crate::passer::Passer;

/// Something that can carry out an [`Action`](crate::job::Action).
#[derive(Debug, Clone, PartialEq)]
pub enum Actor {
    /// Runs a command in bash.
    BashScript {
        /// The command or script to run.
        cmd: String,
        /// Arguments passed to the command, in order.
        params: Vec<String>,
    },
    ApiCall {
        verb: ApiVerb,
        url: String,
        input_file: Option<String>,
        payload: Option<String>,
        auth: Option<Auth>
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiVerb {
    GET,
    POST,
    PUT,
    DELETE
}


/// How an [`Actor::ApiCall`] authenticates with the API.
#[derive(Debug, Clone, PartialEq)]
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
