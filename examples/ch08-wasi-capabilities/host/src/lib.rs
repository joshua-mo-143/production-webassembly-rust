use std::path::Path;

use anyhow::{Result, ensure};
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{FsPerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

wasmtime::component::bindgen!({
    path: "../wit",
    world: "capability-reader",
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

/// Evidence that the granted read succeeded and ungranted operations failed.
#[derive(Debug, PartialEq, Eq)]
pub struct CapabilityReport {
    pub allowed_contents: String,
    pub ambient_path_denied: bool,
    pub write_denied: bool,
}

/// Runs a component with one read-only directory preopen and no ambient access.
///
/// # Errors
///
/// Returns an error if runtime setup fails, the allowed file cannot be read,
/// an ungranted host path becomes visible, or the guest writes to the preopen.
pub fn run_capability_checks(
    component_path: &Path,
    allowed_directory: &Path,
    denied_host_path: &Path,
) -> Result<CapabilityReport> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    let engine = Engine::new(&config)?;
    let component = Component::from_file(&engine, component_path)?;
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;

    let mut wasi = WasiCtxBuilder::new();
    wasi.preopened_dir(allowed_directory, "/data", FsPerms::ReadOnly)?;
    let mut store = Store::new(
        &engine,
        HostState {
            table: ResourceTable::new(),
            wasi: wasi.build(),
        },
    );
    let bindings = CapabilityReader::instantiate(&mut store, &component, &linker)?;

    let allowed_contents = bindings
        .call_read_allowed(&mut store)?
        .map_err(anyhow::Error::msg)?;

    let denied_path = denied_host_path.to_string_lossy();
    let ambient_path_denied = bindings.call_read_path(&mut store, &denied_path)?.is_err();
    ensure!(
        ambient_path_denied,
        "guest read an absolute host path that was never preopened"
    );

    let write_denied = bindings.call_write_allowed_directory(&mut store)?.is_err();
    ensure!(
        write_denied,
        "guest wrote through a read-only directory capability"
    );
    ensure!(
        !allowed_directory.join("guest-created.txt").exists(),
        "denied guest write changed the host filesystem"
    );

    Ok(CapabilityReport {
        allowed_contents,
        ambient_path_denied,
        write_denied,
    })
}
