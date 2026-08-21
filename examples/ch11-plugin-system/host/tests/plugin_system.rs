use std::fs;
use std::path::{Path, PathBuf};

use ch11_host::{PluginInvocationError, PluginRegistry, sha256_hex};

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
        registry
            .invoke("uppercase", "healthy")
            .expect("fresh store should recover"),
        "HEALTHY"
    );

    fs::write(directory.path().join("uppercase.wasm"), b"tampered")
        .expect("tampering fixture should succeed");
    let error = PluginRegistry::load(directory.path(), &directory.path().join("plugins.manifest"))
        .err()
        .expect("digest mismatch must reject the artifact");
    assert!(error.to_string().contains("SHA-256 verification failed"));
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
    PluginRegistry::load(directory.path(), &directory.path().join("plugins.manifest"))
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
