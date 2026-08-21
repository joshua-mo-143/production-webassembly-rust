use std::hint::black_box;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

wasmtime::component::bindgen!({
    path: "../wit",
    world: "analyzer",
});

struct HostState {
    table: ResourceTable,
    wasi: WasiCtx,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

fn main() -> Result<()> {
    let component_path = std::env::args_os().nth(1).map_or_else(
        || PathBuf::from("target/wasm32-wasip2/release/ch06_guest.wasm"),
        PathBuf::from,
    );
    let sample_count = parse_arg(2, 10_000)?;
    let iterations = parse_arg(3, 100)?;
    anyhow::ensure!(iterations > 0, "iterations must be greater than zero");

    let samples = make_samples(sample_count);
    let packed = pack_samples(&samples);

    let mut config = Config::new();
    config.wasm_component_model(true);
    let engine = Engine::new(&config)?;
    let component = Component::from_file(&engine, &component_path)?;
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
    let mut store = Store::new(
        &engine,
        HostState {
            table: ResourceTable::new(),
            wasi: WasiCtxBuilder::new().build(),
        },
    );
    let bindings = Analyzer::instantiate(&mut store, &component, &linker)?;

    let (typed_time, typed_sum) = measure(iterations, || {
        Ok(bindings.call_sum_records(&mut store, &samples)?)
    })?;
    let (packed_time, packed_sum) = measure(iterations, || {
        bindings
            .call_sum_packed(&mut store, &packed)?
            .map_err(anyhow::Error::msg)
    })?;
    anyhow::ensure!(
        typed_sum.to_bits() == packed_sum.to_bits(),
        "transfer patterns returned different sums"
    );

    println!("samples={sample_count} iterations={iterations}");
    println!(
        "typed-records: {} ns/call",
        nanos_per_call(typed_time, iterations)
    );
    println!(
        "packed-bytes:  {} ns/call",
        nanos_per_call(packed_time, iterations)
    );
    println!("Both paths returned the same sum.");
    Ok(())
}

fn parse_arg(position: usize, default: usize) -> Result<usize> {
    std::env::args().nth(position).map_or(Ok(default), |value| {
        value
            .parse()
            .with_context(|| format!("argument {position} must be a positive integer"))
    })
}

#[allow(clippy::cast_precision_loss)]
fn make_samples(count: usize) -> Vec<Sample> {
    (0..count)
        .map(|index| Sample {
            timestamp: u64::try_from(index).expect("sample index fits in u64"),
            value: index as f64 * 0.25,
        })
        .collect()
}

fn pack_samples(samples: &[Sample]) -> Vec<u8> {
    let mut packed = Vec::with_capacity(samples.len() * 16);
    for sample in samples {
        packed.extend_from_slice(&sample.timestamp.to_le_bytes());
        packed.extend_from_slice(&sample.value.to_le_bytes());
    }
    packed
}

fn measure(
    iterations: usize,
    mut operation: impl FnMut() -> Result<f64>,
) -> Result<(Duration, f64)> {
    let expected = black_box(operation()?);
    let started = Instant::now();
    for _ in 0..iterations {
        let actual = black_box(operation()?);
        anyhow::ensure!(
            actual.to_bits() == expected.to_bits(),
            "benchmark paths returned unstable sums"
        );
    }
    Ok((started.elapsed(), expected))
}

fn nanos_per_call(duration: Duration, iterations: usize) -> u128 {
    duration.as_nanos() / u128::try_from(iterations).expect("iteration count fits in u128")
}
