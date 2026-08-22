//! Stage B of the staged migration: a WASI 0.1 core module reaches the same
//! typed contract as a natively componentised guest, through an adapter.

use std::path::{Path, PathBuf};

use ch13_host::{CompositionError, CompositionRuntime};
use sha2::{Digest, Sha256};
use wasi_preview1_component_adapter_provider::{
    WASI_SNAPSHOT_PREVIEW1_ADAPTER_NAME, WASI_SNAPSHOT_PREVIEW1_REACTOR_ADAPTER,
};
use wit_component::ComponentEncoder;

const CORE_MODULE_HEADER: &[u8] = b"\0asm\x01\0\0\0";
const COMPONENT_HEADER: &[u8] = b"\0asm\x0d\0\x01\0";

/// The catalogue SKUs exercised by the composition tests, including the two
/// that must fail with typed errors rather than a trap.
const SKUS: [&str; 5] = ["RUST-WASM", "COMP-MODEL", "OLD-ABI", "../secret", "MISSING"];

#[test]
#[ignore = "requires the wasm32-wasip1 core module and both wasm32-wasip2 components; see the chapter README"]
fn adapted_preview1_module_matches_the_componentised_guest() {
    let renderer = read_artifact("CH13_RENDERER");
    let core_module = read_artifact("CH13_CATALOG_PREVIEW1");
    let componentised = read_artifact("CH13_CATALOG");

    assert!(
        core_module.starts_with(CORE_MODULE_HEADER),
        "the wasm32-wasip1 build must still be a core module"
    );
    assert!(
        componentised.starts_with(COMPONENT_HEADER),
        "the wasm32-wasip2 build must already be a component"
    );

    let adapted = adapt(&core_module);
    assert!(
        adapted.starts_with(COMPONENT_HEADER),
        "adaptation must produce a component"
    );

    let native_route = CompositionRuntime::from_bytes(&componentised, &renderer)
        .expect("the componentised guest must satisfy the catalog world");
    let adapted_route = CompositionRuntime::from_bytes(&adapted, &renderer)
        .expect("the adapted module must satisfy the same catalog world");

    for sku in SKUS {
        assert_eq!(
            adapted_route.describe(sku),
            native_route.describe(sku),
            "routes disagree for {sku}"
        );
    }

    // The typed error paths are the ones a migration is most likely to break,
    // so pin them rather than trusting an all-Err agreement.
    assert_eq!(
        adapted_route.describe("../secret"),
        Err(CompositionError::InvalidSku)
    );
    assert_eq!(
        adapted_route.describe("MISSING"),
        Err(CompositionError::NotFound)
    );
    assert_eq!(
        adapted_route.describe("RUST-WASM"),
        Ok("Production WebAssembly with Rust [RUST-WASM]: 7 in stock".to_owned())
    );
}

#[test]
#[ignore = "requires the wasm32-wasip1 core module; see the chapter README"]
fn adaptation_is_reproducible_and_the_adapter_is_part_of_the_identity() {
    let core_module = read_artifact("CH13_CATALOG_PREVIEW1");

    let first = adapt(&core_module);
    let second = adapt(&core_module);
    assert_eq!(
        digest(&first),
        digest(&second),
        "the same module and adapter must produce the same component"
    );

    assert_ne!(
        digest(&first),
        digest(&core_module),
        "the adapted component is a distinct artefact from its input module"
    );

    // A release record needs all three identities: changing the adapter alone
    // changes the shipped bytes, so it cannot be treated as invisible glue.
    println!("module    sha256={}", digest(&core_module));
    println!(
        "adapter   sha256={}",
        digest(WASI_SNAPSHOT_PREVIEW1_REACTOR_ADAPTER)
    );
    println!("component sha256={}", digest(&first));
}

fn adapt(core_module: &[u8]) -> Vec<u8> {
    ComponentEncoder::default()
        .validate(true)
        .module(core_module)
        .expect("the core module must carry a component type section")
        .adapter(
            WASI_SNAPSHOT_PREVIEW1_ADAPTER_NAME,
            WASI_SNAPSHOT_PREVIEW1_REACTOR_ADAPTER,
        )
        .expect("the reactor adapter must satisfy the module's preview1 imports")
        .encode()
        .expect("encoding must produce a valid component")
}

fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .fold(String::new(), |mut accumulator, byte| {
            use std::fmt::Write as _;
            let _ = write!(accumulator, "{byte:02x}");
            accumulator
        })
}

fn read_artifact(variable: &str) -> Vec<u8> {
    let path = std::env::var(variable)
        .unwrap_or_else(|_| panic!("{variable} must name the build artifact"));
    std::fs::read(workspace_relative(Path::new(&path)))
        .unwrap_or_else(|error| panic!("{variable} must be readable: {error}"))
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
