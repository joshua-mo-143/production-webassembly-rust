#!/usr/bin/env bash
set -euo pipefail

export LC_ALL=C
export TZ=UTC
umask 022

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "usage: $0 VERSION [OUTPUT_DIRECTORY]" >&2
  exit 2
fi

version=$1
if [[ ! "$version" =~ ^[0-9A-Za-z][0-9A-Za-z._-]*$ ]]; then
  echo "version must contain only letters, digits, dots, underscores, and hyphens" >&2
  exit 2
fi

repository=$(git -C "$(dirname "${BASH_SOURCE[0]}")/.." rev-parse --show-toplevel)
output_directory=${2:-"$repository/dist"}
package_name="production-webassembly-rust-$version"
commit=$(git -C "$repository" rev-parse HEAD)
source_date_epoch=${SOURCE_DATE_EPOCH:-$(git -C "$repository" show -s --format=%ct HEAD)}
timestamp=$(date --utc --date="@$source_date_epoch" "+%Y-%m-%dT%H:%M:%SZ")

if [[ -n "$(git -C "$repository" status --porcelain=v1 --untracked-files=all)" ]]; then
  echo "refusing to package a dirty working tree" >&2
  exit 1
fi

if [[ "$(rustc --version)" != rustc\ 1.97.1\ * ]]; then
  echo "rustc 1.97.1 is required by rust-toolchain.toml" >&2
  exit 1
fi

mkdir -p "$output_directory"
staging=$(mktemp -d "$output_directory/.package.XXXXXX")
trap 'rm -rf "$staging"' EXIT
bundle="$staging/$package_name"
mkdir -p "$bundle/components" "$bundle/licenses" "$bundle/sbom"

export CARGO_INCREMENTAL=0
export SOURCE_DATE_EPOCH="$source_date_epoch"
cargo build \
  --manifest-path "$repository/Cargo.toml" \
  --release \
  --locked \
  --target wasm32-wasip2 \
  -p ch04-guest \
  -p ch05-guest \
  -p ch06-guest \
  -p ch07-guest \
  -p ch08-guest \
  -p ch10-guest \
  -p ch11-plugin-v1 \
  -p ch11-plugin-v1-1 \
  -p ch12-guest \
  -p ch13-catalog \
  -p ch13-renderer \
  -p ch14-normalizer \
  -p ch14-workspace-reader

artifacts=(
  ch04_guest.wasm
  ch05_guest.wasm
  ch06_guest.wasm
  ch07_guest.wasm
  ch08_guest.wasm
  ch10_guest.wasm
  ch11_plugin_v1.wasm
  ch11_plugin_v1_1.wasm
  ch12_guest.wasm
  ch13_catalog.wasm
  ch13_renderer.wasm
  ch14_normalizer.wasm
  ch14_workspace_reader.wasm
)
for artifact in "${artifacts[@]}"; do
  install -m 0644 \
    "$repository/target/wasm32-wasip2/release/$artifact" \
    "$bundle/components/$artifact"
done

install -m 0644 "$repository/Cargo.lock" "$bundle/Cargo.lock"
install -m 0644 "$repository/README.md" "$bundle/README.md"
install -m 0644 "$repository/release/README.md" "$bundle/RELEASE.md"
install -m 0644 "$repository/LICENSE-APACHE" "$bundle/licenses/LICENSE-APACHE"
install -m 0644 "$repository/LICENSE-MIT" "$bundle/licenses/LICENSE-MIT"

git -C "$repository" archive \
  --format=tar \
  --prefix="$package_name-source/" \
  HEAD \
  -- . ':(exclude)case-study/keys/**' |
  gzip -n >"$bundle/source.tar.gz"

while IFS= read -r archived_path; do
  if [[ "$archived_path" == *"/case-study/keys/"* ]]; then
    echo "Chapter 14 test key entered the source archive: $archived_path" >&2
    exit 1
  fi
done < <(gzip -dc "$bundle/source.tar.gz" | tar -tf -)

cat >"$bundle/build-info.txt" <<EOF
version=$version
git_commit=$commit
source_date_epoch=$source_date_epoch
rustc=$(rustc --version)
target=wasm32-wasip2
profile=release
EOF

python3 "$repository/scripts/generate-sbom.py" \
  --output "$bundle/sbom/cyclonedx-1.6.json" \
  --components-directory "$bundle/components" \
  --version "$version" \
  --commit "$commit" \
  --timestamp "$timestamp"

checksum_paths=(
  Cargo.lock
  README.md
  RELEASE.md
  build-info.txt
  components/ch04_guest.wasm
  components/ch05_guest.wasm
  components/ch06_guest.wasm
  components/ch07_guest.wasm
  components/ch08_guest.wasm
  components/ch10_guest.wasm
  components/ch11_plugin_v1.wasm
  components/ch11_plugin_v1_1.wasm
  components/ch12_guest.wasm
  components/ch13_catalog.wasm
  components/ch13_renderer.wasm
  components/ch14_normalizer.wasm
  components/ch14_workspace_reader.wasm
  licenses/LICENSE-APACHE
  licenses/LICENSE-MIT
  sbom/cyclonedx-1.6.json
  source.tar.gz
)
(cd "$bundle" && sha256sum "${checksum_paths[@]}") >"$bundle/SHA256SUMS"

archive="$output_directory/$package_name.tar.gz"
temporary_archive="$staging/$package_name.tar.gz"
tar \
  --sort=name \
  --mtime="@$source_date_epoch" \
  --owner=0 \
  --group=0 \
  --numeric-owner \
  --pax-option=delete=atime,delete=ctime \
  -C "$staging" \
  -cf - \
  "$package_name" |
  gzip -n >"$temporary_archive"
mv -f "$temporary_archive" "$archive"

printf '%s\n' "$archive"
