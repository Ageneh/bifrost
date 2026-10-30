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
use bifrost_api::admin::pairing::{
    IdentityBundle, IdentityMacChange, IdentityMacChangeReply, IdentityPairingEntry,
    IdentityRestore, IdentityRestoreReply, LinkButtonPress, LinkButtonStatus, PairingHistory,
    PairingMode, PairingRecord,
};
use bifrost_api::admin::{
    ApplyAction, ApplyFailure, ApplyPlan, ApplyResponse, CertificateInfo, ConfigChange,
    ConfigImport, DiscoveredDevice, DiscoveredRoom, DiscoveryRequest, DiscoveryResult, DraftPatch,
    DraftResponse, DraftWrite, ErrorResponse, Reason, RestartRequest, RevisionRequest,
    RevisionResponse, RoomDevice, RoomEntry, RoomList, RoomScene, SceneRecallRequest, SetupStatus,
    Severity, StateCounts, StateResetRequest, StateSummary, ValidationError, WizardProgress,
    WizardWrite,
};
use bifrost_api::config::{
    AppConfig, AuthConfig, BifrostConfig, BridgeConfig, DeviceConfig, PairingConfig, RoomConfig,
    Z2mServer,
};
use bifrost_api::entertainment::{
    EntertainmentCapabilities, EntertainmentConfigurationSummary, EntertainmentDevice,
    EntertainmentStatus, StreamStats,
};
use bifrost_api::inventory::{
    Availability, Capabilities, DeviceInfo, DeviceType, ExposeSummary, GroupRef, IdentifyRequest,
    InventorySnapshot, InventoryUpdate, PermitJoinRequest, PermitJoinResponse, PermitJoinState,
    ResourceRef, RoomRef, ServerInfo,
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
    PairingConfig::export_all(&cfg).unwrap();
    DeviceConfig::export_all(&cfg).unwrap();
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
    ApplyFailure::export_all(&cfg).unwrap();
    RoomList::export_all(&cfg).unwrap();
    RoomEntry::export_all(&cfg).unwrap();
    RoomDevice::export_all(&cfg).unwrap();
    RoomScene::export_all(&cfg).unwrap();
    SceneRecallRequest::export_all(&cfg).unwrap();
    WizardProgress::export_all(&cfg).unwrap();
    WizardWrite::export_all(&cfg).unwrap();
    DiscoveryRequest::export_all(&cfg).unwrap();
    DiscoveryResult::export_all(&cfg).unwrap();
    DiscoveredDevice::export_all(&cfg).unwrap();
    DiscoveredRoom::export_all(&cfg).unwrap();
    ErrorResponse::export_all(&cfg).unwrap();
    SetupStatus::export_all(&cfg).unwrap();

    CertificateInfo::export_all(&cfg).unwrap();
    StateSummary::export_all(&cfg).unwrap();
    StateCounts::export_all(&cfg).unwrap();
    StateResetRequest::export_all(&cfg).unwrap();

    Availability::export_all(&cfg).unwrap();
    Capabilities::export_all(&cfg).unwrap();
    DeviceInfo::export_all(&cfg).unwrap();
    DeviceType::export_all(&cfg).unwrap();
    ExposeSummary::export_all(&cfg).unwrap();
    GroupRef::export_all(&cfg).unwrap();
    IdentifyRequest::export_all(&cfg).unwrap();
    InventorySnapshot::export_all(&cfg).unwrap();
    InventoryUpdate::export_all(&cfg).unwrap();
    PermitJoinRequest::export_all(&cfg).unwrap();
    PermitJoinResponse::export_all(&cfg).unwrap();
    PermitJoinState::export_all(&cfg).unwrap();
    ResourceRef::export_all(&cfg).unwrap();
    RoomRef::export_all(&cfg).unwrap();
    ServerInfo::export_all(&cfg).unwrap();

    EntertainmentCapabilities::export_all(&cfg).unwrap();
    EntertainmentConfigurationSummary::export_all(&cfg).unwrap();
    EntertainmentDevice::export_all(&cfg).unwrap();
    EntertainmentStatus::export_all(&cfg).unwrap();
    StreamStats::export_all(&cfg).unwrap();

    LinkButtonStatus::export_all(&cfg).unwrap();
    LinkButtonPress::export_all(&cfg).unwrap();
    PairingMode::export_all(&cfg).unwrap();
    PairingRecord::export_all(&cfg).unwrap();
    PairingHistory::export_all(&cfg).unwrap();
    IdentityPairingEntry::export_all(&cfg).unwrap();
    IdentityBundle::export_all(&cfg).unwrap();
    IdentityMacChange::export_all(&cfg).unwrap();
    IdentityMacChangeReply::export_all(&cfg).unwrap();
    IdentityRestore::export_all(&cfg).unwrap();
    IdentityRestoreReply::export_all(&cfg).unwrap();

    SessionResponse::export_all(&cfg).unwrap();
    LoginRequest::export_all(&cfg).unwrap();
    SetupRequest::export_all(&cfg).unwrap();
    PasswordChange::export_all(&cfg).unwrap();
    LogoutRequest::export_all(&cfg).unwrap();
}
