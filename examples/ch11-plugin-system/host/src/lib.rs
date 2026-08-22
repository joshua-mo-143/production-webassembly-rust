use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component as PathComponent, Path};

use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

const SUPPORTED_MAJOR: u64 = 1;
const INSTANTIATION_FUEL: u64 = 2_000_000;
const INVOCATION_FUEL: u64 = 2_000_000;
const MEMORY_LIMIT_BYTES: usize = 4 * 1024 * 1024;
/// Largest accepted manifest, measured in bytes.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
/// Largest accepted component artifact, measured in bytes.
pub const MAX_COMPONENT_BYTES: usize = 8 * 1024 * 1024;
/// Largest successful transform output, measured in bytes.
pub const MAX_PLUGIN_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_METADATA_NAME_BYTES: usize = 64;
const MAX_METADATA_VERSION_BYTES: usize = 32;

// Keeps the output ceiling the binding constraint. Raised above the memory
// ceiling, no guest could allocate an output large enough to reach it and the
// output policy would stop applying.
const _: () = assert!(MAX_PLUGIN_OUTPUT_BYTES < MEMORY_LIMIT_BYTES);

wasmtime::component::bindgen!({
    path: "../wit",
    world: "text-plugin",
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

struct LoadedPlugin {
    component: Component,
    version: String,
}

/// A verified set of plugins discovered from an explicit local manifest.
pub struct PluginRegistry {
    engine: Engine,
    linker: Linker<HostState>,
    plugins: BTreeMap<String, LoadedPlugin>,
}

/// A public failure that does not expose runtime or guest internals.
#[derive(Debug, PartialEq, Eq)]
pub enum PluginInvocationError {
    NotFound,
    Failed,
}

impl fmt::Display for PluginInvocationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("plugin not found"),
            Self::Failed => formatter.write_str("plugin invocation failed"),
        }
    }
}

impl std::error::Error for PluginInvocationError {}

