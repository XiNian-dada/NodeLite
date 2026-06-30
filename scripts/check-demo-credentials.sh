#!/bin/sh
set -eu

set --
for path in README.md README.en.md config docs ops .github; do
  if [ -e "$path" ]; then
    set -- "$@" "$path"
  fi
done

found=0

check_pattern() {
  pattern="$1"
  shift
  matches="$(grep -RInF -- "$pattern" "$@" 2>/dev/null || true)"
  if [ -n "$matches" ]; then
    found=1
    printf '%s\n' "$matches"
  fi
}

check_pattern "change-me" "$@"
check_pattern "viewer:secret" "$@"
check_pattern "Str0ng#Passphrase!2026" "$@"
check_pattern "JBSWY3DPEHPK3PXP" "$@"

if [ "$found" -ne 0 ]; then
  printf '%s\n' "fixed demo credentials found in docs, config, ops, or workflows" >&2
  exit 1
fi
