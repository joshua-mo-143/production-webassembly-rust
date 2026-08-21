use std::path::{Path, PathBuf};

use ch10_host::{ApplicationRequest, RequestLimits, ServerRuntime};

#[test]
#[ignore = "requires the wasm32-wasip2 guest artifact; see the chapter README"]
fn cached_component_handles_requests_with_fresh_limits() {
    let path =
        std::env::var("CH10_COMPONENT").expect("CH10_COMPONENT must name the guest artifact");
    let runtime = ServerRuntime::load(
        &workspace_relative(Path::new(&path)),
        RequestLimits::default(),
    )
    .expect("component should compile once");

    let success = runtime.invoke(&ApplicationRequest {
        request_id: 41,
        path: "/uppercase",
        body: "cached",
        work: 10,
    });
    assert_eq!(success.response.status, 200);
    assert_eq!(success.response.body, "CACHED");
    assert_eq!(success.telemetry.outcome, "ok");

    let rejected = runtime.invoke(&ApplicationRequest {
        request_id: 42,
        path: "/reject",
        body: "guest detail must not escape",
        work: 10,
    });
    assert_eq!(rejected.response.status, 400);
    assert_eq!(rejected.response.body, "request rejected");
    assert_eq!(rejected.telemetry.error_code, Some("component_rejected"));

    let exhausted = runtime.invoke(&ApplicationRequest {
        request_id: 43,
        path: "/uppercase",
        body: "never returned",
        work: 10_000_000,
    });
    assert_eq!(exhausted.response.status, 503);
    assert_eq!(exhausted.response.body, "service temporarily unavailable");
    assert_eq!(
        exhausted.telemetry.error_code,
        Some("component_runtime_failure")
    );

    let recovered = runtime.invoke(&ApplicationRequest {
        request_id: 44,
        path: "/uppercase",
        body: "fresh store",
        work: 10,
    });
    assert_eq!(recovered.response.status, 200);
    assert_eq!(recovered.response.body, "FRESH STORE");
}

fn workspace_relative(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .join(path)
    }
}
