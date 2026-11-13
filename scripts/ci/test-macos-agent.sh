#!/bin/bash
# Native tests exercise Mach syscalls that a cross build cannot validate.
set -euo pipefail
[ "$(uname -s)" = Darwin ]
[ "$(uname -m)" = x86_64 ]
: "${RUNNER_TEMP:?Runner temporary directory is required}"
: "${SOURCE_COMMIT:?Source commit is required}"
[[ "$SOURCE_COMMIT" =~ ^[0-9a-f]{40}$ ]]
export CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
export CARGO_TARGET_DIR="$RUNNER_TEMP/nodelite-macos-target"
export MACOSX_DEPLOYMENT_TARGET=11.0
free_kib=$(df -k "$RUNNER_TEMP" | awk 'NR==2 {print $4}')
[ "$free_kib" -ge 6291456 ] || { echo 'At least 6 GiB free space is required' >&2; exit 1; }
mkdir -p dist/macos-native
{
  printf 'source_commit=%s\n' "$SOURCE_COMMIT"
  sw_vers
  uname -m
  xcrun --show-sdk-version
  clang --version
  rustc +1.93.1 --version
  cargo +1.93.1 --version
  node --version
  forgejo-runner --version
  df -h "$RUNNER_TEMP"
} | tee dist/macos-native/BUILD_INFO.txt
cargo +1.93.1 test -p nodelite-proto --no-default-features --locked
cargo +1.93.1 test -p nodelite-agent --locked
cargo +1.93.1 clippy -p nodelite-agent --all-targets --locked -- -D warnings
for target in x86_64-apple-darwin aarch64-apple-darwin; do
  cargo +1.93.1 build --locked --release -p nodelite-agent --target "$target"
  binary="$CARGO_TARGET_DIR/$target/release/nodelite-agent"
  description=$(file -b "$binary")
  case "$description" in
    Mach-O\ 64-bit*executable*) ;;
    *) printf 'Unexpected binary format: %s\n' "$description" >&2; exit 1 ;;
  esac
  case "$target" in
    x86_64-apple-darwin) expected_arch=x86_64 ;;
    aarch64-apple-darwin) expected_arch=arm64 ;;
  esac
  [ "$(xcrun lipo -archs "$binary")" = "$expected_arch" ]
  printf '%s: %s\n' "$target" "$description" | tee -a dist/macos-native/BUILD_INFO.txt
  xcrun vtool -show-build "$binary" | tee -a dist/macos-native/BUILD_INFO.txt
  install -m 0755 "$binary" "dist/macos-native/nodelite-agent-$target"
done
binary=dist/macos-native/nodelite-agent-x86_64-apple-darwin
"$binary" --help
# A disposable loopback configuration keeps this collector check offline.
config="$RUNNER_TEMP/macos-agent-sample.toml"
umask 077
cat > "$config" <<'TOML'
[agent]
node_id = "macos-ci"
node_label = "macOS CI"
server = "ws://127.0.0.1:9/ws"
token = "disposable-ci-token"
TOML
RUST_LOG=off "$binary" --config "$config" --sample-once > dist/macos-native/sample.json
node - dist/macos-native/sample.json <<'JS'
const fs = require('fs');
const sample = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
if (sample.identity.node_id !== 'macos-ci' || !sample.snapshot || typeof sample.snapshot !== 'object') {
  throw new Error('Native collector did not return the expected sample');
}
console.log('Native macOS collector returned a valid sample');
JS
(cd dist/macos-native && shasum -a 256 nodelite-agent-* > SHA256SUMS.txt)
