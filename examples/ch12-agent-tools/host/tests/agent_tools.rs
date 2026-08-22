use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ch12_host::{AgentToolBoundary, BoundaryError, ToolCredential};

#[test]
#[ignore = "requires the wasm32-wasip2 guest artifact; see the chapter README"]
fn secure_boundary_rejects_bad_requests_and_executes_allowed_tool() {
    let boundary = load_boundary();
    let credential = ToolCredential::for_demo();

    assert_eq!(
        boundary.execute("{", &credential),
        Err(BoundaryError::MalformedRequest)
    );
    assert_eq!(
        boundary.execute(
            r#"{"tool":"shell","arguments":{"document_id":"handbook","query":"pwd"}}"#,
            &credential,
        ),
        Err(BoundaryError::DeniedTool)
    );
    assert_eq!(
        boundary.execute(
            r#"{"tool":"local-document-search","arguments":{"document_id":"secrets","query":"token"}}"#,
            &credential,
        ),
        Err(BoundaryError::DeniedArguments)
    );
    assert_eq!(
        boundary.execute(
            r#"{"tool":"local-document-search","arguments":{"document_id":"handbook","query":"safe"}}"#,
            &ToolCredential::without_grant(),
        ),
        Err(BoundaryError::DeniedCapability)
    );

    let response = boundary
        .execute(
            r#"{"tool":"local-document-search","arguments":{"document_id":"runbook","query":"  safe   restart "}}"#,
            &credential,
        )
        .expect("allowlisted request should execute");
    assert_eq!(response.tool, "local-document-search");
    assert_eq!(
        response.content,
        "runbook: safe restart => Restart safely, then verify health."
    );
}

#[test]
#[ignore = "requires the wasm32-wasip2 guest artifact; see the chapter README"]
fn component_resource_exhaustion_is_contained() {
    let boundary = load_boundary();
    assert_eq!(
        boundary.probe_fuel_limit(),
        Err(BoundaryError::FuelExhausted)
    );
    assert_eq!(
        boundary.probe_memory_limit(),
        Err(BoundaryError::MemoryLimitDenied)
    );
    assert_eq!(boundary.probe_guest_trap(), Err(BoundaryError::GuestTrap));
    assert_eq!(
        boundary.probe_component_declared_failure(),
        Err(BoundaryError::ComponentDeclaredFailure)
    );
    assert_eq!(
        boundary.probe_invalid_output(),
        Err(BoundaryError::InvalidComponentOutput)
    );
    assert_eq!(boundary.probe_healthy_component(), Ok(()));
}

#[test]
#[ignore = "requires the wasm32-wasip2 guest artifact; see the chapter README"]
fn runtime_failure_is_non_trap_and_healthy_component_still_loads() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must follow the Unix epoch")
        .as_nanos();
    let invalid =
        std::env::temp_dir().join(format!("ch12-invalid-{}-{nonce}.wasm", std::process::id()));
    fs::write(&invalid, b"deterministically not a WebAssembly component")
        .expect("write invalid component fixture");
    assert_eq!(
        AgentToolBoundary::load(invalid.as_path()).err(),
        Some(BoundaryError::RuntimeFailure)
    );
    fs::remove_file(invalid).expect("remove invalid component fixture");

    let healthy = load_boundary();
    assert_eq!(healthy.probe_healthy_component(), Ok(()));
}

fn load_boundary() -> AgentToolBoundary {
    let path =
        std::env::var("CH12_COMPONENT").expect("CH12_COMPONENT must name the guest artifact");
    AgentToolBoundary::load(workspace_relative(Path::new(&path)))
        .expect("component should load through its typed interface")
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
