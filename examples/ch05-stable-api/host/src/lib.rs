use std::path::Path;

use anyhow::{Result, bail};
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

wasmtime::component::bindgen!({
    path: "../wit",
    world: "text-processor",
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

/// Loads a v1 text processor and returns its version and normalized output.
///
/// # Errors
///
/// Returns an error if the component cannot be loaded or instantiated, does
/// not match the WIT contract, traps during a call, or reports a non-v1 API.
pub fn run_component(path: &Path, input: &str) -> Result<(String, String)> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    let engine = Engine::new(&config)?;
    let component = Component::from_file(&engine, path)?;
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
    let mut store = Store::new(
        &engine,
        HostState {
            table: ResourceTable::new(),
            wasi: WasiCtxBuilder::new().build(),
        },
    );
    let bindings = TextProcessor::instantiate(&mut store, &component, &linker)?;

    let version = bindings.call_api_version(&mut store)?;
    require_v1(&version)?;
    let output = bindings.call_normalize(&mut store, input)?;
    Ok((version, output))
}

fn require_v1(version: &str) -> Result<()> {
    let parsed = semver::Version::parse(version)?;
    if parsed.major == 1 {
        Ok(())
    } else {
        bail!("host requires stable API major version 1, guest reported {version}")
    }
}

#[cfg(test)]
mod tests {
    use super::require_v1;

    #[test]
    fn accepts_compatible_v1_versions() {
        assert!(require_v1("1.0.0").is_ok());
        assert!(require_v1("1.7.3").is_ok());
    }

    #[test]
    fn rejects_other_major_versions() {
        assert!(require_v1("2.0.0").is_err());
        assert!(require_v1("not-semver").is_err());
        assert!(require_v1("1.invalid").is_err());
    }
}