impl PluginRegistry {
    /// Discovers only manifest entries inside `allowlisted_directory`, verifies
    /// each SHA-256 digest, type-checks it, and enforces the WIT major policy.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed manifests, path escapes, symlinks,
    /// digest mismatches, duplicate names, incompatible components, or runtime
    /// setup failures.
    pub fn load(
        allowlisted_directory: impl AsRef<Path>,
        manifest_path: impl AsRef<Path>,
    ) -> Result<Self> {
        let allowlisted_directory = allowlisted_directory.as_ref();
        let manifest_path = manifest_path.as_ref();
        let allowed = allowlisted_directory
            .canonicalize()
            .context("allowlisted plugin directory does not exist")?;
        ensure!(allowed.is_dir(), "plugin allowlist must be a directory");

        let mut config = Config::new();
        config.wasm_component_model(true);
        config.consume_fuel(true);
        let engine = Engine::new(&config)?;
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
        let manifest_bytes = read_bounded(manifest_path, MAX_MANIFEST_BYTES, "plugin manifest")
            .context("could not read plugin manifest")?;
        let manifest =
            String::from_utf8(manifest_bytes).context("plugin manifest is not valid UTF-8")?;
        let mut plugins = BTreeMap::new();

        for (index, line) in manifest.lines().enumerate() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line_number = index + 1;
            let mut fields = line.split('|');
            let name = fields.next().unwrap_or_default();
            let filename = fields.next().unwrap_or_default();
            let expected_digest = fields.next().unwrap_or_default();
            ensure!(
                fields.next().is_none()
                    && valid_name(name)
                    && valid_filename(filename)
                    && valid_digest(expected_digest),
                "invalid manifest entry on line {line_number}"
            );
            ensure!(
                !plugins.contains_key(name),
                "duplicate plugin name on line {line_number}"
            );

            let candidate = allowed.join(filename);
            let metadata = fs::symlink_metadata(&candidate)
                .with_context(|| format!("plugin artifact is missing on line {line_number}"))?;
            ensure!(
                metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
                "plugin artifact must be a regular non-symlink file"
            );
            let canonical = candidate.canonicalize()?;
            ensure!(
                canonical.parent() == Some(allowed.as_path()),
                "plugin artifact escaped the allowlisted directory"
            );
            let bytes = read_bounded(&canonical, MAX_COMPONENT_BYTES, "plugin component")?;
            ensure!(
                sha256_hex(&bytes) == expected_digest,
                "SHA-256 verification failed for plugin {name}"
            );
            let component = Component::new(&engine, &bytes).map_err(|error| {
                anyhow::anyhow!("plugin {name} is not a compatible component: {error}")
            })?;
            let reported = inspect_metadata(&engine, &linker, &component)
                .with_context(|| format!("plugin {name} metadata failed"))?;
            validate_metadata(&reported)
                .with_context(|| format!("plugin {name} metadata is invalid"))?;
            ensure!(
                reported.name == name,
                "plugin identity does not match manifest"
            );
            ensure!(
                parse_major(&reported.version) == Some(SUPPORTED_MAJOR),
                "plugin {name} has an unsupported contract major"
            );
            plugins.insert(
                name.to_owned(),
                LoadedPlugin {
                    component,
                    version: reported.version,
                },
            );
        }
        ensure!(!plugins.is_empty(), "manifest contains no plugins");
        Ok(Self {
            engine,
            linker,
            plugins,
        })
    }

    /// Lists verified plugin names and their self-reported compatible versions.
    #[must_use]
    pub fn plugins(&self) -> Vec<(&str, &str)> {
        self.plugins
            .iter()
            .map(|(name, plugin)| (name.as_str(), plugin.version.as_str()))
            .collect()
    }

    /// Invokes a plugin in a fresh store so a trap cannot poison later calls.
    ///
    /// # Errors
    ///
    /// Returns `NotFound` for an unknown manifest name and `Failed` for any
    /// contained instantiation, resource-limit, trap, or runtime failure.
    pub fn invoke(&self, name: &str, input: &str) -> Result<String, PluginInvocationError> {
        let plugin = self
            .plugins
            .get(name)
            .ok_or(PluginInvocationError::NotFound)?;
        invoke_component(&self.engine, &self.linker, &plugin.component, input)
            .map_err(|_internal| PluginInvocationError::Failed)
    }
}

/// Computes the lowercase SHA-256 value used by the local manifest.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn inspect_metadata(
    engine: &Engine,
    linker: &Linker<HostState>,
    component: &Component,
) -> Result<Metadata> {
    let (mut store, bindings) = instantiate(engine, linker, component)?;
    store.set_fuel(INVOCATION_FUEL)?;
    Ok(bindings.call_metadata(&mut store)?)
}

fn invoke_component(
    engine: &Engine,
    linker: &Linker<HostState>,
    component: &Component,
    input: &str,
) -> Result<String> {
    let (mut store, bindings) = instantiate(engine, linker, component)?;
    store.set_fuel(INVOCATION_FUEL)?;
    let output = bindings.call_transform(&mut store, input)?;
    validate_text(
        &output,
        MAX_PLUGIN_OUTPUT_BYTES,
        "plugin output exceeds byte limit",
        "plugin output contains unsafe control characters",
    )?;
    Ok(output)
}

fn read_bounded(path: &Path, limit: usize, description: &str) -> Result<Vec<u8>> {
    let file = File::open(path)?;
    let read_limit = u64::try_from(limit)
        .expect("configured byte limits fit in u64")
        .saturating_add(1);
    let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
    file.take(read_limit).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "{description} exceeds {limit}-byte limit"
    );
    Ok(bytes)
}

