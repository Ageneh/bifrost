//! The log viewer's API under `/bifrost/logs`.
//!
//! Reads the buffer the logger writes into, so the viewer shows the same
//! lines the console does. New lines arrive over the websocket as
//! `Update::Log`.

use axum::Router;
use axum::http::Uri;
use axum::routing::get;

use bifrost_api::admin::LogList;

use crate::logbuffer;
use crate::routes::bifrost::BifrostApiResult;
use crate::routes::extractor::Json;
use crate::server::appstate::AppState;

/// Lines returned when the caller names no limit.
const DEFAULT_LIMIT: usize = 200;

/// The most lines one response carries, whatever the caller asks for.
const MAX_LIMIT: usize = logbuffer::CAPACITY;

/// The `limit` of `?limit=N`, or the default.
///
/// Parsed here rather than with a query extractor: axum's `Query` is behind
/// a feature this build does not enable, and the API takes one parameter.
fn limit_from(query: Option<&str>) -> usize {
    query
        .and_then(|query| {
            query
                .split('&')
                .find_map(|pair| pair.strip_prefix("limit="))
        })
        .and_then(|limit| limit.parse().ok())
        .unwrap_or(DEFAULT_LIMIT)
        .min(MAX_LIMIT)
}

async fn get_logs(uri: Uri) -> BifrostApiResult<Json<LogList>> {
    let limit = limit_from(uri.query());

    Ok(Json(LogList {
        records: logbuffer::shared().snapshot(Some(limit)),
    }))
}

/// The log routes, mounted by the `/bifrost` nest and therefore behind the
/// same guard as every other admin route.
pub fn router() -> Router<AppState> {
    Router::new().route("/", get(get_logs))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request};
    use hyper::StatusCode;
    use serde_json::Value;
    use svc::manager::ServiceManager;
    use tower::ServiceExt;

    use bifrost_api::admin::LogRecord;

    use crate::config_store::ConfigStore;
    use crate::logbuffer;
    use crate::routes::bifrost::logs::router;
    use crate::server::appstate::AppState;
    use crate::testutil::{MINIMAL_CONFIG, TempDir};

    /// The buffer is shared by the whole test process, so these two tests
    /// take turns.
    static BUFFER_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn record(message: &str) -> LogRecord {
        LogRecord {
            timestamp: "2026-10-30T00:00:00Z".into(),
            level: "INFO".into(),
            target: "bifrost::test".into(),
            message: message.into(),
        }
    }

    async fn call(uri: &str) -> (StatusCode, Value) {
        let dir = TempDir::new();
        let store = ConfigStore::load(&dir.write("config.yaml", MINIMAL_CONFIG)).unwrap();
        let (svm, _future) = ServiceManager::spawn();
        let state = AppState::from_store(Arc::new(store), svm).await.unwrap();
        let router: Router = router().with_state(state);
        let request = Request::builder()
            .method(Method::GET)
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        let response = router.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn the_viewer_serves_the_buffer() {
        let _guard = BUFFER_LOCK.lock().await;
        let buffer = logbuffer::shared();
        buffer.push(record("first line of this test"));
        buffer.push(record("second line of this test"));

        let (status, body) = call("/").await;

        assert_eq!(status, StatusCode::OK);
        let messages: Vec<&str> = body["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|record| record["message"].as_str().unwrap())
            .filter(|message| message.ends_with("line of this test"))
            .collect();
        assert_eq!(
            messages,
            ["first line of this test", "second line of this test"]
        );
    }

    #[tokio::test]
    async fn limit_returns_the_most_recent_lines() {
        let _guard = BUFFER_LOCK.lock().await;
        let buffer = logbuffer::shared();
        for index in 0..10 {
            buffer.push(record(&format!("line {index} of the limit test")));
        }

        let (_, body) = call("/?limit=3").await;

        let messages: Vec<&str> = body["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|record| record["message"].as_str().unwrap())
            .filter(|message| message.ends_with("of the limit test"))
            .collect();
        assert_eq!(
            messages,
            [
                "line 7 of the limit test",
                "line 8 of the limit test",
                "line 9 of the limit test"
            ]
        );
    }
}
