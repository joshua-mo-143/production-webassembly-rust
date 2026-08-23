use std::str;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, OriginalUri, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use tokio::sync::Semaphore;

use crate::{ApplicationRequest, ServerRuntime, TelemetryEvent};

/// Largest HTTP request body accepted before component invocation.
pub const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024;
/// Maximum number of component invocations admitted concurrently.
pub const MAX_IN_FLIGHT_REQUESTS: usize = 16;
const REQUEST_WORK: u64 = 10;

#[derive(Clone)]
struct HttpState {
    runtime: Arc<ServerRuntime>,
    request_ids: Arc<AtomicU64>,
    invocation_slots: Arc<Semaphore>,
}

/// Builds an HTTP adapter for a cached component runtime.
pub fn router(runtime: Arc<ServerRuntime>) -> Router {
    let state = HttpState {
        runtime,
        request_ids: Arc::new(AtomicU64::new(1)),
        invocation_slots: Arc::new(Semaphore::new(MAX_IN_FLIGHT_REQUESTS)),
    };

    Router::new()
        .route("/", post(invoke_component))
        .route("/{*path}", post(invoke_component))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
        .with_state(state)
}

async fn invoke_component(
    State(state): State<HttpState>,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Response {
    let request_id = state.request_ids.fetch_add(1, Ordering::Relaxed);
    let Ok(body) = str::from_utf8(&body) else {
        return error_response(
            request_id,
            StatusCode::BAD_REQUEST,
            "request body must be UTF-8",
            "http_body_invalid_utf8",
        );
    };
    let body = body.to_owned();
    let path = uri.path().to_owned();

    let Ok(permit) = Arc::clone(&state.invocation_slots).try_acquire_owned() else {
        return error_response(
            request_id,
            StatusCode::SERVICE_UNAVAILABLE,
            "service temporarily unavailable",
            "server_overloaded",
        );
    };

    let runtime = Arc::clone(&state.runtime);
    let invocation = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        runtime.invoke(&ApplicationRequest {
            request_id,
            path: &path,
            body: &body,
            work: REQUEST_WORK,
        })
    })
    .await;

    let Ok(invocation) = invocation else {
        return error_response(
            request_id,
            StatusCode::SERVICE_UNAVAILABLE,
            "service temporarily unavailable",
            "component_worker_failure",
        );
    };

    println!("{}", invocation.telemetry);
    let status = StatusCode::from_u16(invocation.response.status)
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (
        status,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        invocation.response.body,
    )
        .into_response()
}

fn error_response(
    request_id: u64,
    status: StatusCode,
    body: &'static str,
    error_code: &'static str,
) -> Response {
    println!(
        "{}",
        TelemetryEvent {
            request_id,
            outcome: "error",
            status: status.as_u16(),
            error_code: Some(error_code),
        }
    );
    (
        status,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        body,
    )
        .into_response()
}
