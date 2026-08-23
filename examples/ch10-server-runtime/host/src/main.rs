use std::path::PathBuf;

use anyhow::Result;
use ch10_host::{ApplicationRequest, RequestLimits, ServerRuntime};

fn main() -> Result<()> {
    let component_path = std::env::args_os().nth(1).map_or_else(
        || PathBuf::from("target/wasm32-wasip2/debug/ch10_guest.wasm"),
        PathBuf::from,
    );
    let runtime = ServerRuntime::load(component_path, RequestLimits::default())?;
    println!("component_cache: compiled=1");

    let requests = vec![
        ApplicationRequest {
            request_id: 1,
            path: "/uppercase",
            body: "server component",
            work: 10,
        },
        ApplicationRequest {
            request_id: 2,
            path: "/reject",
            body: "not logged",
            work: 10,
        },
        ApplicationRequest {
            request_id: 3,
            path: "/uppercase",
            body: "bounded",
            work: 10_000_000,
        },
    ];

    for request in requests {
        let invocation = runtime.invoke(&request);
        println!("{}", invocation.telemetry);
        println!(
            "response: status={} body={}",
            invocation.response.status, invocation.response.body
        );
    }
    Ok(())
}
