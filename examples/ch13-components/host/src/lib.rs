use std::path::Path;

use anyhow::Result;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

const CALL_FUEL: u64 = 100_000;
const MEMORY_LIMIT_BYTES: usize = 4 * 1024 * 1024;

mod catalog_bindings {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "catalog-component",
    });
}

mod renderer_bindings {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "renderer-component",
    });
}

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

#[derive(Debug, PartialEq, Eq)]
pub enum CompositionError {
    InvalidSku,
    NotFound,
    ComponentFailure,
}

/// A typed host composition: catalog output becomes renderer input through
/// generated WIT types rather than bytes, pointers, or hand-written JSON.
pub struct CompositionRuntime {
    engine: Engine,
    linker: Linker<HostState>,
    catalog: Component,
    renderer: Component,
}

impl CompositionRuntime {
    /// Compiles both components against their generated world bindings.
    ///
    /// # Errors
    ///
    /// Returns an error if either artifact is not a compatible component.
    pub fn load(catalog_path: impl AsRef<Path>, renderer_path: impl AsRef<Path>) -> Result<Self> {
        Self::from_bytes(
            &std::fs::read(catalog_path.as_ref())?,
            &std::fs::read(renderer_path.as_ref())?,
        )
    }

    /// Compiles both components from bytes, for callers holding an artefact
    /// that was produced in memory rather than read from disk.
    ///
    /// # Errors
    ///
    /// Returns an error if either artifact is not a compatible component.
    pub fn from_bytes(catalog: &[u8], renderer: &[u8]) -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.consume_fuel(true);
        let engine = Engine::new(&config)?;
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
        let catalog = Component::new(&engine, catalog)
            .map_err(|error| anyhow::anyhow!("catalog interface mismatch: {error}"))?;
        let renderer = Component::new(&engine, renderer)
            .map_err(|error| anyhow::anyhow!("renderer interface mismatch: {error}"))?;
        let runtime = Self {
            engine,
            linker,
            catalog,
            renderer,
        };
        let mut catalog_store = runtime.store()?;
        catalog_bindings::CatalogComponent::instantiate(
            &mut catalog_store,
            &runtime.catalog,
            &runtime.linker,
        )
        .map_err(|error| anyhow::anyhow!("catalog interface mismatch: {error}"))?;
        let mut renderer_store = runtime.store()?;
        renderer_bindings::RendererComponent::instantiate(
            &mut renderer_store,
            &runtime.renderer,
            &runtime.linker,
        )
        .map_err(|error| anyhow::anyhow!("renderer interface mismatch: {error}"))?;
        Ok(runtime)
    }

    /// Looks up a typed product in one component and passes its record and
    /// availability variant to the second component.
    ///
    /// # Errors
    ///
    /// Returns typed catalog errors or a contained component failure.
    pub fn describe(&self, sku: &str) -> Result<String, CompositionError> {
        let product = self.lookup(sku)?;
        self.render(product)
    }

    fn lookup(
        &self,
        sku: &str,
    ) -> Result<catalog_bindings::book::composition::domain::Product, CompositionError> {
        let mut store = self
            .store()
            .map_err(|_| CompositionError::ComponentFailure)?;
        let bindings = catalog_bindings::CatalogComponent::instantiate(
            &mut store,
            &self.catalog,
            &self.linker,
        )
        .map_err(|_| CompositionError::ComponentFailure)?;
        let result = bindings
            .book_composition_catalog()
            .call_lookup(&mut store, sku)
            .map_err(|_| CompositionError::ComponentFailure)?;
        result.map_err(|error| match error {
            catalog_bindings::book::composition::domain::LookupError::InvalidSku => {
                CompositionError::InvalidSku
            }
            catalog_bindings::book::composition::domain::LookupError::NotFound => {
                CompositionError::NotFound
            }
        })
    }

    fn render(
        &self,
        product: catalog_bindings::book::composition::domain::Product,
    ) -> Result<String, CompositionError> {
        let renderer_product = renderer_bindings::book::composition::domain::Product {
            sku: product.sku,
            name: product.name,
            availability: match product.availability {
                catalog_bindings::book::composition::domain::Availability::InStock(count) => {
                    renderer_bindings::book::composition::domain::Availability::InStock(count)
                }
                catalog_bindings::book::composition::domain::Availability::BackOrder(date) => {
                    renderer_bindings::book::composition::domain::Availability::BackOrder(date)
                }
                catalog_bindings::book::composition::domain::Availability::Discontinued => {
                    renderer_bindings::book::composition::domain::Availability::Discontinued
                }
            },
        };
        let mut store = self
            .store()
            .map_err(|_| CompositionError::ComponentFailure)?;
        let bindings = renderer_bindings::RendererComponent::instantiate(
            &mut store,
            &self.renderer,
            &self.linker,
        )
        .map_err(|_| CompositionError::ComponentFailure)?;
        bindings
            .book_composition_renderer()
            .call_render(&mut store, &renderer_product)
            .map_err(|_| CompositionError::ComponentFailure)?
            .map_err(|_domain_message| CompositionError::ComponentFailure)
    }

    fn store(&self) -> Result<Store<HostState>> {
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
        store.set_fuel(CALL_FUEL)?;
        Ok(store)
    }
}
