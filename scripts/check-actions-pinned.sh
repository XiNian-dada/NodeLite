#!/bin/sh
set -eu

found=0

for file in .github/workflows/*.yml .github/workflows/*.yaml; do
  if [ ! -e "$file" ]; then
    continue
  fi

  awk '
    /uses:[[:space:]]*/ {
      value = $0
      sub(/^[[:space:]]*-[[:space:]]*uses:[[:space:]]*/, "", value)
      sub(/^[[:space:]]*uses:[[:space:]]*/, "", value)
      sub(/[[:space:]]*#.*/, "", value)
      sub(/[[:space:]]+$/, "", value)

      if (value ~ /^\.\//) {
        next
      }

      if (value !~ /@/) {
        print FILENAME ":" FNR ":" $0
        bad = 1
        next
      }

      ref = value
      sub(/^.*@/, "", ref)

      if (length(ref) == 40 && ref !~ /[^0-9a-f]/) {
        next
      }

      print FILENAME ":" FNR ":" $0
      bad = 1
    }
    END { exit bad ? 1 : 0 }
  ' "$file" || found=1
done

if [ "$found" -ne 0 ]; then
  printf '%s\n' "GitHub Actions uses must be pinned to full commit SHAs" >&2
  exit 1
fi
