use std::path::{Path, PathBuf};

#[test]
#[ignore = "requires the wasm32-wasip2 guest artifact; see the chapter README"]
fn v1_guest_satisfies_host_contract() {
    let path =
        std::env::var("CH05_COMPONENT").expect("CH05_COMPONENT must name the guest artifact");
    let path = workspace_relative(Path::new(&path));
    let (version, output) =
        ch05_host::run_component(&path, "  one\n two  ").expect("component should run");

    assert_eq!(version, "1.0.0");
    assert_eq!(output, "one two");
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
