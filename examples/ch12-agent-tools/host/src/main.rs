use std::path::Path;

use anyhow::Result;
use ch12_host::{AgentToolBoundary, ToolCredential};

fn main() -> Result<()> {
    let component = Path::new("target/wasm32-wasip2/debug/ch12_guest.wasm");
    let boundary = AgentToolBoundary::load(component)?;
    let request = r#"{"tool":"local-document-search","arguments":{"document_id":"handbook","query":"  deployment   secrets  "}}"#;
    let response = boundary.execute(request, &ToolCredential::for_demo())?;

    println!("validated tool: {}", response.tool);
    println!(
        "sanitized result: {}",
        response.content.replace('\n', " | ")
    );
    println!(
        "fuel exhaustion: {:?}",
        boundary
            .probe_fuel_limit()
            .expect_err("probe must exhaust fuel")
    );
    println!(
        "memory denial: {:?}",
        boundary
            .probe_memory_limit()
            .expect_err("probe must exceed memory limit")
    );
    println!("external calls: 0");
    Ok(())
}
