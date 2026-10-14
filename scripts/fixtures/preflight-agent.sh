#!/bin/sh
set -eu
[ "$#" -eq 3 ]
[ "$1" = --config ]
[ "$3" = --check-credentials ]
[ "$2" = "$PREFLIGHT_EXPECTED_CONFIG" ]
[ -r "$2" ]
[ "${PREFLIGHT_TEST_REJECT:-0}" -eq 0 ]
