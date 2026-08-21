use std::path::PathBuf;

use anyhow::Result;

fn main() -> Result<()> {
    let component_path = std::env::args_os().nth(1).map_or_else(
        || PathBuf::from("target/wasm32-wasip2/debug/ch07_guest.wasm"),
        PathBuf::from,
    );
    let report = ch07_host::run_containment_checks(&component_path)?;

    println!("fuel: trapped after budget exhaustion");
    println!("memory: trapped at configured growth limit");
    println!("guest failure: trapped; host process survived");
    println!("fresh store after traps: {}", report.recovery_value);
    Ok(())
}