fn validate_metadata(metadata: &Metadata) -> Result<()> {
    ensure!(
        metadata.name.len() <= MAX_METADATA_NAME_BYTES,
        "plugin metadata name exceeds byte limit"
    );
    ensure!(
        !metadata.name.chars().any(char::is_control),
        "plugin metadata name contains control characters"
    );
    ensure!(
        metadata.version.len() <= MAX_METADATA_VERSION_BYTES,
        "plugin metadata version exceeds byte limit"
    );
    ensure!(
        !metadata.version.chars().any(char::is_control),
        "plugin metadata version contains control characters"
    );
    Ok(())
}

fn validate_text(
    value: &str,
    max_bytes: usize,
    length_error: &str,
    control_error: &str,
) -> Result<()> {
    ensure!(value.len() <= max_bytes, "{length_error}");
    ensure!(!has_unsafe_control(value), "{control_error}");
    Ok(())
}

fn has_unsafe_control(value: &str) -> bool {
    value
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\t' | '\n' | '\r'))
}

fn instantiate(
    engine: &Engine,
    linker: &Linker<HostState>,
    component: &Component,
) -> Result<(Store<HostState>, TextPlugin)> {
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
            wasi: WasiCtxBuilder::new().build(),
        },
    );
    store.limiter(|state| &mut state.limits);
    store.set_fuel(INSTANTIATION_FUEL)?;
    let bindings = TextPlugin::instantiate(&mut store, component, linker)?;
    Ok((store, bindings))
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn valid_filename(filename: &str) -> bool {
    let path = Path::new(filename);
    let mut components = path.components();
    matches!(components.next(), Some(PathComponent::Normal(_)))
        && components.next().is_none()
        && path
            .extension()
            .is_some_and(|extension| extension == "wasm")
}

fn valid_digest(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn parse_major(version: &str) -> Option<u64> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    parts.next()?.parse::<u64>().ok()?;
    parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(major)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        MAX_METADATA_NAME_BYTES, Metadata, PluginRegistry, parse_major, sha256_hex, valid_digest,
        valid_filename, valid_name, validate_metadata,
    };

    #[test]
    fn manifest_fields_are_strict() {
        assert!(valid_name("uppercase"));
        assert!(!valid_name("../plugin"));
        assert!(valid_filename("plugin.wasm"));
        assert!(!valid_filename("../plugin.wasm"));
        assert!(!valid_filename("nested/plugin.wasm"));
        assert!(valid_digest(&"a".repeat(64)));
        assert!(!valid_digest(&"A".repeat(64)));
    }

    #[test]
    fn compatibility_requires_three_numeric_parts() {
        assert_eq!(parse_major("1.1.0"), Some(1));
        assert_eq!(parse_major("1"), None);
        assert_eq!(parse_major("1.0.0-extra"), None);
    }

    #[test]
    fn metadata_is_length_bounded_and_control_safe() {
        let oversized = Metadata {
            name: "x".repeat(MAX_METADATA_NAME_BYTES + 1),
            version: "1.0.0".to_owned(),
        };
        assert!(
            validate_metadata(&oversized)
                .expect_err("oversized metadata must fail")
                .to_string()
                .contains("name exceeds byte limit")
        );

        let unsafe_control = Metadata {
            name: "uppercase".to_owned(),
            version: "1.0\u{7f}.0".to_owned(),
        };
        assert!(
            validate_metadata(&unsafe_control)
                .expect_err("control characters must fail")
                .to_string()
                .contains("version contains control")
        );
    }

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn loader_rejects_path_traversal_before_artifact_access() {
        let directory = tempfile::tempdir().expect("temporary allowlist should be created");
        let manifest = directory.path().join("plugins.manifest");
        fs::write(
            &manifest,
            format!("escape|../escape.wasm|{}\n", "a".repeat(64)),
        )
        .expect("manifest fixture should be written");

        let error = PluginRegistry::load(directory.path(), manifest)
            .err()
            .expect("path traversal must fail closed");
        assert!(error.to_string().contains("invalid manifest entry"));
    }
}
