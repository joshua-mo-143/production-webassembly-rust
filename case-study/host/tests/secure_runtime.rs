use std::fs;
use std::path::{Path, PathBuf};

use ch14_host::{
    EVENT_CAPACITY, HostCredential, ManifestEnvelope, RuntimeError, RuntimePolicy,
    SecureAgentRuntime, SignedPayload, ToolIdentity, ToolManifest, provision_test_manifest,
    signing_bytes,
};
use tempfile::TempDir;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{FsPerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

mod malicious_reader_bindings {
    wasmtime::component::bindgen!({
        path: "../../examples/ch08-wasi-capabilities/wit",
        world: "capability-reader",
    });
}

struct MaliciousReaderState {
    table: ResourceTable,
    wasi: WasiCtx,
}

impl WasiView for MaliciousReaderState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

const PUBLIC_KEY: &str = "case-study/keys/TEST-ONLY-public-key.hex";
const SECRET_KEY: &str = "case-study/keys/TEST-ONLY-secret-key.hex";

#[test]
fn signing_bytes_follow_independently_specified_jcs_ordering_and_escaping() {
    let payload = SignedPayload {
        manifest_version: 1,
        tools: vec![ToolManifest {
            name: "Ω tool".to_owned(),
            interface: "book:test\\iface@1.0.0".to_owned(),
            artifact: "z/\"component.wasm".to_owned(),
            sha256: "00".to_owned(),
            capabilities: vec!["line\nfeed".to_owned(), "€".to_owned()],
        }],
    };
    // Written from RFC 8785 rules, not produced by either JSON serializer.
    // ToolManifest declares name/interface/artifact/sha256/capabilities, while
    // JCS must emit artifact/capabilities/interface/name/sha256.
    let expected = concat!(
        "production-webassembly-rust/ch14-manifest\0jcs-rfc8785\0v1\0",
        r#"{"manifest_version":1,"tools":[{"artifact":"z/\"component.wasm","#,
        r#""capabilities":["line\nfeed","€"],"#,
        r#""interface":"book:test\\iface@1.0.0","name":"Ω tool","sha256":"00"}]}"#
    );

    let actual = signing_bytes(&payload).expect("canonicalize independently specified vector");
    assert_eq!(actual, expected.as_bytes());
    assert_ne!(
        actual,
        [
            b"production-webassembly-rust/ch14-manifest\0jcs-rfc8785\0v1\0".as_slice(),
            &serde_json::to_vec(&payload).expect("serialize field-ordered JSON"),
        ]
        .concat(),
        "plain serde_json field order must not satisfy this vector"
    );
}

#[test]
#[ignore = "requires the ch14 wasm32-wasip2 artifacts; see case-study/README.md"]
fn request_policy_capabilities_limits_and_recovery() {
    let fixture = Fixture::new();
    let runtime = fixture.runtime(RuntimePolicy::allow_all(&fixture.workspace));
    let credential = HostCredential::test_only();

    let normalized = runtime
        .execute(
            r#"{"tool":"normalize","arguments":{"text":"  safe   deploy "}}"#,
            &credential,
        )
        .expect("allowed normalization should succeed");
    assert_eq!(normalized.content, "safe deploy");
    let document = runtime
        .execute(
            r#"{"tool":"workspace-read","arguments":{"path":"runbook.txt"}}"#,
            &credential,
        )
        .expect("allowlisted workspace read should succeed");
    assert_eq!(document.content, "verify health\n");

    assert_eq!(
        runtime.execute("{", &credential),
        Err(RuntimeError::MalformedRequest)
    );
    assert_eq!(
        runtime.execute(
            r#"{"tool":"shell","arguments":{"command":"pwd"}}"#,
            &credential
        ),
        Err(RuntimeError::UnknownTool)
    );
    assert_eq!(
        runtime.execute(
            r#"{"tool":"normalize","arguments":{"text":"ok","extra":true}}"#,
            &credential
        ),
        Err(RuntimeError::InvalidArguments)
    );
    assert_eq!(
        runtime.execute(
            r#"{"tool":"workspace-read","arguments":{"path":"../secret.txt"}}"#,
            &credential
        ),
        Err(RuntimeError::InvalidArguments)
    );

    let normalize_only = fixture.runtime(RuntimePolicy::normalize_only());
    assert_eq!(
        normalize_only.execute(
            r#"{"tool":"workspace-read","arguments":{"path":"runbook.txt"}}"#,
            &credential
        ),
        Err(RuntimeError::DeniedTool)
    );
    let missing_grant = fixture.runtime(RuntimePolicy::without_workspace_grant());
    assert_eq!(
        missing_grant.execute(
            r#"{"tool":"workspace-read","arguments":{"path":"runbook.txt"}}"#,
            &credential
        ),
        Err(RuntimeError::DeniedCapability)
    );

    assert_eq!(runtime.probe_fuel_limit(), Err(RuntimeError::FuelExhausted));
    assert_eq!(
        runtime.probe_memory_limit(),
        Err(RuntimeError::MemoryLimitDenied)
    );
    assert_eq!(runtime.probe_guest_trap(), Err(RuntimeError::GuestTrap));
    assert_eq!(
        runtime.probe_component_declared_failure(),
        Err(RuntimeError::ComponentDeclaredFailure)
    );
    assert_eq!(
        runtime.probe_invalid_output(),
        Err(RuntimeError::InvalidOutput)
    );
    assert_eq!(runtime.probe_healthy_component(), Ok(()));
    let recovery = runtime
        .execute(
            r#"{"tool":"normalize","arguments":{"text":"still healthy"}}"#,
            &credential,
        )
        .expect("fresh store should recover after traps");
    assert_eq!(recovery.content, "still healthy");
    assert!(
        runtime
            .events()
            .events
            .iter()
            .any(|event| { event.stage == "execute" && event.outcome == "rejected" })
    );
}

#[test]
#[ignore = "requires the ch14 wasm32-wasip2 artifacts; see case-study/README.md"]
fn telemetry_is_bounded_and_unknown_names_are_not_retained() {
    let fixture = Fixture::new();
    let runtime = fixture.runtime(RuntimePolicy::allow_all(&fixture.workspace));
    let credential = HostCredential::test_only();

    for index in 0..(EVENT_CAPACITY + 17) {
        let attacker_name = format!("attacker-controlled-{index}");
        let request = format!(r#"{{"tool":"{attacker_name}","arguments":{{}}}}"#);
        assert_eq!(
            runtime.execute(&request, &credential),
            Err(RuntimeError::UnknownTool)
        );
    }

    let snapshot = runtime.events();
    assert_eq!(snapshot.events.len(), EVENT_CAPACITY);
    assert_eq!(snapshot.overwritten, 17);
    assert!(
        snapshot
            .events
            .iter()
            .all(|event| event.tool == Some(ToolIdentity::Unknown))
    );
}

#[test]
#[ignore = "requires ch08 and ch14 wasm32-wasip2 artifacts; see case-study/README.md"]
fn malicious_reader_cannot_read_sibling_and_healthy_reader_still_succeeds() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace.join("sibling.txt"), "attacker sibling\n")
        .expect("write sibling fixture");

    let request_directory = tempfile::tempdir().expect("create request-specific preopen");
    fs::copy(
        fixture.workspace.join("runbook.txt"),
        request_directory.path().join("authorized.txt"),
    )
    .expect("stage only the authorized document");
    let mut config = Config::new();
    config.wasm_component_model(true);
    let engine = Engine::new(&config).expect("create malicious-reader engine");
    let malicious_component = Component::from_file(
        &engine,
        workspace_path("target/wasm32-wasip2/debug/ch08_guest.wasm"),
    )
    .expect("load malicious reader fixture");
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).expect("link WASI");
    let mut wasi = WasiCtxBuilder::new();
    wasi.preopened_dir(request_directory.path(), "/workspace", FsPerms::ReadOnly)
        .expect("preopen only the request directory");
    let mut store = Store::new(
        &engine,
        MaliciousReaderState {
            table: ResourceTable::new(),
            wasi: wasi.build(),
        },
    );
    let malicious = malicious_reader_bindings::CapabilityReader::instantiate(
        &mut store,
        &malicious_component,
        &linker,
    )
    .expect("instantiate malicious reader");
    assert!(
        malicious
            .call_read_path(&mut store, "/workspace/sibling.txt")
            .expect("execute malicious sibling read")
            .is_err(),
        "the request-specific preopen must not expose the source sibling"
    );
    assert_eq!(
        malicious
            .call_read_path(&mut store, "/workspace/authorized.txt")
            .expect("execute healthy staged-file read")
            .expect("authorized staged file must remain readable"),
        "verify health\n"
    );

    let runtime = fixture.runtime(RuntimePolicy::allow_all(&fixture.workspace));
    let response = runtime
        .execute(
            r#"{"tool":"workspace-read","arguments":{"path":"runbook.txt"}}"#,
            &HostCredential::test_only(),
        )
        .expect("reader receives only the authorized bytes");
    assert_eq!(response.content, "verify health\n");
    assert!(!response.content.contains("attacker sibling"));
}

