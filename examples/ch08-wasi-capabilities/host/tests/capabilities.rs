use std::path::{Path, PathBuf};

#[test]
#[ignore = "requires the wasm32-wasip2 guest artifact; see the chapter README"]
fn only_explicit_read_capability_is_available() {
    let component =
        std::env::var("CH08_COMPONENT").expect("CH08_COMPONENT must name the guest artifact");
    let allowed = tempfile::tempdir().expect("create allowed directory");
    let denied = tempfile::tempdir().expect("create denied directory");
    std::fs::write(allowed.path().join("allowed.txt"), "explicit capability")
        .expect("write allowed fixture");
    let denied_file = denied.path().join("host-secret.txt");
    std::fs::write(&denied_file, "ambient authority would expose this")
        .expect("write denied fixture");

    let report = ch08_host::run_capability_checks(
        &workspace_relative(Path::new(&component)),
        allowed.path(),
        &denied_file,
    )
    .expect("least-privilege checks should pass");

    assert_eq!(report.allowed_contents, "explicit capability");
    assert!(report.ambient_path_denied);
    assert!(report.write_denied);
    assert!(!allowed.path().join("guest-created.txt").exists());
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
