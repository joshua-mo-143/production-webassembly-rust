use std::fmt;
use std::path::Path;

use serde::Deserialize;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, ResourceLimiter, Store, Trap};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

const INSTANTIATION_FUEL: u64 = 2_000_000;
const CALL_FUEL: u64 = 75_000;
const MEMORY_LIMIT_BYTES: usize = 4 * 1024 * 1024;
const MAX_REQUEST_BYTES: usize = 1_024;
const MAX_QUERY_CHARS: usize = 256;
const MAX_OUTPUT_CHARS: usize = 200;

wasmtime::component::bindgen!({
    path: "../wit",
    world: "agent-tool-logic",
});

use exports::book::agent_tools::preprocessing::{Operation, Prepared, Rejection, Request};

struct HostState {
    memory_limit_denied: bool,
    table: ResourceTable,
    wasi: WasiCtx,
}

impl ResourceLimiter for HostState {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > MEMORY_LIMIT_BYTES {
            self.memory_limit_denied = true;
            return Err(wasmtime::Error::msg("configured memory limit denied"));
        }
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        _desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(true)
    }

    fn instances(&self) -> usize {
        10
    }

    fn tables(&self) -> usize {
        10
    }

    fn memories(&self) -> usize {
        2
    }
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

/// Host-owned credential. It is never placed in WIT values or guest state.
pub struct ToolCredential(String);

impl ToolCredential {
    #[must_use]
    pub fn for_demo() -> Self {
        Self("host-only-demo-token".to_owned())
    }

