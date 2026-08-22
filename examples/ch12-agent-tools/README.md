# Chapter 12: secure AI-agent tool boundaries

Development-branch repository link:
<https://github.com/joshua-mo-143/production-webassembly-rust/tree/main/examples/ch12-agent-tools>

This example accepts a representative JSON tool request without calling a
model, network service, or subprocess. The host parses JSON into a
`deny_unknown_fields` schema, checks the tool and every argument against an
allowlist, and sends only the query to a Wasm component for deterministic
whitespace normalization. The credential, document lookup, decision to invoke,
and public response stay in trusted host code.

Run all commands from the repository root.

## Build and run

```fish
cargo build --target wasm32-wasip2 -p ch12-guest
cargo run -p ch12-host
```

Expected output:

```text
validated tool: local-document-search
sanitized result: handbook: deployment secrets => Deployments require review. | Never print secrets.
fuel exhaustion: FuelExhausted
memory denial: MemoryLimitDenied
external calls: 0
```

Run the native schema and sanitizer tests plus the artifact-backed boundary
and fuel tests:

```fish
cargo test -p ch12-host
env CH12_COMPONENT=target/wasm32-wasip2/debug/ch12_guest.wasm \
 cargo test -p ch12-host --test agent_tools -- --ignored
```

Expected result:

```text
test result: ok. 4 passed; 0 failed; 0 ignored
test component_resource_exhaustion_is_contained ... ok
test runtime_failure_is_non_trap_and_healthy_component_still_loads ... ok
test secure_boundary_rejects_bad_requests_and_executes_allowed_tool ... ok
```

The integration tests cover malformed JSON and unknown fields, denied tool
names, denied document IDs, successful typed component execution, output
sanitization, deliberate fuel exhaustion, configured memory-limit denial, a
guest trap, component-declared rejection, invalid component output, a
deterministic non-trap component-load `RuntimeFailure`, and healthy positive
controls after both execution and load failures. Probe operations are not
representable in the accepted JSON schema; they are exposed only through the
host's test API.

## Trust boundary

The WIT contract carries an operation, text, and numeric amount into the
component and returns a typed `result` containing a `prepared` record or
`rejection` variant. Production request handling always constructs
`operation.normalize` itself. A model-shaped request cannot select the
fuel-burning or allocation probes.

The component has no application-specific host imports. The configured WASI
context does not inherit environment variables or stdio, preopen directories,
or expose network sockets. In particular, the host-owned `ToolCredential`
never enters a WIT value.

The host checks the component's normalized text and token estimate before
executing a deterministic in-memory lookup. It then removes control characters
and caps the public output. Internally, the boundary keeps exact categories for
`FuelExhausted`, `MemoryLimitDenied`, `GuestTrap`, `RuntimeFailure`,
`DeniedCapability`, `ComponentDeclaredFailure`, and
`InvalidComponentOutput`. Artifact-backed tests assert each category exercised
by the fixture; the load test establishes `RuntimeFailure` without a trap.
Healthy controls follow the execution failures and the failed load. A trap
therefore cannot pass as evidence that invalid output was rejected.

## Security and operational caveats

- This is an execution-boundary example, not an agent framework, prompt
  injection detector, authorization system, or model integration. Tool policy
  must be based on authenticated user and tenant context in a real service.
- Fuel bounds Wasm instructions, not wall-clock time. The per-memory limit is
  not a process-wide memory quota, and neither control replaces admission
  control, deadlines, or operating-system isolation.
- The demo credential is a value object with a fixed string so the example is
  reproducible. Real credentials need a secret manager, scoped permissions,
  rotation, audit logging, and redaction.
- Output sanitization here removes control characters and truncates by Unicode
  scalar count. Output sent into HTML, SQL, shells, logs, or another model
  still needs context-specific encoding and policy checks.
- There are no external calls. The local search corpus is hard-coded so tests
  are deterministic and safe to run offline.

For the component security model, see the stable
[WebAssembly Component Model repository](https://github.com/WebAssembly/component-model).
For Wasmtime resource limiting, see the pinned
[Wasmtime v48.0.0 `Store` source](https://github.com/bytecodealliance/wasmtime/blob/v48.0.0/crates/wasmtime/src/runtime/store.rs).
