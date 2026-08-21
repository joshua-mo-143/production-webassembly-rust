use std::path::{Path, PathBuf};

#[test]
#[ignore = "requires the wasm32-wasip2 guest artifact; see the chapter README"]
fn configured_limits_fail_closed_and_host_recovers() {
    let path =
        std::env::var("CH07_COMPONENT").expect("CH07_COMPONENT must name the guest artifact");
    let report = ch07_host::run_containment_checks(&workspace_relative(Path::new(&path)))
        .expect("all containment checks should pass");

    assert!(report.fuel_exhausted);
    assert!(report.memory_growth_trapped);
    assert!(report.guest_failure_trapped);
    assert_eq!(report.recovery_value, 42);
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