#[test]
#[ignore = "requires the ch14 wasm32-wasip2 artifacts; see case-study/README.md"]
fn tampered_manifest_is_rejected() {
    let fixture = Fixture::new();
    let mut envelope: ManifestEnvelope =
        serde_json::from_slice(&fs::read(&fixture.manifest).expect("read manifest"))
            .expect("parse manifest");
    envelope.signed.tools[0]
        .capabilities
        .push("network".to_owned());
    fs::write(
        &fixture.manifest,
        serde_json::to_vec_pretty(&envelope).expect("serialize manifest"),
    )
    .expect("write tampered manifest");

    assert_eq!(
        SecureAgentRuntime::load(
            fixture.root.path(),
            &fixture.manifest,
            workspace_path(PUBLIC_KEY),
            RuntimePolicy::allow_all(&fixture.workspace),
        )
        .err(),
        Some(RuntimeError::ArtefactRejected)
    );
}

#[test]
#[ignore = "requires the ch14 wasm32-wasip2 artifacts; see case-study/README.md"]
fn tampered_component_is_rejected() {
    let fixture = Fixture::new();
    let artifact = fixture
        .root
        .path()
        .join("target/wasm32-wasip2/debug/ch14_normalizer.wasm");
    let mut bytes = fs::read(&artifact).expect("read component");
    bytes[0] ^= 0xff;
    fs::write(artifact, bytes).expect("write tampered component");

    assert_eq!(
        SecureAgentRuntime::load(
            fixture.root.path(),
            &fixture.manifest,
            workspace_path(PUBLIC_KEY),
            RuntimePolicy::allow_all(&fixture.workspace),
        )
        .err(),
        Some(RuntimeError::ArtefactRejected)
    );
}

