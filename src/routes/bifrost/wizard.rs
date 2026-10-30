//! The first-run wizard's record, on a bridge that already runs.
//!
//! Setup mode serves the same record while no configuration exists. A bridge
//! whose configuration exists but has no identity or z2m server yet keeps
//! the wizard alive, and resuming has to work from either, so both read and
//! write the one file beside the draft.

use axum::Router;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};

use bifrost_api::admin::{
    DiscoveryRequest, DiscoveryResult, ErrorResponse, WizardProgress, WizardWrite,
};

use crate::routes::bifrost::setup::{read_wizard, wizard_progress, write_wizard};
use crate::routes::extractor::Json;
use crate::server::appstate::AppState;

async fn get_wizard(State(state): State<AppState>) -> Json<WizardProgress> {
    Json(wizard_progress(read_wizard(&state.store().paths().wizard)))
}

// the refusal is a whole response, which is what the handler must return
#[allow(clippy::result_large_err)]
async fn put_wizard(
    State(state): State<AppState>,
    Json(req): Json<WizardWrite>,
) -> Result<Json<WizardProgress>, Response> {
    let record = write_wizard(&state.store().paths().wizard, req.step, req.values)
        .map_err(IntoResponse::into_response)?;

    Ok(Json(wizard_progress(Some(record))))
}

/// Ask a z2m server what it can see. Setup mode serves the same endpoint;
/// a bridge that already runs answers from the same one-shot connection, so
/// the wizard behaves the same from either.
#[allow(clippy::result_large_err)]
async fn post_discovery(
    Json(req): Json<DiscoveryRequest>,
) -> Result<Json<DiscoveryResult>, Response> {
    let (devices, rooms) = crate::backend::z2m::discover::devices(&req.server)
        .await
        .map_err(|err| {
            (
                hyper::StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    error: err.to_string(),
                    errors: vec![],
                }),
            )
                .into_response()
        })?;

    Ok(Json(DiscoveryResult { devices, rooms }))
}

/// The wizard's record and its one question to zigbee2mqtt. Everything else
/// the wizard does — drafting the configuration at review and applying it —
/// is the configuration API.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/wizard", get(get_wizard).put(put_wizard))
        .route("/discovery", post(post_discovery))
}
