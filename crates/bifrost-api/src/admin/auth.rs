//! Wire types for the authentication endpoints under `/bifrost/auth`.

use std::fmt::{self, Debug};

use serde::{Deserialize, Serialize};

/// Cookie holding the session token.
pub const SESSION_COOKIE: &str = "bifrost_session";

/// Header every mutating request with a session must carry, holding
/// [`SessionResponse::csrf`].
pub const CSRF_HEADER: &str = "x-bifrost-csrf";

/// Placeholder `Debug` prints instead of a password.
const HIDDEN: &str = "<<REDACTED>>";

/// How the current request is authenticated.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionResponse {
    /// The request may use the admin API.
    pub authenticated: bool,
    /// No admin password is set yet; `POST /bifrost/auth/setup` sets one.
    pub setup_required: bool,
    /// The request came through Home Assistant Ingress, which authenticated
    /// the user already.
    pub ingress: bool,
    /// Token to send as `X-Bifrost-Csrf` on every mutating request. Only set
    /// for a session.
    pub csrf: Option<String>,
}

/// Log in with the admin password.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoginRequest {
    pub password: String,
}

/// Set the first admin password.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetupRequest {
    /// The setup code Bifrost printed to its console at startup. Not needed
    /// through Home Assistant Ingress.
    #[serde(default)]
    pub setup_code: Option<String>,
    pub password: String,
}

/// Change the admin password. Every other session is logged out.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PasswordChange {
    pub current_password: String,
    pub new_password: String,
}

/// End the current session, or every session with `all`.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogoutRequest {
    #[serde(default)]
    pub all: bool,
}

// passwords and setup codes must never reach a log line through Debug

impl Debug for LoginRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoginRequest")
            .field("password", &HIDDEN)
            .finish()
    }
}

impl Debug for SetupRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SetupRequest")
            .field("setup_code", &self.setup_code.as_ref().map(|_| HIDDEN))
            .field("password", &HIDDEN)
            .finish()
    }
}

impl Debug for PasswordChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PasswordChange")
            .field("current_password", &HIDDEN)
            .field("new_password", &HIDDEN)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{LoginRequest, PasswordChange, SetupRequest};

    #[test]
    fn debug_hides_passwords() {
        let debug = format!(
            "{:?} {:?} {:?}",
            LoginRequest {
                password: "hunter2-login".into()
            },
            SetupRequest {
                setup_code: Some("ABCD-EFGH".into()),
                password: "hunter2-setup".into()
            },
            PasswordChange {
                current_password: "hunter2-old".into(),
                new_password: "hunter2-new".into()
            },
        );
        for secret in ["hunter2", "ABCD"] {
            assert!(!debug.contains(secret), "{debug}");
        }
    }
}
