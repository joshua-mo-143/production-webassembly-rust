# Chapter 11: a verified third-party plugin system

Development-branch repository link:
<https://github.com/joshua-mo-143/production-webassembly-rust/tree/main/examples/ch11-plugin-system>

The contract is the versioned WIT package `book:plugin@1.0.0`. The host
discovers plugins only from an explicit local manifest and allowlisted
directory, verifies each artifact's SHA-256 digest before compiling it, checks
the plugin identity and supported major version, and invokes every call in a
fresh resource-limited store. There are no downloads, credentials, or external
services.

Run all commands from the repository root.

## Build and run

```fish
cargo build --target wasm32-wasip2 \
 -p ch11-plugin-v1 -p ch11-plugin-v1-1
cargo run -p ch11-host
```

Expected output:

```text
verified uppercase 1.0.0 sha256=ok
v1 output: PLUGIN
v1 trap: contained (plugin invocation failed)
verified uppercase 1.1.0 sha256=ok
upgrade output: PLUGIN 1.1
```

Run verification, containment, and upgrade compatibility tests:

```fish
env CH11_PLUGIN_V1=target/wasm32-wasip2/debug/ch11_plugin_v1.wasm \
 CH11_PLUGIN_V1_1=target/wasm32-wasip2/debug/ch11_plugin_v1_1.wasm \
 cargo test -p ch11-host --test plugin_system -- --ignored
```

Expected result: both tests pass:

```text
test v1_1_upgrade_remains_compatible_with_v1_contract ... ok
test verification_and_failure_containment_fail_closed ... ok
```

## Manifest and upgrade policy

The intentionally small local manifest format has one entry per line:

```text
plugin-name|artifact.wasm|64-lowercase-hex-sha256
```

Names are restricted to ASCII letters, digits, `_`, and `-`. Artifact names
must be a single `.wasm` filename: absolute paths, nested paths, `..`, and
symlinks are rejected. Files must resolve directly inside the canonical
allowlisted directory. Duplicate names and empty manifests fail closed.

Both fixture plugins implement the same WIT 1.0 contract. The second reports
implementation version 1.1.0 and adds compatible behavior without changing the
contract shape. Loading it through the unchanged host bindings is the
compatibility test. A breaking WIT change requires a new package major and a
deliberate host migration; a reported version string cannot make incompatible
component types compatible.

## Security and operational caveats

- SHA-256 checks integrity against the supplied manifest; it does not establish
  publisher identity or trust. Production systems should authenticate a signed
  manifest or transparency-log record, protect rollback metadata, and secure
  the provisioning path.
- Verification occurs before Wasmtime compiles the bytes. The example copies
  artifacts into temporary allowlisted directories for a deterministic demo.
  Production installers should use atomic, immutable placement and defend
  against filesystem races with platform-appropriate handles and permissions.
- A trap returns only `plugin invocation failed`; guest and runtime details do
  not cross the public API. The trapped store is discarded, and a fresh store
  successfully serves the next call.
- Fuel and a per-memory limit bound selected guest resources. They are not a
  process-wide quota, elapsed-time deadline, admission-control policy, or
  substitute for operating-system isolation.
- The host grants no filesystem, environment, stdio, or network capabilities.
  Adding imports expands the trusted host API and must be reviewed explicitly.

For the digest definition, see the stable
[FIPS 180-4 publication](https://csrc.nist.gov/pubs/fips/180-4/upd1/final).
For component contracts, see the
[WebAssembly Component Model repository](https://github.com/WebAssembly/component-model).
