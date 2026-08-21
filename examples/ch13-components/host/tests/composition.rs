use std::path::{Path, PathBuf};

use ch13_host::{CompositionError, CompositionRuntime};

#[test]
#[ignore = "requires both wasm32-wasip2 component artifacts; see the chapter README"]
fn generated_interfaces_compose_records_variants_and_results() {
    let runtime = load_runtime();

    assert_eq!(
        runtime.describe("RUST-WASM"),
        Ok("Production WebAssembly with Rust [RUST-WASM]: 7 in stock".to_owned())
    );
    assert_eq!(
        runtime.describe("COMP-MODEL"),
        Ok("Component Model Field Notes [COMP-MODEL]: back-order until 2026-09-01".to_owned())
    );
    assert_eq!(
        runtime.describe("OLD-ABI"),
        Ok("Raw Pointer ABI Guide [OLD-ABI]: discontinued".to_owned())
    );
}

#[test]
#[ignore = "requires both wasm32-wasip2 component artifacts; see the chapter README"]
fn typed_errors_cross_the_catalog_boundary() {
    let runtime = load_runtime();
    assert_eq!(
        runtime.describe("../secret"),
        Err(CompositionError::InvalidSku)
    );
    assert_eq!(runtime.describe("MISSING"), Err(CompositionError::NotFound));
}

fn load_runtime() -> CompositionRuntime {
    let catalog =
        std::env::var("CH13_CATALOG").expect("CH13_CATALOG must name the catalog artifact");
    let renderer =
        std::env::var("CH13_RENDERER").expect("CH13_RENDERER must name the renderer artifact");
    CompositionRuntime::load(
        &workspace_relative(Path::new(&catalog)),
        &workspace_relative(Path::new(&renderer)),
    )
    .expect("both components must satisfy their generated WIT worlds")
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
