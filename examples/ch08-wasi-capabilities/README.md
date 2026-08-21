# Chapter 8: least-privilege WASI capabilities

Development-branch repository link:
<https://github.com/joshua-mo-143/production-webassembly-rust/tree/main/examples/ch08-wasi-capabilities>

The host creates two temporary directories but preopens only one as the guest's
read-only `/data` directory. The guest can read `/data/allowed.txt`. It cannot
read the exact absolute path of a file in the ungranted directory, and it
cannot write through the read-only preopen.

Run all commands from the repository root. No network or external service is
used.

## Build and run

```fish
cargo build --target wasm32-wasip2 -p ch08-guest
cargo run -p ch08-host
```

Expected output:

```text
allowed read: chapter 8
ambient host path: denied
write in read-only preopen: denied
```

Run the capability integration test:

```fish
env CH08_COMPONENT=target/wasm32-wasip2/debug/ch08_guest.wasm \
 cargo test -p ch08-host --test capabilities -- --ignored
```

Expected result:

```text
test only_explicit_read_capability_is_available ... ok
```

The test passes the guest the real absolute path of an existing host-side
secret fixture. Reading it still fails because that path is not reachable from
any preopen. The test also checks that the denied write created no host file.

## Threat model and guarantees

The guest starts without ambient filesystem access. `WasiCtxBuilder` opens one
specific host directory on the host's behalf and exposes that capability as
read-only `/data`. Wasmtime-WASI resolves guest filesystem operations through
preopened capabilities; knowing an absolute host path does not grant access.
This host does not inherit host environment variables, stdio, or networking,
and the default WASI networking policy remains denied.

The host still has ambient authority and must:

- Preopen the narrowest directory with the narrowest permissions. Preopening a
  home directory or `/` would intentionally grant a much larger capability.
- Control the contents and lifecycle of granted directories and avoid placing
  unrelated secrets inside them.
- Decide whether clocks, randomness, environment entries, stdio, or networking
  are needed. This example leaves default clocks and randomness available but
  grants no filesystem path except `/data`.
- Validate untrusted outputs and resource use. Capability security limits what
  a guest can address; it does not make guest data trustworthy or impose CPU
  and memory quotas.

The host-side act of opening the temporary directory uses native authority.
The security boundary is the narrower capability inserted into the guest's
WASI context, not an assertion that the embedding process itself lacks access.
