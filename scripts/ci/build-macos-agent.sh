#!/bin/bash
# Build both release artifacts on Linux using one pinned cross toolchain.
set -euo pipefail
export MACOSX_DEPLOYMENT_TARGET=11.0
export CARGO_ZIGBUILD_PYTHON_PATH=/opt/macos-cross/bin/python
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
for target in x86_64-apple-darwin aarch64-apple-darwin; do
  cargo zigbuild --locked --release -p nodelite-agent --target "$target"
  binary="target/$target/release/nodelite-agent"
  description=$(file -b "$binary")
  case "$target:$description" in
    x86_64-apple-darwin:Mach-O\ 64-bit\ x86_64\ executable*) ;;
    aarch64-apple-darwin:Mach-O\ 64-bit\ arm64\ executable*) ;;
    *) printf 'Unexpected target binary: %s: %s\n' "$target" "$description" >&2; exit 1 ;;
  esac
  printf '%s: %s\n' "$target" "$description"
  mkdir -p dist/macos
  install -m 0755 "$binary" "dist/macos/nodelite-agent-$target"
  (cd dist/macos && sha256sum "nodelite-agent-$target" >"SHA256SUMS-$target.txt")
done
