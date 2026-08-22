---
name: idiomatic-rust
description: >
  Write and review idiomatic Rust in this companion repository. Use when
  editing .rs files, adding host APIs, converting types, or reviewing
  builders, From/TryFrom, and numeric conversions. Also apply the personal
  rust-skills ruleset when available.
---

# Idiomatic Rust

Read `~/.cursor/skills/rust-skills/SKILL.md` first, then the conversion,
builder, and type-safety rules it links.

## This repository

- Prefer **builders** for multi-field policy and limit types (`RequestLimits`, `RuntimePolicy`).
- Implement **`From` / `TryFrom`**, not `fn into_x` / `fn to_y` conversion helpers.
- Implement **`From`**, not `Into`. Accept `impl Into<T>` and `impl AsRef<Path>` at load boundaries.
- Replace `as` casts with **`From`** (widening) or **`TryFrom`** (narrowing). `usize` → `f64` goes through a bounded integer first.
- Keep teaching examples small. Do not invent builders for two-field DTOs that are only constructed in tests.
- `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` after Rust edits.

## Do not

- Do not commit or push unless asked.
- Do not weaken fail-closed host checks to make a conversion prettier.
