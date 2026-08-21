use std::path::{Path, PathBuf};

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
    assert!(boundary.prove_fuel_limit());
    assert!(boundary.prove_memory_limit());
}

fn load_boundary() -> AgentToolBoundary {
    let path =
        std::env::var("CH12_COMPONENT").expect("CH12_COMPONENT must name the guest artifact");
    AgentToolBoundary::load(&workspace_relative(Path::new(&path)))
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
