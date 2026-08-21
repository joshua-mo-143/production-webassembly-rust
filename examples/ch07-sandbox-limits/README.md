# Chapter 7: sandbox controls and limits

Development-branch repository link:
<https://github.com/joshua-mo-143/production-webassembly-rust/tree/main/examples/ch07-sandbox-limits>

The guest supplies CPU-bound work, a large allocation, a deliberate panic, and
a small healthy function. The embeddable Wasmtime host configures fuel,
per-memory and resource limits, and a fresh store for each untrusted operation.
The integration test treats any unexpectedly successful constrained operation
as a test failure.

Run all commands from the repository root.

## Build and run

```fish
cargo build --target wasm32-wasip2 -p ch07-guest
cargo run -p ch07-host
```

Expected output:

```text
fuel: trapped after budget exhaustion
memory: trapped at configured growth limit
guest failure: trapped; host process survived
fresh store after traps: 42
```

Run the fail-closed integration test:

```fish
env CH07_COMPONENT=target/wasm32-wasip2/debug/ch07_guest.wasm \
 cargo test -p ch07-host --test containment -- --ignored
```

Expected result:

```text
test configured_limits_fail_closed_and_host_recovers ... ok
```

The host gives each store enough fuel to instantiate, then resets the
CPU-bound call to 10,000 fuel units. Its `StoreLimits` caps each linear memory
at 4 MiB, bounds instance/table/memory counts, and converts failed growth into
a trap. The example requests 16 MiB to exercise that boundary.

## Threat model and guarantees

What Wasmtime enforces when the host enables and configures it:

- Fuel is deterministic instruction-style metering. Exhaustion interrupts
  WebAssembly execution with a trap.
- The resource limiter refuses linear-memory growth beyond its configured
  per-memory ceiling and limits selected store resources.
- A guest panic or `unreachable` traps the current WebAssembly call rather than
  unwinding through or terminating this host process.

What remains the host's responsibility:

- Choose and replenish fuel budgets. Fuel is not a wall-clock deadline; use
  epoch interruption when elapsed-time cancellation is the requirement.
- Account for all memories, stores, host allocations, handles, I/O, and
  concurrent requests. A per-memory limit is not a process-wide memory budget.
- Treat a trapped store as suspect. This example discards it and proves that a
  fresh store still works.
- Keep host callbacks, the runtime, and the operating system trusted and
  patched. WebAssembly isolation does not sandbox arbitrary native host code or
  eliminate runtime vulnerabilities.

These checks demonstrate configured containment, not a complete production
admission-control or process-isolation policy.
