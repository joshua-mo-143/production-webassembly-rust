# Chapter 4: your first Rust component

`wit/world.wit` is the contract. `guest` implements it and stable Rust emits a
WebAssembly component directly. `host` uses Wasmtime's generated typed bindings
to load that artifact and invoke `greet`.

Run all commands from the repository root.

## Build and run

```fish
cargo build --target wasm32-wasip2 -p ch04-guest
cargo run -p ch04-host
```

Expected program output:

```text
Hello, reader, from WebAssembly!
```

The host accepts an explicit artifact path if you are not using the workspace
target directory:

```fish
cargo run -p ch04-host -- target/wasm32-wasip2/debug/ch04_guest.wasm
```

It prints the same greeting.

## Inspect and debug

These commands require the optional `wasm-tools` prerequisite from the
top-level README:

```fish
wasm-tools validate target/wasm32-wasip2/debug/ch04_guest.wasm
wasm-tools component wit target/wasm32-wasip2/debug/ch04_guest.wasm
wasm-tools print target/wasm32-wasip2/debug/ch04_guest.wasm >ch04.wat
```

`validate` succeeds without output. `component wit` prints the component's
resolved `root:component` world, its WASI 0.2 imports, and this export:

```wit
export greet: func(name: string) -> string;
```

`print` creates `ch04.wat`; search it for `greet` to locate the lowered
component export. To include a host backtrace when experimenting with a broken
path or component:

```fish
env RUST_BACKTRACE=1 cargo run -p ch04-host -- does-not-exist.wasm
```

Expected result: a non-zero exit and an error reporting that the component file
could not be read, followed by a Rust backtrace.
