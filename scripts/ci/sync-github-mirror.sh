#!/usr/bin/env bash
set -euo pipefail

: "${FORGEJO_TOKEN:?FORGEJO_TOKEN is required}"
: "${SOURCE_SERVER:?SOURCE_SERVER is required}"
: "${SOURCE_REPOSITORY:?SOURCE_REPOSITORY is required}"
: "${SOURCE_REF:?SOURCE_REF is required}"
: "${SOURCE_COMMIT:?SOURCE_COMMIT is required}"

GH_TOKEN="${GH_SYNC_TOKEN:-${GH_RELEASE_TOKEN:-}}"
if [ -z "$GH_TOKEN" ]; then
  echo "GH_SYNC_TOKEN (or GH_RELEASE_TOKEN with Contents: write for NodeLite) is required" >&2
  exit 1
fi

TARGET_REPOSITORY="${TARGET_REPOSITORY:-$SOURCE_REPOSITORY}"
TARGET_URL="${TARGET_REMOTE_URL:-https://github.com/${TARGET_REPOSITORY}.git}"
SOURCE_URL="${SOURCE_REMOTE_URL:-${SOURCE_SERVER%/}/${SOURCE_REPOSITORY}.git}"

WORK_DIR="$(mktemp -d "${RUNNER_TEMP:-/tmp}/nodelite-sync-XXXXXX")"
cleanup() {
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT

make_askpass() {
  local script_path="$1"
  local env_var="$2"
  cat >"$script_path" <<EOF
#!/bin/sh
case "\$1" in
  *Username*) printf '%s\n' 'x-access-token' ;;
  *Password*) eval "printf '%s\\n' \"\\\$$env_var\"" ;;
  *) printf '\n' ;;
esac
EOF
  chmod 700 "$script_path"
}

SOURCE_ASKPASS="$WORK_DIR/askpass-source.sh"
TARGET_ASKPASS="$WORK_DIR/askpass-target.sh"
make_askpass "$SOURCE_ASKPASS" "FORGEJO_TOKEN"
make_askpass "$TARGET_ASKPASS" "GH_TOKEN"

export GH_TOKEN
export GIT_TERMINAL_PROMPT=0

git init --bare "$WORK_DIR/repo.git" >/dev/null

SOURCE_GIT_ARGS=()
if [ -n "${ARCHIVE_PROXY:-}" ]; then
  SOURCE_GIT_ARGS=(-c "http.proxy=${ARCHIVE_PROXY}")
fi

case "$SOURCE_REF" in
  refs/heads/main)
    echo "Fetching refs/heads/main from Forgejo..."
    GIT_ASKPASS="$SOURCE_ASKPASS" git ${SOURCE_GIT_ARGS[@]+"${SOURCE_GIT_ARGS[@]}"} -C "$WORK_DIR/repo.git" fetch --no-tags "$SOURCE_URL" "refs/heads/main:refs/heads/main"
    FETCHED_SHA="$(git -C "$WORK_DIR/repo.git" rev-parse refs/heads/main)"
    if [ "$FETCHED_SHA" != "$SOURCE_COMMIT" ]; then
      echo "Fetched main ($FETCHED_SHA) differs from workflow commit ($SOURCE_COMMIT); pushing exact $SOURCE_COMMIT instead"
      GIT_ASKPASS="$SOURCE_ASKPASS" git ${SOURCE_GIT_ARGS[@]+"${SOURCE_GIT_ARGS[@]}"} -C "$WORK_DIR/repo.git" fetch --no-tags "$SOURCE_URL" "$SOURCE_COMMIT"
      PUSH_SPEC="$SOURCE_COMMIT:refs/heads/main"
    else
      PUSH_SPEC="refs/heads/main:refs/heads/main"
    fi
    echo "Pushing main (non-pruning, fast-forward only) to target remote..."
    GIT_ASKPASS="$TARGET_ASKPASS" git -C "$WORK_DIR/repo.git" push "$TARGET_URL" "$PUSH_SPEC"
    ;;
  refs/tags/*)
    echo "Fetching $SOURCE_REF from Forgejo..."
    GIT_ASKPASS="$SOURCE_ASKPASS" git ${SOURCE_GIT_ARGS[@]+"${SOURCE_GIT_ARGS[@]}"} -C "$WORK_DIR/repo.git" fetch --no-tags "$SOURCE_URL" "$SOURCE_REF:$SOURCE_REF"
    echo "Pushing $SOURCE_REF (non-pruning) to target remote..."
    GIT_ASKPASS="$TARGET_ASKPASS" git -C "$WORK_DIR/repo.git" push "$TARGET_URL" "$SOURCE_REF:$SOURCE_REF"
    ;;
  *)
    echo "Skipping unsupported ref: $SOURCE_REF (only refs/heads/main and refs/tags/* are synced)"
    exit 0
    ;;
esac

echo "Non-pruning sync complete for $SOURCE_REF"
