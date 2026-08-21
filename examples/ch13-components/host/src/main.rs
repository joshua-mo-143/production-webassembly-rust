use std::path::Path;

use anyhow::Result;
use ch13_host::{CompositionError, CompositionRuntime};

fn main() -> Result<()> {
    let runtime = CompositionRuntime::load(
        Path::new("target/wasm32-wasip2/debug/ch13_catalog.wasm"),
        Path::new("target/wasm32-wasip2/debug/ch13_renderer.wasm"),
    )?;

    println!(
        "catalog + renderer: {}",
        runtime
            .describe("RUST-WASM")
            .map_err(|error| { anyhow::anyhow!("composition failed: {error:?}") })?
    );
    println!(
        "typed not-found: {}",
        runtime.describe("MISSING") == Err(CompositionError::NotFound)
    );
    println!("boundary encoding: canonical ABI (generated)");
    Ok(())
}
