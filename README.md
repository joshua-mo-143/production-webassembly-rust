# Production WebAssembly with Rust — companion examples

Runnable examples for the book's Component Model chapters. Each example keeps
the WIT contract separate from the Rust guest and host so the boundary remains
visible.

## Prerequisites

- Current stable Rust (this repository was verified with Rust 1.97.1).
- The `wasm32-wasip2` standard library:

  ```fish
  rustup target add wasm32-wasip2
  ```

  On Arch Linux using the distribution Rust toolchain, install its matching
  target package instead:

  ```fish
  paru -S rust-wasm
  ```

- Optional: [`wasm-tools`](https://github.com/bytecodealliance/wasm-tools) for
  inspection:

  ```fish
  cargo install wasm-tools
  ```

No standalone Wasmtime CLI or `cargo-component` installation is required. The
guests use stable Rust's native WASI 0.2 target and the hosts embed Wasmtime.

## Quick start

Run from the repository root:

```fish
cargo build --target wasm32-wasip2 -p ch04-guest
cargo run -p ch04-host
```

Expected output:

```text
Hello, reader, from WebAssembly!
```

For the complete commands, tests, and expected output, use each chapter's
README.

## Chapter map

| Chapter | Status | Example |
| --- | --- | --- |
| 1–3 | Not available | — |
| 4 | Available | [First module](examples/ch04-first-module/) |
| 5 | Available | [Stable API](examples/ch05-stable-api/) |
| 6 | Available | [Data exchange](examples/ch06-data-exchange/) |
| 7 | Available | [Sandbox limits](examples/ch07-sandbox-limits/) |
| 8 | Available | [WASI capabilities](examples/ch08-wasi-capabilities/) |
| 9 | Not available | — |
| 10 | Available | [Server runtime](examples/ch10-server-runtime/) |
| 11 | Available | [Plugin system](examples/ch11-plugin-system/) |
| 12 onward | Not available | — |

## Repository checks

```fish
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --target wasm32-wasip2 \
 -p ch04-guest -p ch05-guest -p ch06-guest \
 -p ch07-guest -p ch08-guest -p ch10-guest \
 -p ch11-plugin-v1 -p ch11-plugin-v1-1 -- -D warnings
cargo build --target wasm32-wasip2 \
 -p ch04-guest -p ch05-guest -p ch07-guest -p ch08-guest \
 -p ch10-guest -p ch11-plugin-v1 -p ch11-plugin-v1-1
cargo build --release --target wasm32-wasip2 -p ch06-guest
cargo test --workspace
```

The lockfile intentionally records the exact dependency set used by CI while
the manifests state direct compatibility requirements.
