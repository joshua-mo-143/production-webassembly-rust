use std::collections::{BTreeSet, VecDeque};
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Component as PathComponent, Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow, bail};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, ResourceLimiter, Store, Trap};
use wasmtime_wasi::{FsPerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

const MANIFEST_VERSION: u32 = 1;
const INSTANTIATION_FUEL: u64 = 3_000_000;
const CALL_FUEL: u64 = 100_000;
const MEMORY_LIMIT_BYTES: usize = 4 * 1024 * 1024;
const MAX_REQUEST_BYTES: usize = 2_048;
const MAX_TEXT_CHARS: usize = 256;
const MAX_DOCUMENT_BYTES: usize = 16 * 1024;
const MAX_OUTPUT_CHARS: usize = 1_024;
const MAX_PUBLIC_KEY_BYTES: usize = 4 * 1024;
const MAX_SECRET_KEY_BYTES: usize = 4 * 1024;
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_COMPONENT_BYTES: usize = 16 * 1024 * 1024;
pub const EVENT_CAPACITY: usize = 64;
const SIGNING_DOMAIN: &[u8] = b"production-webassembly-rust/ch14-manifest\0jcs-rfc8785\0v1\0";

mod normalizer_bindings {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "normalizer-tool",
    });
}

mod reader_bindings {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "workspace-reader-tool",
    });
}

