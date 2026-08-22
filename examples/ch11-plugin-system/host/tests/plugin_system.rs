use std::fs;
use std::path::{Path, PathBuf};

use ch11_host::{
    MAX_COMPONENT_BYTES, MAX_MANIFEST_BYTES, PluginInvocationError, PluginRegistry, sha256_hex,
};

#[test]
fn manifest_limit_rejects_limit_plus_one_bytes() {
    let directory = tempfile::tempdir().expect("temporary allowlist should be created");
    let manifest = directory.path().join("plugins.manifest");
    fs::write(&manifest, vec![b'#'; MAX_MANIFEST_BYTES + 1])
        .expect("oversized manifest fixture should be written");

    let error = PluginRegistry::load(directory.path(), manifest)
        .err()
        .expect("limit-plus-one manifest must fail closed");
    assert!(
        format!("{error:#}").contains("plugin manifest exceeds 65536-byte limit"),
        "unexpected error chain: {error:#}"
    );
}

#[test]
#[ignore = "requires both wasm32-wasip2 plugin artifacts; see the chapter README"]
fn v1_1_upgrade_remains_compatible_with_v1_contract() {
    let v1 = artifact_from_env("CH11_PLUGIN_V1");
    let v1_1 = artifact_from_env("CH11_PLUGIN_V1_1");

    let v1_directory = allowlist(&v1);
    let v1_registry = load(&v1_directory);
    assert_eq!(v1_registry.plugins(), vec![("uppercase", "1.0.0")]);
    assert_eq!(
        v1_registry
            .invoke("uppercase", "plugin")
            .expect("v1 should run"),
        "PLUGIN"
    );

    let v1_1_directory = allowlist(&v1_1);
    let v1_1_registry = load(&v1_1_directory);
    assert_eq!(v1_1_registry.plugins(), vec![("uppercase", "1.1.0")]);
    assert_eq!(
        v1_1_registry
            .invoke("uppercase", "plugin")
            .expect("v1.1 should run through the v1 contract"),
        "PLUGIN 1.1"
    );
}

#[test]
#[ignore = "requires the wasm32-wasip2 v1 plugin artifact; see the chapter README"]
fn verification_and_failure_containment_fail_closed() {
    let v1 = artifact_from_env("CH11_PLUGIN_V1");
    let directory = allowlist(&v1);
    let registry = load(&directory);

    assert_eq!(
        registry.invoke("uppercase", "trap"),
        Err(PluginInvocationError::Failed)
    );
    assert_eq!(
        registry.invoke("uppercase", "unsafe-control"),
        Err(PluginInvocationError::Failed)
    );
    assert_eq!(
        registry.invoke("uppercase", "oversized-output"),
        Err(PluginInvocationError::Failed)
    );
    assert_eq!(
        registry
            .invoke("uppercase", "healthy")
            .expect("fresh store should recover"),
        "HEALTHY"
    );

    fs::write(directory.path().join("uppercase.wasm"), b"tampered")
        .expect("tampering fixture should succeed");
    let error = PluginRegistry::load(directory.path(), directory.path().join("plugins.manifest"))
        .err()
        .expect("digest mismatch must reject the artifact");
    assert!(error.to_string().contains("SHA-256 verification failed"));

    let oversized_directory = tempfile::tempdir().expect("temporary allowlist should be created");
    let oversized_bytes = vec![0_u8; MAX_COMPONENT_BYTES + 1];
    fs::write(
        oversized_directory.path().join("uppercase.wasm"),
        &oversized_bytes,
    )
    .expect("oversized component fixture should be written");
    fs::write(
        oversized_directory.path().join("plugins.manifest"),
        format!(
            "uppercase|uppercase.wasm|{}\n",
            sha256_hex(&oversized_bytes)
        ),
    )
    .expect("oversized component manifest should be written");
    let error = PluginRegistry::load(
        oversized_directory.path(),
        oversized_directory.path().join("plugins.manifest"),
    )
    .err()
    .expect("limit-plus-one component must fail closed");
    assert!(
        error
            .to_string()
            .contains("plugin component exceeds 8388608-byte limit")
    );
}

fn allowlist(source: &Path) -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("temporary allowlist should be created");
    let bytes = fs::read(source).expect("plugin artifact should be readable");
    fs::write(directory.path().join("uppercase.wasm"), &bytes).expect("plugin should be copied");
    fs::write(
        directory.path().join("plugins.manifest"),
        format!("uppercase|uppercase.wasm|{}\n", sha256_hex(&bytes)),
    )
    .expect("manifest should be written");
    directory
}

fn load(directory: &tempfile::TempDir) -> PluginRegistry {
    PluginRegistry::load(directory.path(), directory.path().join("plugins.manifest"))
        .expect("verified compatible plugin should load")
}

fn artifact_from_env(variable: &str) -> PathBuf {
    let path = std::env::var(variable).unwrap_or_else(|_| panic!("{variable} must be set"));
    workspace_relative(Path::new(&path))
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
