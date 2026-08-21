# Chapter 5: a stable, versioned interface

The shared contract is `wit/stable-api.wit`, whose package identity is
`book:stable-api@1.0.0`. The guest and host are separate crates and both
generate bindings from that contract. The host also checks the guest's reported
major version before using its result.

This separation is deliberate: hosts depend on the interface, not the guest's
Rust implementation. A breaking contract should receive a new WIT major
version and an intentional host migration; a runtime version string cannot make
structurally incompatible WIT types compatible.

Run all commands from the repository root.

## Build, run, and test

```fish
cargo build --target wasm32-wasip2 -p ch05-guest
cargo run -p ch05-host
```

Expected output:

```text
API 1.0.0: stable interfaces last
```

Run the native policy and guest logic tests:

```fish
cargo test -p ch05-guest -p ch05-host
```

Expected result: all tests pass; the artifact-dependent compatibility test is
listed as ignored.

Run that host/guest compatibility test explicitly:

```fish
env CH05_COMPONENT=target/wasm32-wasip2/debug/ch05_guest.wasm \
 cargo test -p ch05-host --test compatibility -- --ignored
```

Expected result:

```text
test v1_guest_satisfies_host_contract ... ok
```

The unit tests cover the v1 host policy and normalization behavior. The ignored
integration test is the important boundary check: it loads the independently
built component, lets Wasmtime type-check it against the host's WIT bindings,
then checks observable version and normalization behavior.
