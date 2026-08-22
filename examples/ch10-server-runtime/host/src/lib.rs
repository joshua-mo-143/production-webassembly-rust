use std::fmt;
use std::path::Path;

use anyhow::Result;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

const INSTANTIATION_FUEL: u64 = 2_000_000;
/// Largest component response body accepted by the host.
pub const MAX_RESPONSE_BODY_BYTES: usize = 64 * 1024;
/// Default per-memory ceiling for each request store.
pub const DEFAULT_MEMORY_LIMIT_BYTES: usize = 4 * 1024 * 1024;
const MIN_RESPONSE_STATUS: u16 = 200;
const MAX_RESPONSE_STATUS: u16 = 599;

const _: () = assert!(MAX_RESPONSE_BODY_BYTES < DEFAULT_MEMORY_LIMIT_BYTES);

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

impl RequestLimits {
    pub fn builder() -> RequestLimitsBuilder {
        RequestLimitsBuilder::default()
    }
}

impl Default for RequestLimits {
    fn default() -> Self {
        Self::builder().build()
    }
}

impl From<RequestLimitsBuilder> for RequestLimits {
    fn from(builder: RequestLimitsBuilder) -> Self {
        builder.build()
    }
}

/// Builds [`RequestLimits`] without a positional constructor.
#[must_use = "a builder has no effect until it is built or converted"]
#[derive(Clone, Copy, Debug)]
pub struct RequestLimitsBuilder {
    fuel: u64,
    memory_bytes: usize,
}

impl Default for RequestLimitsBuilder {
    fn default() -> Self {
        Self {
            fuel: 2_000_000,
            memory_bytes: DEFAULT_MEMORY_LIMIT_BYTES,
        }
    }
}

impl RequestLimitsBuilder {
    pub fn fuel(mut self, fuel: u64) -> Self {
        self.fuel = fuel;
        self
    }

    pub fn memory_bytes(mut self, memory_bytes: usize) -> Self {
        self.memory_bytes = memory_bytes;
        self
    }

    #[must_use]
    pub fn build(self) -> RequestLimits {
        RequestLimits {
            fuel: self.fuel,
            memory_bytes: self.memory_bytes,
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

impl fmt::Display for TelemetryEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.error_code {
            Some(code) => write!(
                formatter,
                "{{\"request_id\":{},\"outcome\":\"{}\",\"status\":{},\"error_code\":\"{code}\"}}",
                self.request_id, self.outcome, self.status
            ),
            None => write!(
                formatter,
                "{{\"request_id\":{},\"outcome\":\"{}\",\"status\":{},\"error_code\":null}}",
                self.request_id, self.outcome, self.status
            ),
        }
    }
}

impl TelemetryEvent {
    /// Returns one deterministic JSON log line.
    #[must_use]
    pub fn to_json(&self) -> String {
        self.to_string()
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
    pub fn load(
        component_path: impl AsRef<Path>,
        limits: impl Into<RequestLimits>,
    ) -> Result<Self> {
        let limits = limits.into();
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.consume_fuel(true);
        let engine = Engine::new(&config)?;
        let component = Component::from_file(&engine, component_path.as_ref())?;
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
            Ok(Ok(response)) => match PublicResponse::try_from(response) {
                Ok(response) => invocation(
                    request.request_id,
                    response.status,
                    response.body,
                    "ok",
                    None,
                ),
                Err(error) => invocation(
                    request.request_id,
                    503,
                    "service temporarily unavailable".to_owned(),
                    "error",
                    Some(error.code()),
                ),
            },
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidComponentOutput {
    Status,
    BodyTooLarge,
    UnsafeControl,
}

impl InvalidComponentOutput {
    const fn code(self) -> &'static str {
        match self {
            Self::Status => "component_output_invalid_status",
            Self::BodyTooLarge => "component_output_too_large",
            Self::UnsafeControl => "component_output_unsafe_control",
        }
    }
}

impl TryFrom<Response> for PublicResponse {
    type Error = InvalidComponentOutput;

    fn try_from(response: Response) -> Result<Self, Self::Error> {
        if !(MIN_RESPONSE_STATUS..=MAX_RESPONSE_STATUS).contains(&response.status) {
            return Err(InvalidComponentOutput::Status);
        }
        if response.body.len() > MAX_RESPONSE_BODY_BYTES {
            return Err(InvalidComponentOutput::BodyTooLarge);
        }
        if has_unsafe_control(&response.body) {
            return Err(InvalidComponentOutput::UnsafeControl);
        }
        Ok(Self {
            status: response.status,
            body: response.body,
        })
    }
}

fn has_unsafe_control(value: &str) -> bool {
    value
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\t' | '\n' | '\r'))
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
    use super::{
        InvalidComponentOutput, MAX_RESPONSE_BODY_BYTES, PublicResponse, Response, TelemetryEvent,
    };

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

    #[test]
    fn response_validation_is_byte_bounded_and_rejects_unsafe_controls() {
        let oversized = Response {
            status: 200,
            body: "é".repeat(MAX_RESPONSE_BODY_BYTES / 2 + 1),
        };
        assert_eq!(
            PublicResponse::try_from(oversized).expect_err("oversized body must fail"),
            InvalidComponentOutput::BodyTooLarge
        );
        assert_eq!(
            PublicResponse::try_from(Response {
                status: 199,
                body: String::new(),
            })
            .expect_err("informational status must fail"),
            InvalidComponentOutput::Status
        );
        assert_eq!(
            PublicResponse::try_from(Response {
                status: 200,
                body: "unsafe\u{7f}".to_owned(),
            })
            .expect_err("unsafe control must fail"),
            InvalidComponentOutput::UnsafeControl
        );
        assert!(
            PublicResponse::try_from(Response {
                status: 200,
                body: "tabs\tand\nlines\r\nare allowed".to_owned(),
            })
            .is_ok()
        );
    }
}
