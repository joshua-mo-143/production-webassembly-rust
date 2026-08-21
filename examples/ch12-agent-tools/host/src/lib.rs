use std::fmt;
use std::path::Path;

use anyhow::Result;
use serde::Deserialize;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
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

/// Host-owned credential. It is never placed in WIT values or guest state.
pub struct ToolCredential(String);

impl ToolCredential {
    #[must_use]
    pub fn for_demo() -> Self {
        Self("host-only-demo-token".to_owned())
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
    ComponentRejected,
    ResourceLimit,
    InvalidComponentOutput,
}

impl fmt::Display for BoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::MalformedRequest => "malformed tool request",
            Self::DeniedTool => "tool is not allowed",
            Self::DeniedArguments => "tool arguments are not allowed",
            Self::ComponentRejected => "component rejected input",
            Self::ResourceLimit => "component resource limit exceeded",
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
    pub fn load(component_path: &Path) -> Result<Self> {
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
        let prepared = self
            .call_component(Operation::Normalize, &request.arguments.query, 0)
            .map_err(|_| BoundaryError::ResourceLimit)?
            .map_err(|_rejection| BoundaryError::ComponentRejected)?;
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
    #[must_use]
    pub fn prove_fuel_limit(&self) -> bool {
        self.call_component(Operation::BurnFuel, "probe", 10_000_000)
            .is_err()
    }

    /// Exercises allocation beyond the per-memory limit. As with the fuel
    /// probe, accepted JSON requests cannot select this operation.
    #[must_use]
    pub fn prove_memory_limit(&self) -> bool {
        self.call_component(Operation::GrowMemory, "probe", 32 * 1024 * 1024)
            .is_err()
    }

    fn call_component(
        &self,
        operation: Operation,
        text: &str,
        amount: u32,
    ) -> Result<Result<Prepared, Rejection>> {
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
        store.set_fuel(INSTANTIATION_FUEL)?;
        let bindings = AgentToolLogic::instantiate(&mut store, &self.component, &self.linker)?;
        store.set_fuel(CALL_FUEL)?;
        Ok(bindings.book_agent_tools_preprocessing().call_preprocess(
            &mut store,
            &Request {
                operation,
                text: text.to_owned(),
                amount,
            },
        )?)
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
}
