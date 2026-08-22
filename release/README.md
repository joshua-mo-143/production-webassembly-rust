# Release artifact groundwork

This directory documents reproducible local bundles and the manual artifact
workflow. Neither path creates a tag or a GitHub release.

## Build a local bundle

Use a clean checkout on Linux. `rust-toolchain.toml` pins Rust 1.97.1, the
`rustfmt` and `clippy` components, and the `wasm32-wasip2` target. The packaging
script also requires Git, Python 3.11 or newer, GNU tar, gzip, and `sha256sum`.

```fish
./scripts/package-release.sh book-v1.0.0
```

The script uses the commit timestamp as `SOURCE_DATE_EPOCH`, builds every guest
component in release mode with `Cargo.lock`, and writes
`dist/production-webassembly-rust-book-v1.0.0.tar.gz`. It refuses a dirty
working tree. Repeating the command for the same commit, version, Rust
toolchain, and Linux build environment is intended to produce identical bytes.

## Artifact layout

The archive contains one versioned top-level directory:

- `components/`: the 13 release-mode `wasm32-wasip2` components.
- `source.tar.gz`: the tracked source at the packaged commit, excluding the
  Chapter 14 test-key directory.
- `Cargo.lock`, `README.md`, `RELEASE.md`, and `licenses/`: lockfile,
  documentation, and dual-license texts.
- `sbom/cyclonedx-1.6.json`: a deterministic CycloneDX 1.6 SBOM scoped to the
  13 packaged guest crates and their non-development dependency closure for
  `wasm32-wasip2`. Each shipped `.wasm` file is a component with its SHA-256
  digest and a dependency edge to its Cargo package. Host-only crates and their
  dependencies are not represented as shipped artefact dependencies.
- `build-info.txt`: version, commit, epoch, compiler, target, and profile.
- `SHA256SUMS`: SHA-256 hashes for every payload file.

## Verify a downloaded bundle

```fish
set archive production-webassembly-rust-book-v1.0.0.tar.gz
set verify_dir (mktemp -d)
tar -xzf "$archive" -C "$verify_dir"
cd "$verify_dir/production-webassembly-rust-book-v1.0.0"
sha256sum -c SHA256SUMS
```

For bundles produced by `.github/workflows/package.yml`, download the separate
GitHub build-provenance attestation and verify the archive against the
repository identity:

```fish
gh attestation verify "$archive" \
 --repo joshua-mo-143/production-webassembly-rust
```

The workflow pins every action by full commit SHA, runs only by manual
dispatch, uploads the archive and provenance, and has only `contents: read`,
`id-token: write`, and `attestations: write`. It does not create tags or
releases.

## Linux scope and signing boundary

CI and packaging run on Ubuntu 24.04. Filesystem containment, symlink handling,
and path-policy tests therefore cover Linux behavior only. They do not
establish or claim Windows path or filesystem semantics.

The tracked keys under `case-study/keys/` are Chapter 14 test fixtures. The
packaging script excludes that directory from `source.tar.gz`, does not copy
the keys elsewhere, and performs no signing. The manual workflow uses GitHub
OIDC only to create build provenance; it never signs with the Chapter 14 keys.
