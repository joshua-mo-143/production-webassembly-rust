use std::path::Path;

use anyhow::{Result, bail};
use ch14_host::{HostCredential, RuntimePolicy, SecureAgentRuntime, provision_test_manifest};

const MANIFEST: &str = "target/ch14/manifest.json";
const SECRET_KEY: &str = "case-study/keys/TEST-ONLY-secret-key.hex";
const PUBLIC_KEY: &str = "case-study/keys/TEST-ONLY-public-key.hex";

fn main() -> Result<()> {
    let command = std::env::args().nth(1).unwrap_or_else(|| "run".to_owned());
    let root = Path::new(".");
    match command.as_str() {
        "provision" => {
            provision_test_manifest(root, Path::new(MANIFEST), Path::new(SECRET_KEY))?;
            println!("wrote authenticated test manifest: {MANIFEST}");
        }
        "run" => run_demo(root)?,
        _ => bail!("usage: cargo run -p ch14-host -- [provision|run]"),
    }
    Ok(())
}

fn run_demo(root: &Path) -> Result<()> {
    let runtime = SecureAgentRuntime::load(
        root,
        Path::new(MANIFEST),
        Path::new(PUBLIC_KEY),
        RuntimePolicy::allow_all(Path::new("case-study/fixtures/workspace")),
    )
    .map_err(anyhow::Error::new)?;
    let credential = HostCredential::test_only();

    let normalized = runtime
        .execute(
            r#"{"tool":"normalize","arguments":{"text":"  review   then deploy  "}}"#,
            &credential,
        )
        .map_err(anyhow::Error::new)?;
    let document = runtime
        .execute(
            r#"{"tool":"workspace-read","arguments":{"path":"runbook.txt"}}"#,
            &credential,
        )
        .map_err(anyhow::Error::new)?;

    println!("normalize: {}", normalized.content);
    println!("workspace-read: {}", document.content.trim());
    println!(
        "classifications: fuel={:?} memory={:?} invalid-output={:?}",
        runtime
            .probe_fuel_limit()
            .expect_err("fuel probe must fail"),
        runtime
            .probe_memory_limit()
            .expect_err("memory probe must fail"),
        runtime
            .probe_invalid_output()
            .expect_err("invalid output must fail")
    );
    println!("structured-events: {}", runtime.events().events.len());
    println!("network-calls: 0");
    Ok(())
}
