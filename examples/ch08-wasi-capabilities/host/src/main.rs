use std::path::PathBuf;

use anyhow::Result;

fn main() -> Result<()> {
    let component_path = std::env::args_os().nth(1).map_or_else(
        || PathBuf::from("target/wasm32-wasip2/debug/ch08_guest.wasm"),
        PathBuf::from,
    );

    let allowed = tempfile::tempdir()?;
    let denied = tempfile::tempdir()?;
    std::fs::write(allowed.path().join("allowed.txt"), "chapter 8")?;
    let denied_file = denied.path().join("host-secret.txt");
    std::fs::write(&denied_file, "must remain invisible")?;

    let report = ch08_host::run_capability_checks(&component_path, allowed.path(), &denied_file)?;
    println!("allowed read: {}", report.allowed_contents);
    println!("ambient host path: denied");
    println!("write in read-only preopen: denied");
    Ok(())
}
