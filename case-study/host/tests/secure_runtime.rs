use std::fs;
use std::path::{Path, PathBuf};

use ch14_host::{
    HostCredential, ManifestEnvelope, RuntimeError, RuntimePolicy, SecureAgentRuntime,
    provision_test_manifest,
};
use tempfile::TempDir;

const PUBLIC_KEY: &str = "case-study/keys/TEST-ONLY-public-key.hex";
const SECRET_KEY: &str = "case-study/keys/TEST-ONLY-secret-key.hex";

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

    assert!(runtime.prove_fuel_limit());
    assert!(runtime.prove_memory_limit());
    assert!(runtime.prove_invalid_output_rejected());
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
            .iter()
            .any(|event| { event.stage == "execute" && event.outcome == "rejected" })
    );
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
            &workspace_path(PUBLIC_KEY),
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
            &workspace_path(PUBLIC_KEY),
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
            &workspace_path(PUBLIC_KEY),
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