struct Fixture {
    root: TempDir,
    manifest: PathBuf,
    workspace: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("create fixture root");
        let artifact_dir = root.path().join("target/wasm32-wasip2/debug");
        fs::create_dir_all(&artifact_dir).expect("create artifact directory");
        for artifact in ["ch14_normalizer.wasm", "ch14_workspace_reader.wasm"] {
            fs::copy(
                workspace_path(&format!("target/wasm32-wasip2/debug/{artifact}")),
                artifact_dir.join(artifact),
            )
            .expect("copy built component");
        }
        let workspace = root.path().join("workspace");
        fs::create_dir(&workspace).expect("create workspace");
        fs::write(workspace.join("runbook.txt"), "verify health\n")
            .expect("write workspace fixture");
        let manifest = root.path().join("manifest.json");
        provision_test_manifest(root.path(), &manifest, &workspace_path(SECRET_KEY))
            .expect("sign fixture manifest");
        Self {
            root,
            manifest,
            workspace,
        }
    }

    fn runtime(&self, policy: RuntimePolicy) -> SecureAgentRuntime {
        SecureAgentRuntime::load(
            self.root.path(),
            &self.manifest,
            workspace_path(PUBLIC_KEY),
            policy,
        )
        .expect("load authenticated components")
    }
}

fn workspace_path(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}
