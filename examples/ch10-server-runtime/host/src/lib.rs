use std::path::Path;

use anyhow::Result;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

const INSTANTIATION_FUEL: u64 = 2_000_000;

wasmtime::component::bindgen!({
    path: "../wit",
    world: "server-handler",
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

/// Per-request budgets applied to a fresh Wasmtime store.
#[derive(Clone, Copy, Debug)]
pub struct RequestLimits {
    pub fuel: u64,
    pub memory_bytes: usize,
}

impl Default for RequestLimits {
    fn default() -> Self {
        Self {
            fuel: 50_000,
            memory_bytes: 4 * 1024 * 1024,
        }
    }
}

/// An application request passed to the component.
#[derive(Debug)]
pub struct ApplicationRequest<'a> {
    pub request_id: u64,
    pub path: &'a str,
    pub body: &'a str,
    pub work: u64,
}

/// A response safe to return across an HTTP or RPC boundary.
#[derive(Debug, PartialEq, Eq)]
pub struct PublicResponse {
    pub status: u16,
    pub body: String,
}

/// A bounded, low-cardinality telemetry event.
#[derive(Debug, PartialEq, Eq)]
pub struct TelemetryEvent {
    pub request_id: u64,
    pub outcome: &'static str,
    pub status: u16,
    pub error_code: Option<&'static str>,
}

impl TelemetryEvent {
    /// Returns one deterministic JSON log line.
    #[must_use]
    pub fn to_json(&self) -> String {
        let error = self
            .error_code
            .map_or_else(|| "null".to_owned(), |code| format!("\"{code}\""));
        format!(
            "{{\"request_id\":{},\"outcome\":\"{}\",\"status\":{},\"error_code\":{error}}}",
            self.request_id, self.outcome, self.status
        )
    }
}

/// The public response and its corresponding structured event.
#[derive(Debug, PartialEq, Eq)]
pub struct Invocation {
    pub response: PublicResponse,
    pub telemetry: TelemetryEvent,
}

/// A process-level runtime that compiles once and reuses the cached component.
pub struct ServerRuntime {
    engine: Engine,
    component: Component,
    linker: Linker<HostState>,
    limits: RequestLimits,
}

impl ServerRuntime {
    /// Compiles and caches a component for subsequent request invocations.
    ///
    /// # Errors
    ///
    /// Returns an error if the engine, linker, or component cannot be loaded.
    pub fn load(component_path: &Path, limits: RequestLimits) -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.consume_fuel(true);
        let engine = Engine::new(&config)?;
        let component = Component::from_file(&engine, component_path)?;
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
        Ok(Self {
            engine,
            component,
            linker,
            limits,
        })
    }

    /// Invokes the cached component in a fresh, resource-limited store.
    #[must_use]
    pub fn invoke(&self, request: &ApplicationRequest<'_>) -> Invocation {
        match self.invoke_inner(request) {
            Ok(Ok(response)) => invocation(
                request.request_id,
                response.status,
                response.body,
                "ok",
                None,
            ),
            Ok(Err(_guest_message)) => invocation(
                request.request_id,
                400,
                "request rejected".to_owned(),
                "rejected",
                Some("component_rejected"),
            ),
            Err(_runtime_error) => invocation(
                request.request_id,
                503,
                "service temporarily unavailable".to_owned(),
                "error",
                Some("component_runtime_failure"),
            ),
        }
    }

    fn invoke_inner(&self, request: &ApplicationRequest<'_>) -> Result<Result<Response, String>> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(self.limits.memory_bytes)
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
        store.set_fuel(INSTANTIATION_FUEL)?;
        let bindings = ServerHandler::instantiate(&mut store, &self.component, &self.linker)?;
        store.set_fuel(self.limits.fuel)?;

        Ok(bindings.call_handle(
            &mut store,
            &Request {
                path: request.path.to_owned(),
                body: request.body.to_owned(),
                work: request.work,
            },
        )?)
    }
}

fn invocation(
    request_id: u64,
    status: u16,
    body: String,
    outcome: &'static str,
    error_code: Option<&'static str>,
) -> Invocation {
    Invocation {
        response: PublicResponse { status, body },
        telemetry: TelemetryEvent {
            request_id,
            outcome,
            status,
            error_code,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::TelemetryEvent;

    #[test]
    fn structured_log_is_stable_and_low_cardinality() {
        let event = TelemetryEvent {
            request_id: 7,
            outcome: "error",
            status: 503,
            error_code: Some("component_runtime_failure"),
        };
        assert_eq!(
            event.to_json(),
            "{\"request_id\":7,\"outcome\":\"error\",\"status\":503,\
             \"error_code\":\"component_runtime_failure\"}"
        );
    }
}
