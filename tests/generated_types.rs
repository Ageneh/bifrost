//! Writes the TypeScript bindings the web UI is written against.
//!
//! Run with `cargo test --features ts`. CI regenerates the bindings and fails
//! on a diff, so the checked-in copy under `frontend/lib/generated` always
//! matches the Rust wire types. The `index.ts` beside them is written by hand
//! and re-exports what the UI imports.

#![cfg(feature = "ts")]

use std::fs;
use std::path::PathBuf;

use bifrost_api::admin::auth::{
    LoginRequest, LogoutRequest, PasswordChange, SessionResponse, SetupRequest,
};
use bifrost_api::admin::{
    ApplyAction, ApplyPlan, ApplyResponse, ConfigChange, ConfigImport, DraftPatch, DraftResponse,
    DraftWrite, ErrorResponse, Reason, RestartRequest, RevisionRequest, RevisionResponse,
    SetupStatus, Severity, ValidationError,
};
use bifrost_api::config::{
    AppConfig, AuthConfig, BifrostConfig, BridgeConfig, RoomConfig, Z2mServer,
};
use bifrost_api::service::{Service, ServiceList};
use svc::traits::ServiceState;
use ts_rs::{Config, TS};

/// Directory the bindings are written to, relative to the workspace root.
const OUTPUT: &str = "frontend/lib/generated";

/// Re-exported by hand; never deleted by this test.
const INDEX: &str = "index.ts";

#[test]
fn export_typescript_bindings() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(OUTPUT);
    fs::create_dir_all(&dir).unwrap();

    // a type that was renamed or removed must not leave its old file behind
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "ts")
            && path.file_name() != Some(INDEX.as_ref())
        {
            fs::remove_file(&path).unwrap();
        }
    }

    // JavaScript numbers are the wire format for revisions; the UI never sees
    // a value that does not fit a double
    let cfg = Config::new().with_out_dir(&dir).with_large_int("number");

    AppConfig::export_all(&cfg).unwrap();
    AuthConfig::export_all(&cfg).unwrap();
    BifrostConfig::export_all(&cfg).unwrap();
    BridgeConfig::export_all(&cfg).unwrap();
    RoomConfig::export_all(&cfg).unwrap();
    Z2mServer::export_all(&cfg).unwrap();

    Service::export_all(&cfg).unwrap();
    ServiceList::export_all(&cfg).unwrap();
    ServiceState::export_all(&cfg).unwrap();

    Severity::export_all(&cfg).unwrap();
    ValidationError::export_all(&cfg).unwrap();
    ConfigChange::export_all(&cfg).unwrap();
    ApplyAction::export_all(&cfg).unwrap();
    ApplyPlan::export_all(&cfg).unwrap();
    Reason::export_all(&cfg).unwrap();
    DraftResponse::export_all(&cfg).unwrap();
    DraftWrite::export_all(&cfg).unwrap();
    DraftPatch::export_all(&cfg).unwrap();
    ConfigImport::export_all(&cfg).unwrap();
    RevisionRequest::export_all(&cfg).unwrap();
    RestartRequest::export_all(&cfg).unwrap();
    RevisionResponse::export_all(&cfg).unwrap();
    ApplyResponse::export_all(&cfg).unwrap();
    ErrorResponse::export_all(&cfg).unwrap();
    SetupStatus::export_all(&cfg).unwrap();

    SessionResponse::export_all(&cfg).unwrap();
    LoginRequest::export_all(&cfg).unwrap();
    SetupRequest::export_all(&cfg).unwrap();
    PasswordChange::export_all(&cfg).unwrap();
    LogoutRequest::export_all(&cfg).unwrap();
}
