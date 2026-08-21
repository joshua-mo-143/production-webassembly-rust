use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use ch11_host::{PluginRegistry, sha256_hex};
use tempfile::TempDir;

fn main() -> Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    let v1_source = arguments.next().map_or_else(
        || PathBuf::from("target/wasm32-wasip2/debug/ch11_plugin_v1.wasm"),
        PathBuf::from,
    );
    let v1_1_source = arguments.next().map_or_else(
        || PathBuf::from("target/wasm32-wasip2/debug/ch11_plugin_v1_1.wasm"),
        PathBuf::from,
    );

    let v1_directory = prepare_allowlist(&v1_source)?;
    let v1_registry = PluginRegistry::load(
        v1_directory.path(),
        &v1_directory.path().join("plugins.manifest"),
    )?;
    println!("verified uppercase 1.0.0 sha256=ok");
    println!("v1 output: {}", v1_registry.invoke("uppercase", "plugin")?);
    let contained = v1_registry
        .invoke("uppercase", "trap")
        .expect_err("the v1 fixture must trap");
    println!("v1 trap: contained ({contained})");

    let v1_1_directory = prepare_allowlist(&v1_1_source)?;
    let v1_1_registry = PluginRegistry::load(
        v1_1_directory.path(),
        &v1_1_directory.path().join("plugins.manifest"),
    )?;
    println!("verified uppercase 1.1.0 sha256=ok");
    println!(
        "upgrade output: {}",
        v1_1_registry.invoke("uppercase", "plugin")?
    );
    Ok(())
}

fn prepare_allowlist(source: &Path) -> Result<TempDir> {
    let directory = tempfile::tempdir()?;
    let bytes = fs::read(source)?;
    let artifact = directory.path().join("uppercase.wasm");
    fs::write(&artifact, &bytes)?;
    fs::write(
        directory.path().join("plugins.manifest"),
        format!("uppercase|uppercase.wasm|{}\n", sha256_hex(&bytes)),
    )?;
    Ok(directory)
}
