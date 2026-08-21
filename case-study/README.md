# Chapter 14 case study: a secure AI-agent tool runtime

Development-branch repository link:
<https://github.com/joshua-mo-143/production-webassembly-rust/tree/main/case-study>

This case study combines the book's stable WIT boundary, typed data exchange,
resource containment, capability-based WASI, fresh-store request isolation,
authenticated plug-in loading, host-owned agent policy, and multi-component
composition. The “model” is deliberately a set of deterministic JSON fixtures:
there are no network calls or external services.

## Architecture

```mermaid
flowchart LR
    F["Deterministic model fixture"] --> P["Host parser<br/>deny unknown fields"]
    P --> G["Host policy<br/>tool and capability grants"]
    M["Signed manifest"] --> V["Ed25519 verification"]
    K["Trusted public key"] --> V
    C["Wasm component bytes"] --> H["SHA-256 artefact verification"]
    V --> H --> G
    G -->|"no filesystem"| N["Normalizer component<br/>WIT 1.0.0"]
    G -->|"read-only /workspace"| R["Workspace reader component<br/>WIT 1.0.0"]
    N --> O["Host output validation"]
    R --> O
    Q["Host-only credential"] --> G
    O --> U["Bounded public result"]
    P --> E["Structured internal events"]
    G --> E
    O --> E
```

The Rust host is the security boundary. It owns request parsing, schemas,
allowlists, credentials, capability grants, resource limits, artefact trust,
telemetry, and output validation. Components receive only typed WIT values and
their explicit WASI capabilities. Each call uses a fresh store with fuel and
memory limits. The normalizer gets an empty WASI context; the reader gets one
read-only preopen mapped to `/workspace`.

The checked-in Ed25519 key pair is an
[RFC 8032 test vector](https://www.rfc-editor.org/rfc/rfc8032#section-7.1) and is
marked **TEST ONLY**. `provision` hashes the two built components, signs the
canonical serialized payload, and writes `target/ch14/manifest.json`. Loading
first verifies the signature against the separately supplied public key, then
checks manifest policy, safe relative artefact paths, SHA-256 digests, and WIT
compatibility. Each component is read exactly once into host-owned memory; the
host hashes that byte buffer and passes the same buffer to Wasmtime for
compilation, so replacing the path between verification and compilation cannot
substitute unverified code. See the pinned
[Wasmtime v48.0.0 source](https://github.com/bytecodealliance/wasmtime/tree/v48.0.0/crates/wasmtime) and
[ed25519-dalek 2.2.0 API](https://docs.rs/ed25519-dalek/2.2.0/ed25519_dalek/).
The implementation references stable GitHub tags for
[Wasmtime v48.0.0](https://github.com/bytecodealliance/wasmtime/tree/v48.0.0)
and
[wit-bindgen v0.60.0](https://github.com/bytecodealliance/wit-bindgen/tree/v0.60.0).

## Threat model

| Threat | Control | Residual risk |
| --- | --- | --- |
| Model emits unexpected JSON, tools, or arguments | Size limit, `deny_unknown_fields`, per-tool schemas, allowlists | A real model remains untrusted and needs the same boundary |
| Tool asks for ambient filesystem or network access | Empty WASI context or one read-only preopen; no socket grants | WASI/runtime vulnerabilities remain in the trusted computing base |
| `../`, absolute path, or non-document access | Host accepts only normal relative `.txt` path components; WASI confines lookup to the preopen | Allowed workspace files are intentionally disclosed |
| Component or manifest is replaced | Ed25519 signature, SHA-256 artefact digest, compile directly from the verified in-memory bytes, trusted public-key input, typed WIT instantiation | Test key has no production secrecy; key distribution and rollback protection are deployment concerns |
| Guest loops, allocates excessively, traps, or poisons state | Fuel, store limits, trap-on-growth failure, fresh stores | Fuel is deterministic work accounting, not a wall-clock deadline |
| Guest returns misleading or hostile output | Host validates identity, lengths, controls, and semantic counts; public output is truncated | Domain-specific validation must evolve with each tool |
| Errors leak paths, policy, or internals | Public `Display` is always `tool request failed`; structured categories stay internal | Operators must protect telemetry |
| Credential reaches untrusted code | Credential is host-owned and checked before dispatch; it is absent from WIT and store state | Host memory and process compromise are out of scope |

Trusted inputs are the host binary, configured public key, Wasmtime, operating
system, and the selected workspace directory. The model fixture, manifest file,
component files, tool arguments, and component outputs are untrusted.

## Build and run

Run these exact Fish commands from the repository root. They use only Cargo and
the repository files; do not install privileged packages.

```fish
cargo build --target wasm32-wasip2 \
 -p ch14-normalizer -p ch14-workspace-reader
cargo run -p ch14-host -- provision
cargo run -p ch14-host -- run
```

Expected output (Cargo build lines omitted):

```text
wrote authenticated test manifest: target/ch14/manifest.json
normalize: review then deploy
workspace-read: Deployments require review, a bounded rollout, and a health check.
limits: fuel=true memory=true invalid-output=true
structured-events: 2
network-calls: 0
```

Run the complete verification:

```fish
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --target wasm32-wasip2 \
 -p ch14-normalizer -p ch14-workspace-reader -- \
 -D warnings -A clippy::same-length-and-capacity
cargo build --target wasm32-wasip2 \
 -p ch14-normalizer -p ch14-workspace-reader
cargo run -p ch14-host -- provision
cargo test -p ch14-host -- --ignored
cargo test --workspace
```

The narrow Clippy exception covers code generated by `wit-bindgen`; handwritten
guest modules still deny unsafe code through the workspace lint.

## Test matrix

| Case | Expected result |
| --- | --- |
| Normalization and read-only workspace read | Typed, validated success |
| Unknown tool | `UnknownTool`; generic public message |
| Known but ungranted tool | `DeniedTool` |
| Tool lacks its workspace grant | `DeniedCapability` |
| Malformed JSON, extra fields, traversal, wrong extension | Rejected before component execution |
| Signed manifest changed | Signature verification rejects load |
| Component bytes changed | Digest verification rejects load |
| Component path replaced after digest verification | Previously read, verified bytes still compile; invalid replacement is unused |
| Fuel loop and oversized allocation | Trapped within the call's fresh store |
| Semantically invalid/control-character output | Host output validation rejects it |
| Healthy request after traps | Succeeds in a fresh store |

## Operational caveats

- Replace both test keys and the provisioning process. Keep the signing key
  offline; distribute the public key independently of manifests and artefacts.
- Add expiry, build identity, monotonic version/rollback protection, and an
  atomic release process before production deployment.
- Verification and compilation are race-free with respect to component
  contents, but production path resolution should additionally use an
  OS-specific immutable descriptor strategy such as Linux `openat2` with
  no-symlink constraints if avoiding every path-resolution race is required.
- Pin and audit the runtime and cryptographic dependency supply chain. The
  lockfile captures this example's resolved versions but is not a vulnerability
  management process.
- Add an epoch interruption or outer process deadline when wall-clock latency
  matters. Fuel alone does not bound blocking host operations.
- Use a dedicated, minimally populated directory for each workspace grant.
  Read-only prevents mutation, not disclosure of every file in that preopen.
- Forward structured events to a protected sink with retention and redaction;
  this example keeps them in memory and records no argument or credential data.
