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

Confirm that Cargo can see the target before building:

```fish
set wasm_target_libdir (rustc --print target-libdir --target wasm32-wasip2)
test -d "$wasm_target_libdir"
and echo "wasm32-wasip2 is installed"
or echo "wasm32-wasip2 is missing"
```

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

## Examples index

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
| 12 | Available | [Agent tool boundary](examples/ch12-agent-tools/) |
| 13 | Available | [Component composition](examples/ch13-components/) |
| 14 | Available | [Secure agent runtime case study](case-study/) |
| 15 onward | Not available | — |

Every linked example README contains exact build and run commands, expected
output, and any example-specific test command. All commands are run from this
repository's root and require only the prerequisites above.

## Arch Linux troubleshooting (without rustup)

Arch's `rust` and `rust-wasm` packages are a matched toolchain. If `rustup` is
not installed, that is expected: do not mix a rustup-managed target into the
system toolchain.

```fish
paru -Syu rust rust-wasm
rustc --version
cargo --version
set wasm_target_libdir (rustc --print target-libdir --target wasm32-wasip2)
test -d "$wasm_target_libdir"
```

If the final command fails, make sure both packages came from the same current
repository snapshot, then reinstall them together:

```fish
paru -S rust rust-wasm
```

If Cargo unexpectedly invokes another toolchain, inspect the resolved binaries
and remove stale shell overrides before retrying:

```fish
type -a rustc cargo
set -q RUSTUP_TOOLCHAIN; and echo $RUSTUP_TOOLCHAIN
```

## Repository checks

```fish
cargo fmt --all -- --check
cargo build --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --target wasm32-wasip2 \
 -p ch04-guest -p ch05-guest -p ch06-guest \
 -p ch07-guest -p ch08-guest -p ch10-guest \
 -p ch11-plugin-v1 -p ch11-plugin-v1-1 -p ch12-guest \
 -p ch13-catalog -p ch13-renderer \
 -p ch14-normalizer -p ch14-workspace-reader -- \
 -D warnings -A clippy::same-length-and-capacity
cargo build --target wasm32-wasip2 \
 -p ch04-guest -p ch05-guest -p ch07-guest -p ch08-guest \
 -p ch10-guest -p ch11-plugin-v1 -p ch11-plugin-v1-1 \
 -p ch12-guest -p ch13-catalog -p ch13-renderer \
 -p ch14-normalizer -p ch14-workspace-reader
cargo build --release --target wasm32-wasip2 -p ch06-guest
cargo run -p ch14-host -- provision
cargo test -p ch14-host -- --ignored
cargo test --workspace
```

The lockfile intentionally records the exact dependency set used by CI while
the manifests state direct compatibility requirements.

CI additionally runs every artifact-backed integration test and each
deterministic command documented in the example READMEs. The workflow at
`.github/workflows/ci.yml` is the authoritative complete command list.

## Edition and release policy

All crates inherit Rust edition 2024 and a minimum supported Rust version of
1.97 from the workspace manifest. A change to either value is repository-wide,
must pass the full CI matrix, and is called out in release notes.

The `main` branch is the next companion revision. Published book links should
use an immutable GitHub release tag such as `book-v1.0.0`, not `main`. For each
book edition or corrected example release:

1. Run the complete CI workflow from a clean checkout.
2. Review dependency and license changes in `Cargo.lock`.
3. Create a signed `book-vMAJOR.MINOR.PATCH` tag and GitHub release.
4. Record the supported book edition, Rust version, and notable corrections in
   that release.
5. Use tag-pinned `/tree/book-vMAJOR.MINOR.PATCH/...` URLs in published
   material.

## License

This companion code is available under either the
[Apache License 2.0](LICENSE-APACHE) or the [MIT license](LICENSE-MIT), at your
option. This matches the `MIT OR Apache-2.0` SPDX expression inherited by every
workspace crate.
