use std::path::Path;

use anyhow::{Result, ensure};
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

const INITIAL_FUEL: u64 = 1_000_000;
const CALL_FUEL_BUDGET: u64 = 10_000;
const MEMORY_LIMIT_BYTES: usize = 4 * 1024 * 1024;

wasmtime::component::bindgen!({
    path: "../wit",
    world: "sandboxed-work",
});

struct HostState {
    limits: StoreLimits,
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

/// Evidence returned only after every fail-closed check succeeds.
#[derive(Debug, PartialEq, Eq)]
pub struct ContainmentReport {
    pub fuel_exhausted: bool,
    pub memory_growth_trapped: bool,
    pub guest_failure_trapped: bool,
    pub recovery_value: u32,
}

/// Loads the component and proves that configured failures do not escape.
///
/// # Errors
///
/// Returns an error if setup fails, a deliberately constrained call succeeds,
/// or a fresh store cannot execute a healthy call after earlier traps.
pub fn run_containment_checks(component_path: &Path) -> Result<ContainmentReport> {
    let sandbox = Sandbox::load(component_path)?;

    let (mut fuel_store, fuel_bindings) = sandbox.instantiate()?;
    fuel_store.set_fuel(CALL_FUEL_BUDGET)?;
    let fuel_result = fuel_bindings.call_burn_fuel(&mut fuel_store, 10_000_000);
    let fuel_exhausted = fuel_result.is_err() && fuel_store.get_fuel()? == 0;
    ensure!(
        fuel_exhausted,
        "CPU-bound guest did not exhaust its fuel budget"
    );

    let (mut memory_store, memory_bindings) = sandbox.instantiate()?;
    let memory_growth_trapped = memory_bindings
        .call_allocate(&mut memory_store, 16)
        .is_err();
    ensure!(
        memory_growth_trapped,
        "guest allocation escaped the configured memory limit"
    );

    let (mut failure_store, failure_bindings) = sandbox.instantiate()?;
    let guest_failure_trapped = failure_bindings
        .call_fail_deliberately(&mut failure_store)
        .is_err();
    ensure!(
        guest_failure_trapped,
        "deliberate guest failure returned successfully"
    );

    let (mut recovery_store, recovery_bindings) = sandbox.instantiate()?;
    let recovery_value = recovery_bindings.call_add(&mut recovery_store, 20, 22)?;
    ensure!(
        recovery_value == 42,
        "fresh store failed after contained guest traps"
    );

    Ok(ContainmentReport {
        fuel_exhausted,
        memory_growth_trapped,
        guest_failure_trapped,
        recovery_value,
    })
}

struct Sandbox {
    component: Component,
    engine: Engine,
    linker: Linker<HostState>,
}

impl Sandbox {
    fn load(component_path: &Path) -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.consume_fuel(true);
        let engine = Engine::new(&config)?;
        let component = Component::from_file(&engine, component_path)?;
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
        Ok(Self {
            component,
            engine,
            linker,
        })
    }

    fn instantiate(&self) -> Result<(Store<HostState>, SandboxedWork)> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(MEMORY_LIMIT_BYTES)
            .instances(10)
            .tables(10)
            .memories(2)
            .trap_on_grow_failure(true)
            .build();
        let mut store = Store::new(
            &self.engine,
            HostState {
                limits,
                table: ResourceTable::new(),
                wasi: WasiCtxBuilder::new().build(),
            },
        );
        store.limiter(|state| &mut state.limits);
        store.set_fuel(INITIAL_FUEL)?;
        let bindings = SandboxedWork::instantiate(&mut store, &self.component, &self.linker)?;
        Ok((store, bindings))
    }
}
