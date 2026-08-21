use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Component as PathComponent, Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow, bail};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{FsPerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

const MANIFEST_VERSION: u32 = 1;
const INSTANTIATION_FUEL: u64 = 3_000_000;
const CALL_FUEL: u64 = 100_000;
const MEMORY_LIMIT_BYTES: usize = 4 * 1024 * 1024;
const MAX_REQUEST_BYTES: usize = 2_048;
const MAX_TEXT_CHARS: usize = 256;
const MAX_DOCUMENT_BYTES: usize = 16 * 1024;
const MAX_OUTPUT_CHARS: usize = 1_024;

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
    pub tool: Option<String>,
    pub detail: &'static str,
}

#[derive(Debug, Clone)]
pub struct RuntimePolicy {
    allowed_tools: BTreeSet<String>,
    workspace_root: Option<PathBuf>,
}

impl RuntimePolicy {
    #[must_use]
    pub fn allow_all(workspace_root: &Path) -> Self {
        Self {
            allowed_tools: ["normalize", "workspace-read"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            workspace_root: Some(workspace_root.to_owned()),
        }
    }

    #[must_use]
    pub fn normalize_only() -> Self {
        Self {
            allowed_tools: ["normalize"].into_iter().map(str::to_owned).collect(),
            workspace_root: None,
        }
    }

    #[must_use]
    pub fn without_workspace_grant() -> Self {
        Self {
            allowed_tools: ["normalize", "workspace-read"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            workspace_root: None,
        }
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
    ResourceLimit,
    InvalidOutput,
    ToolFailure,
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("tool request failed")
    }
}

impl std::error::Error for RuntimeError {}

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

pub struct SecureAgentRuntime {
    engine: Engine,
    normalizer: Component,
    reader: Component,
    normalizer_linker: Linker<HostState>,
    reader_linker: Linker<HostState>,
    policy: RuntimePolicy,
    events: Mutex<Vec<Event>>,
}

impl SecureAgentRuntime {
    /// Verifies the signed manifest and every component before compiling them.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::ArtefactRejected`] for invalid signatures,
    /// hashes, paths, interfaces, or components.
    pub fn load(
        repository_root: &Path,
        manifest_path: &Path,
        public_key_path: &Path,
        policy: RuntimePolicy,
    ) -> Result<Self, RuntimeError> {
        let result = Self::load_verified(repository_root, manifest_path, public_key_path, policy);
        result.map_err(|_| RuntimeError::ArtefactRejected)
    }

    fn load_verified(
        repository_root: &Path,
        manifest_path: &Path,
        public_key_path: &Path,
        policy: RuntimePolicy,
    ) -> Result<Self> {
        let envelope: ManifestEnvelope =
            serde_json::from_slice(&fs::read(manifest_path).context("read manifest")?)
                .context("parse manifest")?;
        verify_envelope(&envelope, public_key_path)?;
        validate_manifest(&envelope.signed)?;

        let normalizer_entry = manifest_tool(&envelope.signed, "normalize")?;
        let reader_entry = manifest_tool(&envelope.signed, "workspace-read")?;
        let normalizer_path = verified_artifact(repository_root, normalizer_entry)?;
        let reader_path = verified_artifact(repository_root, reader_entry)?;

        let mut config = Config::new();
        config.wasm_component_model(true);
        config.consume_fuel(true);
        let engine = Engine::new(&config)?;
        let normalizer = Component::from_file(&engine, normalizer_path)?;
        let reader = Component::from_file(&engine, reader_path)?;
        let mut normalizer_linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut normalizer_linker)?;
        let mut reader_linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut reader_linker)?;

        let runtime = Self {
            engine,
            normalizer,
            reader,
            normalizer_linker,
            reader_linker,
            policy,
            events: Mutex::new(Vec::new()),
        };
        runtime.typecheck()?;
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
    pub fn events(&self) -> Vec<Event> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    #[must_use]
    pub fn prove_fuel_limit(&self) -> bool {
        self.call_normalizer(normalizer_types::Operation::BurnFuel, "probe", 10_000_000)
            .is_err()
    }

    #[must_use]
    pub fn prove_memory_limit(&self) -> bool {
        self.call_normalizer(
            normalizer_types::Operation::GrowMemory,
            "probe",
            32 * 1024 * 1024,
        )
        .is_err()
    }

    pub fn prove_invalid_output_rejected(&self) -> bool {
        match self.call_normalizer(normalizer_types::Operation::InvalidOutput, "probe", 0) {
            Ok(Ok(value)) => validate_normalized(&value.text, value.word_count).is_err(),
            Ok(Err(_)) | Err(_) => true,
        }
    }

    fn execute_normalize(
        &self,
        arguments: serde_json::Value,
    ) -> Result<ToolResponse, RuntimeError> {
        let arguments: NormalizeArguments =
            serde_json::from_value(arguments).map_err(|_| RuntimeError::InvalidArguments)?;
        validate_text(&arguments.text)?;
        let output = self
            .call_normalizer(normalizer_types::Operation::Normalize, &arguments.text, 0)
            .map_err(|_| RuntimeError::ResourceLimit)?
            .map_err(|_| RuntimeError::ToolFailure)?;
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
        let (mut store, bindings) = self
            .instantiate_reader(workspace)
            .map_err(|_| RuntimeError::DeniedCapability)?;
        store
            .set_fuel(CALL_FUEL)
            .map_err(|_| RuntimeError::ResourceLimit)?;
        let document = bindings
            .book_secure_agent_tools_workspace_reader()
            .call_read(&mut store, &arguments.path)
            .map_err(|_| RuntimeError::ResourceLimit)?
            .map_err(|_| RuntimeError::ToolFailure)?;
        if document.path != arguments.path
            || document.contents.len() > MAX_DOCUMENT_BYTES
            || document
                .contents
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
        {
            return Err(RuntimeError::InvalidOutput);
        }
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
    ) -> Result<Result<normalizer_types::Normalized, String>> {
        let (mut store, bindings) = self.instantiate_normalizer()?;
        store.set_fuel(CALL_FUEL)?;
        Ok(bindings
            .book_secure_agent_tools_normalizer()
            .call_normalize(
                &mut store,
                &normalizer_types::Request {
                    operation,
                    text: text.to_owned(),
                    amount,
                },
            )?)
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
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(Event {
                stage,
                outcome,
                tool: tool.map(str::to_owned),
                detail,
            });
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
            Self::ResourceLimit => "resource-limit",
            Self::InvalidOutput => "invalid-output",
            Self::ToolFailure => "tool-failure",
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

fn new_store(engine: &Engine, wasi: WasiCtx) -> Result<Store<HostState>> {
    let limits = StoreLimitsBuilder::new()
        .memory_size(MEMORY_LIMIT_BYTES)
        .instances(10)
        .tables(10)
        .memories(2)
        .trap_on_grow_failure(true)
        .build();
    let mut store = Store::new(
        engine,
        HostState {
            limits,
            table: ResourceTable::new(),
            wasi,
        },
    );
    store.limiter(|state| &mut state.limits);
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

fn verified_artifact(repository_root: &Path, tool: &ToolManifest) -> Result<PathBuf> {
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
    let digest = hex_encode(&Sha256::digest(fs::read(&artifact)?));
    if digest != tool.sha256 {
        bail!("artifact digest mismatch");
    }
    Ok(artifact)
}

fn verify_envelope(envelope: &ManifestEnvelope, public_key_path: &Path) -> Result<()> {
    let public = read_hex_file(public_key_path)?;
    let verifying_key = VerifyingKey::from_bytes(
        &public
            .try_into()
            .map_err(|_| anyhow!("public key must be 32 bytes"))?,
    )?;
    let signature_bytes = hex_decode(&envelope.signature)?;
    let signature = Signature::from_slice(&signature_bytes)?;
    verifying_key.verify(&serde_json::to_vec(&envelope.signed)?, &signature)?;
    Ok(())
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
    let secret = read_hex_file(secret_key_path)?;
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
        let bytes = fs::read(repository_root.join(artifact))
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
    let signature = signing_key.sign(&serde_json::to_vec(&signed)?);
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

fn read_hex_file(path: &Path) -> Result<Vec<u8>> {
    let text = fs::read_to_string(path)?;
    let value = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .ok_or_else(|| anyhow!("missing hex value"))?;
    hex_decode(value)
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
    use super::{RuntimeError, validate_relative_path, validate_text};

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
            RuntimeError::ResourceLimit.to_string(),
            "tool request failed"
        );
    }
}
