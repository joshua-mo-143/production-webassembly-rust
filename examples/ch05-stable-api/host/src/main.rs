use std::path::PathBuf;

use anyhow::Result;

fn main() -> Result<()> {
    let component_path = std::env::args_os().nth(1).map_or_else(
        || PathBuf::from("target/wasm32-wasip2/debug/ch05_guest.wasm"),
        PathBuf::from,
    );

    let (version, normalized) =
        ch05_host::run_component(&component_path, "  stable\n  interfaces\tlast  ")?;
    println!("API {version}: {normalized}");
    Ok(())
}
