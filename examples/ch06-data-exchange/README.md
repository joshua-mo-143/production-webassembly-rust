# Chapter 6: host–guest data exchange

This component exposes the same calculation through two boundary designs:

1. `list<sample>` uses typed WIT records. It is self-describing, generated
   bindings enforce the schema, and it is the default choice for application
   APIs.
2. `list<u8>` uses fixed-width records (`u64` timestamp then `f64` value, both
   little-endian). It is useful when bytes already exist in that representation
   or when one explicitly versioned wire format must cross several systems. The
   guest validates the byte length before decoding.

Packed bytes trade generated field-level type safety for explicit encoding
rules. They are not inherently faster.

Run all commands from the repository root.

## Build and measure

```fish
cargo build --release --target wasm32-wasip2 -p ch06-guest
cargo run --release -p ch06-host
```

Expected output shape (timings vary by machine and run):

```text
samples=10000 iterations=100
typed-records: <measured> ns/call
packed-bytes:  <measured> ns/call
Both paths returned the same sum.
```

For a short repeatable smoke run, pass the component path, sample count, and
iteration count:

```fish
cargo run --release -p ch06-host -- \
 target/wasm32-wasip2/release/ch06_guest.wasm 100 5
```

Expected first and last lines:

```text
samples=100 iterations=5
Both paths returned the same sum.
```

Run the packed-format decoder test with:

```fish
cargo test -p ch06-guest
```

Expected result: `sums_values_from_fixed_width_records` passes.

## What the harness measures

The host creates both representations before timing, performs one untimed call,
then measures repeated synchronous Wasmtime calls with `Instant`. Therefore it
includes canonical ABI transfer and guest work, including packed decoding, but
excludes component compilation, instantiation, and host-side encoding.

Use release builds, repeat runs, vary payload sizes, and record the machine and
runtime versions for manuscript measurements. This is a transparent comparison
harness, not a statistically rigorous benchmark suite, and its results do not
support universal performance conclusions.
