# Chapter 13: typed Component Model composition

Stable book link:
<https://github.com/joshua-mo-143/production-webassembly-rust/tree/main/examples/ch13-components>

Two independently built components share the versioned
`book:composition@1.0.0` WIT package. The catalog returns a `product` record
whose `availability` is a variant inside a typed `result`. A typed Wasmtime
host handles the catalog error or passes the product to the renderer. No
component exchanges JSON, raw pointers, allocator functions, or numeric status
codes.

Run all commands from the repository root.

## Build and run

```fish
cargo build --target wasm32-wasip2 \
 -p ch13-catalog -p ch13-renderer
cargo run -p ch13-host
```

Expected output:

```text
catalog + renderer: Production WebAssembly with Rust [RUST-WASM]: 7 in stock
typed not-found: true
boundary encoding: canonical ABI (generated)
```

Run automated interface and composition tests:

```fish
env CH13_CATALOG=target/wasm32-wasip2/debug/ch13_catalog.wasm \
 CH13_RENDERER=target/wasm32-wasip2/debug/ch13_renderer.wasm \
 cargo test -p ch13-host --test composition -- --ignored
```

Expected result:

```text
test generated_interfaces_compose_records_variants_and_results ... ok
test typed_errors_cross_the_catalog_boundary ... ok
```

Loading and instantiating through bindings generated from each WIT world is the
compatibility check: an artifact that exports a core module or an incompatible
world fails before business logic runs. The tests then exercise every
`availability` case and both catalog error cases across real component calls.

## What is being composed

`catalog-component` exports the `catalog` interface and
`renderer-component` exports the `renderer` interface. Both interfaces reuse
the same `domain` types. The host is the composition root: it invokes one
component, maps the generated Rust representation of the shared WIT record
between world-specific binding modules, and invokes the other.

That small mapping is host-language plumbing, not serialization. At the
component boundaries, the canonical ABI lowers and lifts the WIT values. A
deployment can later replace this runtime orchestration with static component
composition tooling while retaining the same interfaces, provided the final
composed world wires matching imports and exports.

## Migrating from a core-module boundary

A core-module design commonly exports `lookup(ptr, len) -> i32`, shares a
linear-memory convention, and encodes records and errors in JSON. Migrate one
boundary at a time:

1. Write a versioned WIT package that names the existing domain concepts.
2. Wrap the core implementation behind a component that owns canonical ABI
   lifting and lowering.
3. Generate host bindings and remove hand-written pointer, allocator, and JSON
   code.
4. Split interfaces into independently replaceable components only after the
   typed contract and error semantics are stable.

## Caveats

- This demonstrates dynamic composition in a typed host. It does not produce
  one statically composed `.wasm` artifact or demonstrate a component registry.
- The host conversion exists because each generated world has a distinct Rust
  module. WIT identity and structural compatibility govern the boundary; Rust
  type identity is local to generated bindings.
- Records, variants, and results are demonstrated because they fit the domain.
  WIT resources are intentionally omitted: neither component exposes a
  stateful handle with a lifecycle to manage.
- Each call uses a fresh store with fuel and memory limits. This favors clear
  isolation over pooling performance and is not a complete process-level
  resource policy.
- Package versioning communicates contract identity; it does not make breaking
  changes compatible. Breaking type changes need a new package major and an
  explicit migration or adapter.

See the stable
[Component Model repository](https://github.com/WebAssembly/component-model)
and the
[WIT specification](https://github.com/WebAssembly/component-model/blob/main/design/mvp/WIT.md).
