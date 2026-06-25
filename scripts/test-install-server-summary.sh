#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
INSTALLER="$SCRIPT_DIR/install-server.sh"
HAS_TTY_MODE="non-tty"

extract_summary_function() {
  awk '
    /^print_readonly_credentials_summary\(\) \{/ { printing = 1 }
    printing { print }
    printing && /^\}/ { exit }
  ' "$INSTALLER"
}

run_summary() {
  HAS_TTY_MODE="$1"

  READONLY_USERNAME="viewer"
  READONLY_PASSWORD="secret-from-test"
  CONFIG_PATH="/opt/nodelite/config/server.toml"
  export READONLY_USERNAME READONLY_PASSWORD CONFIG_PATH HAS_TTY_MODE

  eval "$(extract_summary_function)"
  print_readonly_credentials_summary
}

has_tty() {
  [ "$HAS_TTY_MODE" = "tty" ]
}

tty_println() {
  printf '%s\n' "$*"
}

non_tty_output="$(run_summary non-tty)"
case "$non_tty_output" in
  *secret-from-test*)
    printf '%s\n' "non-tty summary leaked readonly password" >&2
    exit 1
    ;;
esac
printf '%s\n' "$non_tty_output" | grep -F "Readonly password: written to /opt/nodelite/config/server.toml" >/dev/null

tty_output="$(run_summary tty)"
printf '%s\n' "$tty_output" | grep -F "Readonly password: secret-from-test" >/dev/null