    /// Constructs a credential-less test context to exercise capability denial.
    #[must_use]
    pub fn without_grant() -> Self {
        Self(String::new())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ToolResponse {
    pub tool: String,
    pub content: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BoundaryError {
    MalformedRequest,
    DeniedTool,
    DeniedArguments,
    DeniedCapability,
    FuelExhausted,
    MemoryLimitDenied,
    GuestTrap,
    RuntimeFailure,
    ComponentDeclaredFailure,
    InvalidComponentOutput,
}

impl fmt::Display for BoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::MalformedRequest => "malformed tool request",
            Self::DeniedTool => "tool is not allowed",
            Self::DeniedArguments => "tool arguments are not allowed",
            Self::DeniedCapability => "required host capability is not available",
            Self::FuelExhausted => "component fuel exhausted",
            Self::MemoryLimitDenied => "component memory limit denied",
            Self::GuestTrap => "component trapped",
            Self::RuntimeFailure => "component runtime failed",
            Self::ComponentDeclaredFailure => "component rejected input",
            Self::InvalidComponentOutput => "component returned invalid output",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for BoundaryError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRequest {
    tool: String,
    arguments: SearchArguments,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArguments {
    document_id: String,
    query: String,
}

/// A host-side boundary around one deterministic local tool.
pub struct AgentToolBoundary {
    engine: Engine,
    component: Component,
    linker: Linker<HostState>,
}

impl AgentToolBoundary {
    /// Loads and type-checks the untrusted preprocessing component.
    ///
    /// # Errors
    ///
    /// Returns an error when runtime setup or component compilation fails.
    pub fn load(component_path: impl AsRef<Path>) -> Result<Self, BoundaryError> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.consume_fuel(true);
        let engine = Engine::new(&config).map_err(|_| BoundaryError::RuntimeFailure)?;
        let component = Component::from_file(&engine, component_path)
            .map_err(|_| BoundaryError::RuntimeFailure)?;
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
            .map_err(|_| BoundaryError::RuntimeFailure)?;
        Ok(Self {
            engine,
            component,
            linker,
        })
    }

    /// Parses and validates a model-shaped request, delegates only
    /// deterministic preprocessing, executes the host-owned tool, and
    /// sanitizes the public result.
    ///
    /// # Errors
    ///
    /// Returns a low-detail category when parsing, policy, component
    /// execution, or output validation fails.
    pub fn execute(
        &self,
        json: &str,
        credential: &ToolCredential,
    ) -> Result<ToolResponse, BoundaryError> {
        let request = parse_and_validate(json)?;
        if credential.0 != "host-only-demo-token" {
            return Err(BoundaryError::DeniedCapability);
        }
        let prepared = self
            .call_component(Operation::Normalize, &request.arguments.query, 0)?
            .map_err(|_rejection| BoundaryError::ComponentDeclaredFailure)?;
        validate_component_output(&prepared.text, prepared.token_estimate)?;

        let raw = execute_local_search(
            &credential.0,
            &request.arguments.document_id,
            &prepared.text,
        );
        Ok(ToolResponse {
            tool: request.tool,
            content: sanitize_output(&raw),
        })
    }

    /// Exercises a deliberately expensive guest path without exposing that
    /// control through the accepted tool-request schema.
    /// # Errors
    ///
    /// Returns the exact component execution classification.
    pub fn probe_fuel_limit(&self) -> Result<(), BoundaryError> {
        self.call_component(Operation::BurnFuel, "probe", 10_000_000)
            .map(|_| ())
    }

    /// Exercises allocation beyond the per-memory limit. As with the fuel
    /// probe, accepted JSON requests cannot select this operation.
    /// # Errors
    ///
    /// Returns the exact component execution classification.
    pub fn probe_memory_limit(&self) -> Result<(), BoundaryError> {
        self.call_component(Operation::GrowMemory, "probe", 32 * 1024 * 1024)
            .map(|_| ())
    }

    /// # Errors
    ///
    /// Returns the exact component execution classification.
    pub fn probe_guest_trap(&self) -> Result<(), BoundaryError> {
        self.call_component(Operation::Trap, "probe", 0).map(|_| ())
    }

    /// # Errors
    ///
    /// Returns the component-declared failure or an execution classification.
    pub fn probe_component_declared_failure(&self) -> Result<(), BoundaryError> {
        self.call_component(Operation::Normalize, "", 0)?
            .map(|_| ())
            .map_err(|_| BoundaryError::ComponentDeclaredFailure)
    }

    /// # Errors
    ///
    /// Returns invalid output or an earlier exact execution classification.
    pub fn probe_invalid_output(&self) -> Result<(), BoundaryError> {
        let prepared = self
            .call_component(Operation::InvalidOutput, "probe", 0)?
            .map_err(|_| BoundaryError::ComponentDeclaredFailure)?;
        validate_component_output(&prepared.text, prepared.token_estimate)
    }

    /// # Errors
    ///
    /// Returns an exact classification if the positive control fails.
    pub fn probe_healthy_component(&self) -> Result<(), BoundaryError> {
        let prepared = self
            .call_component(Operation::Normalize, "healthy probe", 0)?
            .map_err(|_| BoundaryError::ComponentDeclaredFailure)?;
        validate_component_output(&prepared.text, prepared.token_estimate)
    }

    fn call_component(
        &self,
        operation: Operation,
        text: &str,
        amount: u32,
    ) -> Result<Result<Prepared, Rejection>, BoundaryError> {
        let mut store = Store::new(
            &self.engine,
            HostState {
                memory_limit_denied: false,
                table: ResourceTable::new(),
                wasi: WasiCtxBuilder::new().build(),
            },
        );
        store.limiter(|state| state);
        store
            .set_fuel(INSTANTIATION_FUEL)
            .map_err(|_| BoundaryError::RuntimeFailure)?;
        let bindings = AgentToolLogic::instantiate(&mut store, &self.component, &self.linker)
            .map_err(|error| classify_execution_error(&error, &store))?;
        store
            .set_fuel(CALL_FUEL)
            .map_err(|_| BoundaryError::RuntimeFailure)?;
        bindings
            .book_agent_tools_preprocessing()
            .call_preprocess(
                &mut store,
                &Request {
                    operation,
                    text: text.to_owned(),
                    amount,
                },
            )
            .map_err(|error| classify_execution_error(&error, &store))
    }
}

fn classify_execution_error(error: &wasmtime::Error, store: &Store<HostState>) -> BoundaryError {
    if store.data().memory_limit_denied {
        return BoundaryError::MemoryLimitDenied;
    }
    match error.downcast_ref::<Trap>() {
        Some(Trap::OutOfFuel) => BoundaryError::FuelExhausted,
        Some(_) => BoundaryError::GuestTrap,
        None => BoundaryError::RuntimeFailure,
    }
}

fn parse_and_validate(json: &str) -> Result<WireRequest, BoundaryError> {
    if json.len() > MAX_REQUEST_BYTES {
        return Err(BoundaryError::MalformedRequest);
    }
    let request: WireRequest =
        serde_json::from_str(json).map_err(|_| BoundaryError::MalformedRequest)?;
    if request.tool != "local-document-search" {
        return Err(BoundaryError::DeniedTool);
    }
    let arguments = &request.arguments;
    if !matches!(arguments.document_id.as_str(), "handbook" | "runbook")
        || arguments.query.is_empty()
        || arguments.query.chars().count() > MAX_QUERY_CHARS
        || arguments.query.chars().any(char::is_control)
    {
        return Err(BoundaryError::DeniedArguments);
    }
    Ok(request)
}

fn validate_component_output(text: &str, token_estimate: u32) -> Result<(), BoundaryError> {
    let word_count = text.split_whitespace().count();
    if text.is_empty()
        || text.chars().count() > MAX_QUERY_CHARS
        || text.chars().any(char::is_control)
        || usize::try_from(token_estimate) != Ok(word_count)
    {
        return Err(BoundaryError::InvalidComponentOutput);
    }
    Ok(())
}

fn execute_local_search(credential: &str, document_id: &str, query: &str) -> String {
    assert_eq!(credential, "host-only-demo-token");
    let document = match document_id {
        "handbook" => "Deployments require review.\nNever print secrets.",
        "runbook" => "Restart safely\u{0007}, then verify health.",
        _ => unreachable!("document id was allowlisted"),
    };
    format!("{document_id}: {query} => {document}")
}

fn sanitize_output(raw: &str) -> String {
    raw.chars()
        .filter(|character| !character.is_control() || *character == '\n')
        .take(MAX_OUTPUT_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{BoundaryError, parse_and_validate, sanitize_output};

    #[test]
    fn malformed_and_unknown_fields_are_rejected() {
        assert!(matches!(
            parse_and_validate("{not json"),
            Err(BoundaryError::MalformedRequest)
        ));
        assert!(matches!(
            parse_and_validate(
                r#"{"tool":"local-document-search","arguments":{"document_id":"handbook","query":"x"},"extra":true}"#
            ),
            Err(BoundaryError::MalformedRequest)
        ));
    }

    #[test]
    fn tools_and_arguments_are_allowlisted() {
        assert!(matches!(
            parse_and_validate(
                r#"{"tool":"shell","arguments":{"document_id":"handbook","query":"x"}}"#
            ),
            Err(BoundaryError::DeniedTool)
        ));
        assert!(matches!(
            parse_and_validate(
                r#"{"tool":"local-document-search","arguments":{"document_id":"../../secret","query":"x"}}"#
            ),
            Err(BoundaryError::DeniedArguments)
        ));
    }

    #[test]
    fn output_controls_are_removed_and_length_is_bounded() {
        let dirty = format!("a\u{0007}b{}", "x".repeat(300));
        let clean = sanitize_output(&dirty);
        assert!(!clean.contains('\u{0007}'));
        assert_eq!(clean.chars().count(), 200);
    }

    #[test]
    fn component_load_failures_are_classified_as_runtime_failures() {
        let component =
            std::env::temp_dir().join(format!("ch12-invalid-{}.wasm", std::process::id()));
        fs::write(&component, b"not a component").expect("write invalid component");
        assert_eq!(
            super::AgentToolBoundary::load(component.as_path()).err(),
            Some(BoundaryError::RuntimeFailure)
        );
        fs::remove_file(component).expect("remove invalid component");
    }
}
