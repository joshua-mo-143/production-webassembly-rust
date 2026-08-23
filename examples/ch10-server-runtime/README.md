# Chapter 10: embedding components in a server runtime

Development-branch repository link:
<https://github.com/joshua-mo-143/production-webassembly-rust/tree/main/examples/ch10-server-runtime>

`ServerRuntime::load` compiles the component once into Wasmtime's in-memory
`Component` representation. Every request reuses that cached compiled component
and linker but receives a fresh store with its own fuel and memory limits. The
default binary invokes requests directly and terminates deterministically. A
second `ch10-http-server` binary exposes the same runtime over a bounded HTTP
adapter.

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

## Run over HTTP

Start the second host artifact:

```fish
cargo run -p ch10-host --bin ch10-http-server
```

It listens on `127.0.0.1:3000`. From another terminal, invoke the component
through the HTTP adapter:

```fish
curl --fail-with-body \
 --data 'server component' \
 http://127.0.0.1:3000/uppercase
```

`--fail-with-body` makes curl exit non-zero for an HTTP error response while
still printing its body, so the command works as both a demonstration and a
smoke test.

Expected response body:

```text
SERVER COMPONENT
```

Press Ctrl-C to stop the server. To select another loopback port:

```fish
env CH10_BIND_ADDR 127.0.0.1:3001 \
 cargo run -p ch10-host --bin ch10-http-server
```

The optional first positional argument remains the component path:

```fish
cargo run -p ch10-host --bin ch10-http-server -- \
 target/wasm32-wasip2/debug/ch10_guest.wasm
```

Run the cached-loading, limits, output-validation, error-mapping, recovery, and
real TCP integration tests:

```fish
env CH10_COMPONENT target/wasm32-wasip2/debug/ch10_guest.wasm \
 cargo test -p ch10-host --test server_runtime -- --ignored
env CH10_COMPONENT target/wasm32-wasip2/debug/ch10_guest.wasm \
 cargo test -p ch10-host --test http_server -- --ignored
```

Expected result:

```text
test cached_component_handles_requests_with_fresh_limits ... ok
test component_is_invoked_over_http ... ok
```

## Architecture and operations

- Process startup constructs one engine and linker and compiles the component
  once. Production deployments can additionally enable Wasmtime's on-disk
  cache or distribute precompiled artifacts, but precompiled artifacts must be
  trusted, runtime-version-matched, and loaded through Wasmtime's documented
  unsafe contract. This example stays on the safe bytecode-loading API.
- Every invocation instantiates a fresh store. Stores are not shared across
  concurrent requests or reused after a trap.
- The HTTP adapter accepts only `POST`, rejects request bodies larger than
  64 KiB before component invocation, requires UTF-8, and admits at most 16
  component invocations concurrently. Saturation fails closed with a generic
  `503`.
- Wasmtime invocation is synchronous, so the Axum handler moves it to Tokio's
  blocking pool. The semaphore bounds admitted blocking work; disconnecting an
  HTTP client does not cancel an invocation which has already started.
- Fuel bounds guest execution deterministically; it is not a wall-clock
  deadline. A real server should combine fuel with epoch interruption,
  cancellation, queue limits, and process-level memory accounting.
- The store limiter caps each linear memory at 4 MiB and limits selected store
  resources. It does not cap host allocations or total process memory.
- A successful component result is accepted only when its status is in the
  final-response range `200..=599`, its UTF-8 body is at most 65,536 bytes,
  and it contains no control characters other than tab, line feed, or carriage
  return. The 64 KiB output ceiling is below the configured 4 MiB per-memory
  ceiling. Validation measures encoded bytes, not Unicode scalar values.
- Structured events use numeric request IDs and low-cardinality outcome and
  error-code fields. Request paths, bodies, guest errors, and trap details are
  deliberately excluded to avoid logging sensitive or attacker-controlled
  data.

## Safe error boundary

The component's explicit domain rejection becomes a generic HTTP-style `400`.
Traps, fuel exhaustion, instantiation failures, and other runtime faults become
a generic `503`. Invalid successful results also become that same generic
`503`, while bounded internal error codes distinguish invalid status, oversized
body, and unsafe control-character output. Internal guest output and runtime
messages do not enter the public response. A production host should retain
detailed errors only in access-controlled diagnostics, attach trace context,
and distinguish retryable failures according to its own policy.

The second binary is a working HTTP server, but it is still a teaching
artifact rather than a deployable edge service. It has no TLS, authentication,
distributed rate limiting, request deadline, load balancer integration, or
production observability pipeline.