use normalizer_bindings::exports::book::secure_agent_tools::normalizer as normalizer_types;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolManifest {
    pub name: String,
    pub interface: String,
    pub artifact: String,
    pub sha256: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignedPayload {
    pub manifest_version: u32,
    pub tools: Vec<ToolManifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestEnvelope {
    pub signed: SignedPayload,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub stage: &'static str,
    pub outcome: &'static str,
    pub tool: Option<ToolIdentity>,
    pub detail: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolIdentity {
    Normalize,
    WorkspaceRead,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventSnapshot {
    pub events: Vec<Event>,
    pub overwritten: u64,
}

#[derive(Default)]
struct EventBuffer {
    events: VecDeque<Event>,
    overwritten: u64,
}

#[derive(Debug, Clone)]
pub struct RuntimePolicy {
    allowed_tools: BTreeSet<String>,
    workspace_root: Option<PathBuf>,
}

impl RuntimePolicy {
    pub fn builder() -> RuntimePolicyBuilder {
        RuntimePolicyBuilder::default()
    }

    #[must_use]
    pub fn allow_all(workspace_root: impl AsRef<Path>) -> Self {
        Self::builder()
            .allow("normalize")
            .allow("workspace-read")
            .workspace_root(workspace_root)
            .build()
    }

    #[must_use]
    pub fn normalize_only() -> Self {
        Self::builder().allow("normalize").build()
    }

    #[must_use]
    pub fn without_workspace_grant() -> Self {
        Self::builder()
            .allow("normalize")
            .allow("workspace-read")
            .build()
    }
}

/// Builds a closed tool-allowlist and optional workspace grant.
#[must_use = "a builder has no effect until it is built or converted"]
#[derive(Default)]
pub struct RuntimePolicyBuilder {
    allowed_tools: BTreeSet<String>,
    workspace_root: Option<PathBuf>,
}

impl RuntimePolicyBuilder {
    pub fn allow(mut self, tool: impl Into<String>) -> Self {
        self.allowed_tools.insert(tool.into());
        self
    }

    pub fn workspace_root(mut self, workspace_root: impl AsRef<Path>) -> Self {
        self.workspace_root = Some(workspace_root.as_ref().to_owned());
        self
    }

    #[must_use]
    pub fn build(self) -> RuntimePolicy {
        RuntimePolicy {
            allowed_tools: self.allowed_tools,
            workspace_root: self.workspace_root,
        }
    }
}

impl From<RuntimePolicyBuilder> for RuntimePolicy {
    fn from(builder: RuntimePolicyBuilder) -> Self {
        builder.build()
    }
}

/// Host-owned authority. Its bytes are never placed in a WIT value or guest store.
pub struct HostCredential([u8; 32]);

impl HostCredential {
    #[must_use]
    pub fn test_only() -> Self {
        Self([0xA5; 32])
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResponse {
    pub tool: String,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeError {
    MalformedRequest,
    UnknownTool,
    DeniedTool,
    DeniedCapability,
    InvalidArguments,
    ArtefactRejected,
    FuelExhausted,
    MemoryLimitDenied,
    GuestTrap,
    RuntimeFailure,
    InvalidOutput,
    ComponentDeclaredFailure,
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("tool request failed")
    }
}

impl std::error::Error for RuntimeError {}

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

pub struct SecureAgentRuntime {
    engine: Engine,
    normalizer: Component,
    reader: Component,
    normalizer_linker: Linker<HostState>,
    reader_linker: Linker<HostState>,
    policy: RuntimePolicy,
    events: Mutex<EventBuffer>,
}

impl SecureAgentRuntime {
    /// Verifies the signed manifest and every component before compiling them.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::ArtefactRejected`] for invalid signatures,
    /// hashes, paths, interfaces, or components.
    pub fn load(
        repository_root: impl AsRef<Path>,
        manifest_path: impl AsRef<Path>,
        public_key_path: impl AsRef<Path>,
        policy: impl Into<RuntimePolicy>,
    ) -> Result<Self, RuntimeError> {
        Self::load_verified(
            repository_root.as_ref(),
            manifest_path.as_ref(),
            public_key_path.as_ref(),
            policy.into(),
        )
    }

    fn load_verified(
        repository_root: &Path,
        manifest_path: &Path,
        public_key_path: &Path,
        policy: RuntimePolicy,
    ) -> Result<Self, RuntimeError> {
        let manifest = read_bounded(manifest_path, MAX_MANIFEST_BYTES)
            .map_err(|_| RuntimeError::ArtefactRejected)?;
        let envelope: ManifestEnvelope =
            serde_json::from_slice(&manifest).map_err(|_| RuntimeError::ArtefactRejected)?;
        verify_envelope(&envelope, public_key_path).map_err(|_| RuntimeError::ArtefactRejected)?;
        validate_manifest(&envelope.signed).map_err(|_| RuntimeError::ArtefactRejected)?;

        let normalizer_entry = manifest_tool(&envelope.signed, "normalize")
            .map_err(|_| RuntimeError::ArtefactRejected)?;
        let reader_entry = manifest_tool(&envelope.signed, "workspace-read")
            .map_err(|_| RuntimeError::ArtefactRejected)?;
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.consume_fuel(true);
        let engine = Engine::new(&config).map_err(|_| RuntimeError::RuntimeFailure)?;
        let normalizer = verified_component(repository_root, normalizer_entry, &engine)
            .map_err(|_| RuntimeError::ArtefactRejected)?;
        let reader = verified_component(repository_root, reader_entry, &engine)
            .map_err(|_| RuntimeError::ArtefactRejected)?;
        let mut normalizer_linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut normalizer_linker)
            .map_err(|_| RuntimeError::RuntimeFailure)?;
        let mut reader_linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut reader_linker)
            .map_err(|_| RuntimeError::RuntimeFailure)?;

        let runtime = Self {
            engine,
            normalizer,
            reader,
            normalizer_linker,
            reader_linker,
            policy,
            events: Mutex::new(EventBuffer::default()),
        };
        runtime
            .typecheck()
            .map_err(|_| RuntimeError::RuntimeFailure)?;
        Ok(runtime)
    }

    /// Parses one deterministic model fixture and executes only an allowed tool.
    ///
    /// # Errors
    ///
    /// Returns a low-detail category suitable for mapping to a generic user error.
    pub fn execute(
        &self,
        model_fixture: &str,
        credential: &HostCredential,
    ) -> Result<ToolResponse, RuntimeError> {
        if model_fixture.len() > MAX_REQUEST_BYTES {
            self.record("parse", "rejected", None, "request-size");
            return Err(RuntimeError::MalformedRequest);
        }
        let wire: WireRequest = serde_json::from_str(model_fixture).map_err(|_| {
            self.record("parse", "rejected", None, "invalid-json-or-schema");
            RuntimeError::MalformedRequest
        })?;
        let tool = wire.tool;
        if !matches!(tool.as_str(), "normalize" | "workspace-read") {
            self.record("policy", "rejected", Some(&tool), "unknown-tool");
            return Err(RuntimeError::UnknownTool);
        }
        if !self.policy.allowed_tools.contains(&tool) {
            self.record("policy", "rejected", Some(&tool), "tool-not-granted");
            return Err(RuntimeError::DeniedTool);
        }

        let result = match tool.as_str() {
            "normalize" => self.execute_normalize(wire.arguments),
            "workspace-read" => self.execute_read(wire.arguments, credential),
            _ => unreachable!("known tool checked above"),
        };
        match &result {
            Ok(_) => self.record("execute", "ok", Some(&tool), "validated-output"),
            Err(error) => self.record("execute", "rejected", Some(&tool), error.detail()),
        }
        result
    }

    #[must_use]
    pub fn events(&self) -> EventSnapshot {
        let telemetry = self
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        EventSnapshot {
            events: telemetry.events.iter().cloned().collect(),
            overwritten: telemetry.overwritten,
        }
    }

    /// # Errors
    ///
    /// Returns the exact component execution classification.
    pub fn probe_fuel_limit(&self) -> Result<(), RuntimeError> {
        self.call_normalizer(normalizer_types::Operation::BurnFuel, "probe", 10_000_000)
            .map(|_| ())
    }

    /// # Errors
    ///
    /// Returns the exact component execution classification.
    pub fn probe_memory_limit(&self) -> Result<(), RuntimeError> {
        self.call_normalizer(
            normalizer_types::Operation::GrowMemory,
            "probe",
            32 * 1024 * 1024,
        )
        .map(|_| ())
    }

    /// # Errors
    ///
    /// Returns the exact component execution classification.
    pub fn probe_guest_trap(&self) -> Result<(), RuntimeError> {
        self.call_normalizer(normalizer_types::Operation::Trap, "probe", 0)
            .map(|_| ())
    }

    /// # Errors
    ///
    /// Returns the component-declared failure or an execution classification.
    pub fn probe_component_declared_failure(&self) -> Result<(), RuntimeError> {
        self.call_normalizer(normalizer_types::Operation::Normalize, "", 0)?
            .map(|_| ())
            .map_err(|_| RuntimeError::ComponentDeclaredFailure)
    }

    /// # Errors
    ///
    /// Returns invalid output or an earlier exact execution classification.
    pub fn probe_invalid_output(&self) -> Result<(), RuntimeError> {
        let value = self
            .call_normalizer(normalizer_types::Operation::InvalidOutput, "probe", 0)?
            .map_err(|_| RuntimeError::ComponentDeclaredFailure)?;
        validate_normalized(&value.text, value.word_count)
    }

    /// # Errors
    ///
    /// Returns an exact classification if the positive control fails.
    pub fn probe_healthy_component(&self) -> Result<(), RuntimeError> {
        let value = self
            .call_normalizer(normalizer_types::Operation::Normalize, "healthy probe", 0)?
            .map_err(|_| RuntimeError::ComponentDeclaredFailure)?;
        validate_normalized(&value.text, value.word_count)
    }

    fn execute_normalize(
        &self,
        arguments: serde_json::Value,
    ) -> Result<ToolResponse, RuntimeError> {
        let arguments: NormalizeArguments =
            serde_json::from_value(arguments).map_err(|_| RuntimeError::InvalidArguments)?;
        validate_text(&arguments.text)?;
        let output = self
            .call_normalizer(normalizer_types::Operation::Normalize, &arguments.text, 0)?
            .map_err(|_| RuntimeError::ComponentDeclaredFailure)?;
        validate_normalized(&output.text, output.word_count)?;
        Ok(ToolResponse {
            tool: "normalize".to_owned(),
            content: output.text,
        })
    }

    fn execute_read(
        &self,
        arguments: serde_json::Value,
        credential: &HostCredential,
    ) -> Result<ToolResponse, RuntimeError> {
        let arguments: ReadArguments =
            serde_json::from_value(arguments).map_err(|_| RuntimeError::InvalidArguments)?;
        validate_relative_path(&arguments.path)?;
        if credential.0.iter().all(|byte| *byte == 0) {
            return Err(RuntimeError::DeniedCapability);
        }
        let workspace = self
            .policy
            .workspace_root
            .as_deref()
            .ok_or(RuntimeError::DeniedCapability)?;
        let workspace = workspace
            .canonicalize()
            .map_err(|_| RuntimeError::DeniedCapability)?;
        let authorized_path = workspace
            .join(&arguments.path)
            .canonicalize()
            .map_err(|_| RuntimeError::DeniedCapability)?;
        if !authorized_path.starts_with(&workspace) {
            return Err(RuntimeError::DeniedCapability);
        }
        let authorized_bytes = read_bounded(&authorized_path, MAX_DOCUMENT_BYTES)
            .map_err(|_| RuntimeError::DeniedCapability)?;
        let authorized_text =
            std::str::from_utf8(&authorized_bytes).map_err(|_| RuntimeError::InvalidOutput)?;
        let request_directory = tempfile::tempdir().map_err(|_| RuntimeError::RuntimeFailure)?;
        fs::write(
            request_directory.path().join("authorized.txt"),
            &authorized_bytes,
        )
        .map_err(|_| RuntimeError::RuntimeFailure)?;
        let (mut store, bindings) = self
            .instantiate_reader(request_directory.path())
            .map_err(|_| RuntimeError::RuntimeFailure)?;
        store
            .set_fuel(CALL_FUEL)
            .map_err(|_| RuntimeError::RuntimeFailure)?;
        let document = bindings
            .book_secure_agent_tools_workspace_reader()
            .call_read(&mut store, &arguments.path)
            .map_err(|error| classify_execution_error(&error, &store))?
            .map_err(|_| RuntimeError::ComponentDeclaredFailure)?;
        validate_document(
            &arguments.path,
            authorized_text,
            &document.path,
            &document.contents,
        )?;
        Ok(ToolResponse {
            tool: "workspace-read".to_owned(),
            content: document.contents.chars().take(MAX_OUTPUT_CHARS).collect(),
        })
    }

    fn call_normalizer(
        &self,
        operation: normalizer_types::Operation,
        text: &str,
        amount: u32,
    ) -> Result<Result<normalizer_types::Normalized, String>, RuntimeError> {
        let (mut store, bindings) = self
            .instantiate_normalizer()
            .map_err(|_| RuntimeError::RuntimeFailure)?;
        store
            .set_fuel(CALL_FUEL)
            .map_err(|_| RuntimeError::RuntimeFailure)?;
        bindings
            .book_secure_agent_tools_normalizer()
            .call_normalize(
                &mut store,
                &normalizer_types::Request {
                    operation,
                    text: text.to_owned(),
                    amount,
                },
            )
            .map_err(|error| classify_execution_error(&error, &store))
    }

    fn instantiate_normalizer(
        &self,
    ) -> Result<(Store<HostState>, normalizer_bindings::NormalizerTool)> {
        let mut store = new_store(&self.engine, WasiCtxBuilder::new().build())?;
        let bindings = normalizer_bindings::NormalizerTool::instantiate(
            &mut store,
            &self.normalizer,
            &self.normalizer_linker,
        )?;
        Ok((store, bindings))
    }

    fn instantiate_reader(
        &self,
        workspace: &Path,
    ) -> Result<(Store<HostState>, reader_bindings::WorkspaceReaderTool)> {
        let mut wasi = WasiCtxBuilder::new();
        wasi.preopened_dir(workspace, "/workspace", FsPerms::ReadOnly)?;
        let mut store = new_store(&self.engine, wasi.build())?;
        let bindings = reader_bindings::WorkspaceReaderTool::instantiate(
            &mut store,
            &self.reader,
            &self.reader_linker,
        )?;
        Ok((store, bindings))
    }

    fn typecheck(&self) -> Result<()> {
        self.instantiate_normalizer()?;
        let mut store = new_store(&self.engine, WasiCtxBuilder::new().build())?;
        reader_bindings::WorkspaceReaderTool::instantiate(
            &mut store,
            &self.reader,
            &self.reader_linker,
        )?;
        Ok(())
    }

    fn record(
        &self,
        stage: &'static str,
        outcome: &'static str,
        tool: Option<&str>,
        detail: &'static str,
    ) {
        let mut telemetry = self
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if telemetry.events.len() == EVENT_CAPACITY {
            telemetry.events.pop_front();
            telemetry.overwritten = telemetry.overwritten.saturating_add(1);
        }
        telemetry.events.push_back(Event {
            stage,
            outcome,
            tool: tool.map(ToolIdentity::from_name),
            detail,
        });
    }
}

impl ToolIdentity {
    fn from_name(name: &str) -> Self {
        match name {
            "normalize" => Self::Normalize,
            "workspace-read" => Self::WorkspaceRead,
            _ => Self::Unknown,
        }
    }
}

fn classify_execution_error(error: &wasmtime::Error, store: &Store<HostState>) -> RuntimeError {
    if store.data().memory_limit_denied {
        return RuntimeError::MemoryLimitDenied;
    }
    match error.downcast_ref::<Trap>() {
        Some(Trap::OutOfFuel) => RuntimeError::FuelExhausted,
        Some(_) => RuntimeError::GuestTrap,
        None => RuntimeError::RuntimeFailure,
    }
}

impl RuntimeError {
    fn detail(self) -> &'static str {
        match self {
            Self::MalformedRequest => "malformed-request",
            Self::UnknownTool => "unknown-tool",
            Self::DeniedTool => "denied-tool",
            Self::DeniedCapability => "denied-capability",
            Self::InvalidArguments => "invalid-arguments",
            Self::ArtefactRejected => "artefact-rejected",
            Self::FuelExhausted => "fuel-exhausted",
            Self::MemoryLimitDenied => "memory-limit-denied",
            Self::GuestTrap => "guest-trap",
            Self::RuntimeFailure => "runtime-failure",
            Self::InvalidOutput => "invalid-output",
            Self::ComponentDeclaredFailure => "component-declared-failure",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRequest {
    tool: String,
    arguments: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NormalizeArguments {
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArguments {
    path: String,
}

fn validate_text(text: &str) -> Result<(), RuntimeError> {
    if text.is_empty()
        || text.chars().count() > MAX_TEXT_CHARS
        || text.chars().any(char::is_control)
    {
        return Err(RuntimeError::InvalidArguments);
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<(), RuntimeError> {
    let candidate = Path::new(path);
    if path.is_empty()
        || path.len() > 128
        || candidate.is_absolute()
        || candidate
            .components()
            .any(|part| !matches!(part, PathComponent::Normal(_)))
        || candidate.extension().and_then(|value| value.to_str()) != Some("txt")
    {
        return Err(RuntimeError::InvalidArguments);
    }
    Ok(())
}

fn validate_normalized(text: &str, word_count: u32) -> Result<(), RuntimeError> {
    validate_text(text).map_err(|_| RuntimeError::InvalidOutput)?;
    if usize::try_from(word_count) != Ok(text.split_whitespace().count()) {
        return Err(RuntimeError::InvalidOutput);
    }
    Ok(())
}

fn validate_document(
    authorized_path: &str,
    authorized_contents: &str,
    returned_path: &str,
    returned_contents: &str,
) -> Result<(), RuntimeError> {
    if returned_path != authorized_path
        || returned_contents.len() > MAX_DOCUMENT_BYTES
        || returned_contents != authorized_contents
        || returned_contents
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(RuntimeError::InvalidOutput);
    }
    Ok(())
}

fn new_store(engine: &Engine, wasi: WasiCtx) -> Result<Store<HostState>> {
    let mut store = Store::new(
        engine,
        HostState {
            memory_limit_denied: false,
            table: ResourceTable::new(),
            wasi,
        },
    );
    store.limiter(|state| state);
    store.set_fuel(INSTANTIATION_FUEL)?;
    Ok(store)
}

fn manifest_tool<'a>(payload: &'a SignedPayload, name: &str) -> Result<&'a ToolManifest> {
    payload
        .tools
        .iter()
        .find(|tool| tool.name == name)
        .ok_or_else(|| anyhow!("missing tool"))
}

fn validate_manifest(payload: &SignedPayload) -> Result<()> {
    if payload.manifest_version != MANIFEST_VERSION || payload.tools.len() != 2 {
        bail!("unsupported manifest");
    }
    let normalizer = manifest_tool(payload, "normalize")?;
    let reader = manifest_tool(payload, "workspace-read")?;
    if normalizer.interface != "book:secure-agent-tools/normalizer@1.0.0"
        || !normalizer.capabilities.is_empty()
        || reader.interface != "book:secure-agent-tools/workspace-reader@1.0.0"
        || reader.capabilities != ["filesystem-read:workspace"]
    {
        bail!("manifest policy mismatch");
    }
    Ok(())
}

fn verified_component(
    repository_root: &Path,
    tool: &ToolManifest,
    engine: &Engine,
) -> Result<Component> {
    verified_component_with_hook(repository_root, tool, engine, |_| {})
}

fn verified_component_with_hook(
    repository_root: &Path,
    tool: &ToolManifest,
    engine: &Engine,
    after_verification: impl FnOnce(&Path),
) -> Result<Component> {
    let relative = Path::new(&tool.artifact);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, PathComponent::Normal(_)))
    {
        bail!("unsafe artifact path");
    }
    let root = repository_root.canonicalize()?;
    let artifact = root.join(relative).canonicalize()?;
    if !artifact.starts_with(&root) {
        bail!("artifact escaped repository");
    }
    let bytes = read_bounded(&artifact, MAX_COMPONENT_BYTES)?;
    let digest = hex_encode(&Sha256::digest(&bytes));
    if digest != tool.sha256 {
        bail!("artifact digest mismatch");
    }
    after_verification(&artifact);
    Component::new(engine, &bytes).map_err(Into::into)
}

fn verify_envelope(envelope: &ManifestEnvelope, public_key_path: &Path) -> Result<()> {
    let public = read_hex_file(public_key_path, MAX_PUBLIC_KEY_BYTES)?;
    let verifying_key = VerifyingKey::from_bytes(
        &public
            .try_into()
            .map_err(|_| anyhow!("public key must be 32 bytes"))?,
    )?;
    let signature_bytes = hex_decode(&envelope.signature)?;
    let signature = Signature::from_slice(&signature_bytes)?;
    verifying_key.verify(&signing_bytes(&envelope.signed)?, &signature)?;
    Ok(())
}

/// Returns the versioned, domain-separated RFC 8785/JCS bytes signed by
/// provisioning and verified while loading.
///
/// # Errors
///
/// Returns an error if the payload cannot be represented by the canonicalizer.
pub fn signing_bytes(payload: &SignedPayload) -> Result<Vec<u8>> {
    let canonical = serde_json_canonicalizer::to_vec(payload)?;
    let mut bytes = Vec::with_capacity(SIGNING_DOMAIN.len() + canonical.len());
    bytes.extend_from_slice(SIGNING_DOMAIN);
    bytes.extend_from_slice(&canonical);
    Ok(bytes)
}

/// Writes a deterministic, test-only signed manifest for already-built artifacts.
///
/// # Errors
///
/// Returns an error if keys or artifacts cannot be read or the manifest cannot be written.
pub fn provision_test_manifest(
    repository_root: &Path,
    output: &Path,
    secret_key_path: &Path,
) -> Result<()> {
    let secret = read_hex_file(secret_key_path, MAX_SECRET_KEY_BYTES)?;
    let signing_key = SigningKey::from_bytes(
        &secret
            .try_into()
            .map_err(|_| anyhow!("secret key must be 32 bytes"))?,
    );
    let tools = [
        (
            "normalize",
            "book:secure-agent-tools/normalizer@1.0.0",
            "target/wasm32-wasip2/debug/ch14_normalizer.wasm",
            Vec::new(),
        ),
        (
            "workspace-read",
            "book:secure-agent-tools/workspace-reader@1.0.0",
            "target/wasm32-wasip2/debug/ch14_workspace_reader.wasm",
            vec!["filesystem-read:workspace".to_owned()],
        ),
    ]
    .into_iter()
    .map(|(name, interface, artifact, capabilities)| {
        let bytes = read_bounded(&repository_root.join(artifact), MAX_COMPONENT_BYTES)
            .with_context(|| format!("read {artifact}; build the Wasm tools first"))?;
        Ok(ToolManifest {
            name: name.to_owned(),
            interface: interface.to_owned(),
            artifact: artifact.to_owned(),
            sha256: hex_encode(&Sha256::digest(bytes)),
            capabilities,
        })
    })
    .collect::<Result<Vec<_>>>()?;
    let signed = SignedPayload {
        manifest_version: MANIFEST_VERSION,
        tools,
    };
    let signature = signing_key.sign(&signing_bytes(&signed)?);
    let envelope = ManifestEnvelope {
        signed,
        signature: hex_encode(&signature.to_bytes()),
    };
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        output,
        format!("{}\n", serde_json::to_string_pretty(&envelope)?),
    )?;
    Ok(())
}

fn read_hex_file(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let bytes = read_bounded(path, limit)?;
    let text = std::str::from_utf8(&bytes)?;
    let value = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .ok_or_else(|| anyhow!("missing hex value"))?;
    hex_decode(value)
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    let mut bytes = Vec::with_capacity(limit.min(64 * 1024).saturating_add(1));
    file.take(u64::try_from(limit)?.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        bail!("input exceeds {limit} byte limit");
    }
    Ok(bytes)
}

fn hex_decode(value: &str) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        bail!("invalid hex length");
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).map_err(Into::into))
        .collect()
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    bytes.iter().fold(
        String::with_capacity(bytes.len() * 2),
        |mut output, byte| {
            write!(output, "{byte:02x}").expect("writing to a String cannot fail");
            output
        },
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};
    use wasmtime::{Config, Engine};

    use super::{
        MAX_COMPONENT_BYTES, MAX_MANIFEST_BYTES, MAX_PUBLIC_KEY_BYTES, RuntimeError, RuntimePolicy,
        SecureAgentRuntime, SignedPayload, ToolManifest, hex_encode, read_bounded, signing_bytes,
        validate_document, validate_relative_path, validate_text, verified_component_with_hook,
    };

    #[test]
    fn host_validation_rejects_control_text_and_unsafe_paths() {
        assert_eq!(
            validate_text("bad\u{0007}text"),
            Err(RuntimeError::InvalidArguments)
        );
        for path in [
            "../secret.txt",
            "/etc/passwd",
            "nested/../../x.txt",
            "notes.md",
        ] {
            assert_eq!(
                validate_relative_path(path),
                Err(RuntimeError::InvalidArguments)
            );
        }
        assert!(validate_relative_path("docs/runbook.txt").is_ok());
    }

    #[test]
    fn public_errors_are_generic() {
        assert_eq!(
            RuntimeError::FuelExhausted.to_string(),
            "tool request failed"
        );
    }

    #[test]
    fn bounded_admission_rejects_oversized_keys_manifests_and_components() {
        let directory = tempfile::tempdir().expect("create temporary directory");
        for (name, limit) in [
            ("public-key", MAX_PUBLIC_KEY_BYTES),
            ("manifest", MAX_MANIFEST_BYTES),
            ("component", MAX_COMPONENT_BYTES),
        ] {
            let path = directory.path().join(name);
            fs::write(&path, vec![0_u8; limit + 1]).expect("write oversized fixture");
            assert!(
                read_bounded(&path, limit).is_err(),
                "{name} must be rejected"
            );
        }
    }

    #[test]
    fn oversized_manifest_is_rejected_by_runtime_loading() {
        let directory = tempfile::tempdir().expect("create temporary directory");
        let manifest = directory.path().join("manifest.json");
        let public_key = directory.path().join("public.hex");
        fs::write(&manifest, vec![b' '; MAX_MANIFEST_BYTES + 1]).expect("write oversized manifest");
        fs::write(&public_key, "00").expect("write placeholder key");

        assert_eq!(
            SecureAgentRuntime::load(
                directory.path(),
                manifest,
                public_key,
                RuntimePolicy::normalize_only(),
            )
            .err(),
            Some(RuntimeError::ArtefactRejected)
        );
    }

    #[test]
    fn oversized_component_is_rejected_before_compilation() {
        let repository = tempfile::tempdir().expect("create temporary repository");
        let artifact_name = "oversized.wasm";
        let artifact_path = repository.path().join(artifact_name);
        let bytes = vec![0_u8; MAX_COMPONENT_BYTES + 1];
        fs::write(&artifact_path, &bytes).expect("write oversized component");
        let tool = ToolManifest {
            name: "normalize".to_owned(),
            interface: "book:secure-agent-tools/normalizer@1.0.0".to_owned(),
            artifact: artifact_name.to_owned(),
            sha256: hex_encode(&Sha256::digest(&bytes)),
            capabilities: Vec::new(),
        };
        let mut config = Config::new();
        config.wasm_component_model(true);
        let engine = Engine::new(&config).expect("create engine");

        assert!(super::verified_component(repository.path(), &tool, &engine).is_err());
    }

    #[test]
    fn malicious_reader_output_cannot_substitute_sibling_contents() {
        assert_eq!(
            validate_document(
                "runbook.txt",
                "authorized runbook\n",
                "runbook.txt",
                "sibling secret\n",
            ),
            Err(RuntimeError::InvalidOutput)
        );
        assert_eq!(
            validate_document(
                "runbook.txt",
                "authorized runbook\n",
                "sibling.txt",
                "authorized runbook\n",
            ),
            Err(RuntimeError::InvalidOutput)
        );
        assert_eq!(
            validate_document(
                "runbook.txt",
                "authorized runbook\n",
                "runbook.txt",
                "authorized runbook\n",
            ),
            Ok(())
        );
    }

    #[test]
    fn signing_bytes_and_signature_match_golden_vector() {
        let payload = SignedPayload {
            manifest_version: 1,
            tools: Vec::new(),
        };
        let expected = [
            b"production-webassembly-rust/ch14-manifest\0jcs-rfc8785\0v1\0".as_slice(),
            br#"{"manifest_version":1,"tools":[]}"#,
        ]
        .concat();
        let bytes = signing_bytes(&payload).expect("canonicalize payload");
        assert_eq!(bytes, expected);

        let signature = SigningKey::from_bytes(&[7_u8; 32]).sign(&bytes);
        assert_eq!(
            hex_encode(&signature.to_bytes()),
            concat!(
                "57f3a3681183bdfb422f582e8c09b47d02b0452a3312a66d9f8fbe7ba2ae21c2",
                "67387a709dc239d86ca35b73b29279eb52640ea7269e38232a172e5f99078306"
            )
        );
    }

    #[test]
    #[ignore = "requires the ch14 normalizer artifact; see case-study/README.md"]
    fn verified_bytes_are_compiled_when_the_path_changes_after_verification() {
        let repository = tempfile::tempdir().expect("create temporary repository");
        let artifact_name = "normalizer.wasm";
        let artifact_path = repository.path().join(artifact_name);
        fs::copy(
            workspace_root().join("target/wasm32-wasip2/debug/ch14_normalizer.wasm"),
            &artifact_path,
        )
        .expect("copy built normalizer");
        let verified_bytes = fs::read(&artifact_path).expect("read copied normalizer");
        let tool = ToolManifest {
            name: "normalize".to_owned(),
            interface: "book:secure-agent-tools/normalizer@1.0.0".to_owned(),
            artifact: artifact_name.to_owned(),
            sha256: hex_encode(&Sha256::digest(&verified_bytes)),
            capabilities: Vec::new(),
        };
        let mut config = Config::new();
        config.wasm_component_model(true);
        let engine = Engine::new(&config).expect("create component engine");

        let component =
            verified_component_with_hook(repository.path(), &tool, &engine, |verified_path| {
                fs::write(verified_path, b"unverified replacement")
                    .expect("replace artifact after verification");
            })
            .expect("compile the bytes that passed verification");

        drop(component);
        let replacement = fs::read(&artifact_path).expect("read replacement");
        assert!(
            wasmtime::component::Component::new(&engine, replacement).is_err(),
            "the replacement path must not contain a compilable component"
        );
    }

    fn workspace_root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("host crate should be nested under the workspace")
    }
}
