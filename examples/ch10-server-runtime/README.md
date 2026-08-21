# Chapter 10: embedding components in a server runtime

Stable book link:
<https://github.com/joshua-mo-143/production-webassembly-rust/tree/main/examples/ch10-server-runtime>

`ServerRuntime::load` compiles the component once into Wasmtime's in-memory
`Component` representation. Every request reuses that cached compiled component
and linker but receives a fresh store with its own fuel and memory limits. The
example invokes requests directly instead of opening a socket, so runs and
tests terminate deterministically and require no external service.

Run all commands from the repository root.

## Build and run

```fish
cargo build --target wasm32-wasip2 -p ch10-guest
cargo run -p ch10-host
```

Expected output:

```text
component_cache: compiled=1
{"request_id":1,"outcome":"ok","status":200,"error_code":null}
response: status=200 body=SERVER COMPONENT
{"request_id":2,"outcome":"rejected","status":400,"error_code":"component_rejected"}
response: status=400 body=request rejected
{"request_id":3,"outcome":"error","status":503,"error_code":"component_runtime_failure"}
response: status=503 body=service temporarily unavailable
```

Run the cached-loading, limits, error-mapping, and recovery integration test:

```fish
env CH10_COMPONENT=target/wasm32-wasip2/debug/ch10_guest.wasm \
 cargo test -p ch10-host --test server_runtime -- --ignored
```

Expected result:

```text
test cached_component_handles_requests_with_fresh_limits ... ok
```

## Architecture and operations

- Process startup constructs one engine and linker and compiles the component
  once. Production deployments can additionally enable Wasmtime's on-disk
  cache or distribute precompiled artifacts, but precompiled artifacts must be
  trusted, runtime-version-matched, and loaded through Wasmtime's documented
  unsafe contract. This example stays on the safe bytecode-loading API.
- Every invocation instantiates a fresh store. Stores are not shared across
  concurrent requests or reused after a trap.
- Fuel bounds guest execution deterministically; it is not a wall-clock
  deadline. A real server should combine fuel with epoch interruption,
  cancellation, queue limits, and process-level memory accounting.
- The store limiter caps each linear memory at 4 MiB and limits selected store
  resources. It does not cap host allocations or total process memory.
- Structured events use numeric request IDs and low-cardinality outcome and
  error-code fields. Request paths, bodies, guest errors, and trap details are
  deliberately excluded to avoid logging sensitive or attacker-controlled
  data.

## Safe error boundary

The component's explicit domain rejection becomes a generic HTTP-style `400`.
Traps, fuel exhaustion, instantiation failures, and other runtime faults become
a generic `503`. Internal guest and runtime messages do not enter the public
response. A production host should retain detailed errors only in access-
controlled diagnostics, attach trace context, validate component status codes
and output sizes, and distinguish retryable failures according to its own
policy.

This is a synchronous embedding example, not a complete HTTP server, scheduler,
load balancer, or observability pipeline.
