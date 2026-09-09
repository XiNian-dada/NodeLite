#!/bin/sh
# Keep executable replacement and database recovery in one stopped-service transaction.
# Sourced only after its version-bound release checksum is verified.
# shellcheck disable=SC2034,SC2154

UPGRADE_STARTED=0
UPGRADE_MUTATED=0
UPGRADE_COMMITTED=0
UPGRADE_WAS_ACTIVE=0
UPGRADE_BACKUP_READY=0
UPGRADE_BACKUP_DIGEST=""
UPGRADE_BIN_TARGET=""
UPGRADE_UNIT_TARGET=""
UPGRADE_BACKUP_DIR=""
UPGRADE_READY_URL=""
UPGRADE_OLD_READY_URL=""
TMP_NEXT_CONFIG=""
TMP_MANIFEST=""
TMP_OLD_MANIFEST=""
TMP_REPLACE=""
TMP_READY_HEADERS=""
READINESS_TIMEOUT="${NODELITE_SERVER_READY_TIMEOUT_SECS:-60}"

validate_upgrade_settings() {
  case "$READINESS_TIMEOUT" in
    ''|*[!0-9]*) fail "readiness timeout must be an integer between 2 and 300 seconds" ;;
  esac
  if [ "$READINESS_TIMEOUT" -lt 2 ] || [ "$READINESS_TIMEOUT" -gt 300 ]; then
    fail "readiness timeout must be between 2 and 300 seconds"
  fi
  EXPECTED_VERSION="${VERSION#v}"
  EXPECTED_VERSION="${EXPECTED_VERSION#V}"
  case "$EXPECTED_VERSION" in
    ''|*[!0-9A-Za-z.+-]*) fail "set NODELITE_SERVER_VERSION to a concrete release tag" ;;
  esac
  exec 9>>"${BIN_PATH}.install.lock"
  chmod 0600 "${BIN_PATH}.install.lock"
  flock -n 9 || fail "another NodeLite server installation is in progress"
}

verify_candidate_version() {
  upgrade_candidate_version="$(timeout --kill-after=5 20 "$TMP_BIN" --version)" \
    || fail "candidate server cannot report its version"
  [ "$upgrade_candidate_version" = "nodelite-server $EXPECTED_VERSION" ] \
    || fail "candidate server version does not match $EXPECTED_VERSION"
}

read_upgrade_manifest() {
  (cd "$1" && timeout --kill-after=5 20 "$TMP_BIN" --config "$2" upgrade-manifest) >"$3" \
    || fail "candidate cannot describe compatible upgrade files"
  [ "$(wc -c <"$3")" -le 1048576 ] || fail "upgrade manifest exceeds size limit"
  awk '
    NR == 1 { if ($0 != "nodelite-upgrade-manifest-v1") exit 1; next }
    /^version=/ { versions++; next }
    /^ready_url=http:\/\// { urls++; next }
    /^path=\// { paths++; next }
    { invalid = 1 }
    END { if (invalid || versions != 1 || urls != 1 || paths < 1) exit 1 }
  ' "$3" || fail "invalid upgrade manifest"
  [ "$(sed -n 's/^version=//p' "$3")" = "$EXPECTED_VERSION" ] \
    || fail "upgrade manifest version mismatch"
}

stop_upgrade_service() {
  timeout --kill-after=5 25 systemctl stop "$SERVICE_NAME.service" || return 1
  upgrade_stopped_state="$(timeout --kill-after=2 5 systemctl show \
    --property=ActiveState --value "$SERVICE_NAME.service")" || return 1
  case "$upgrade_stopped_state" in inactive|failed) ;; *) return 1 ;; esac
  upgrade_stopped_pid="$(timeout --kill-after=2 5 systemctl show \
    --property=MainPID --value "$SERVICE_NAME.service")" || return 1
  [ "$upgrade_stopped_pid" = 0 ]
}

prepare_upgrade_config() {
  TMP_NEXT_CONFIG="$(mktemp "${TMPDIR:-/tmp}/nodelite-next-config.XXXXXX")"
  if [ "$MODE" = upgrade ]; then
    cp "$CURRENT_CONFIG_PATH" "$TMP_NEXT_CONFIG"
  else
    render_server_config >"$TMP_NEXT_CONFIG"
  fi
  complete_server_config_defaults "$TMP_NEXT_CONFIG"
  read_upgrade_manifest "$INSTALL_ROOT" "$TMP_NEXT_CONFIG" "$TMP_MANIFEST"
  UPGRADE_READY_URL="$(sed -n 's/^ready_url=//p' "$TMP_MANIFEST")"
}

record_backup_digest() {
  upgrade_digest="$(calculate_sha256 "$UPGRADE_BACKUP_DIR/$1")" || return 1
  [ "${#upgrade_digest}" -eq 64 ] || return 1
  printf '%s  %s\n' "$upgrade_digest" "$1" >>"$UPGRADE_BACKUP_DIR/checksums"
}

backup_upgrade_files() {
  upgrade_backup_parent="${CURRENT_INSTALL_ROOT:-$INSTALL_ROOT}/.upgrade-backups"
  mkdir -p "$upgrade_backup_parent"
  chmod 0700 "$upgrade_backup_parent"
  UPGRADE_BACKUP_DIR="$(mktemp -d "$upgrade_backup_parent/upgrade.XXXXXX")"
  UPGRADE_BIN_TARGET="$BIN_PATH"
  UPGRADE_UNIT_TARGET="$UNIT_PATH"
  if [ -L "$BIN_PATH" ]; then
    UPGRADE_BIN_TARGET="$(realpath "$BIN_PATH")" || fail "cannot resolve the previous executable"
  fi
  if [ -L "$UNIT_PATH" ]; then
    UPGRADE_UNIT_TARGET="$(realpath "$UNIT_PATH")" || fail "cannot resolve the previous service unit"
  fi
  {
    printf '%s\n' "$BIN_PATH" "$UNIT_PATH" "$UPGRADE_BIN_TARGET" "$UPGRADE_UNIT_TARGET" "$CONFIG_PATH"
    sed -n 's/^path=//p' "$TMP_OLD_MANIFEST" "$TMP_MANIFEST"
  } | sort -u >"$UPGRADE_BACKUP_DIR/paths"
  : >"$UPGRADE_BACKUP_DIR/checksums"
  upgrade_backup_index=0
  while IFS= read -r upgrade_backup_path; do
    # The staged config is temporary; its final destination is inventoried above.
    [ "$upgrade_backup_path" != "$TMP_NEXT_CONFIG" ] || continue
    upgrade_backup_index=$((upgrade_backup_index + 1))
    printf '%s\n' "$upgrade_backup_path" >"$UPGRADE_BACKUP_DIR/$upgrade_backup_index.path"
    record_backup_digest "$upgrade_backup_index.path" || fail "failed to checksum backup inventory"
    if [ -e "$upgrade_backup_path" ] || [ -L "$upgrade_backup_path" ]; then
      [ -f "$upgrade_backup_path" ] || [ -L "$upgrade_backup_path" ] \
        || fail "upgrade inventory contains a non-file path: $upgrade_backup_path"
      tar -cpPf "$UPGRADE_BACKUP_DIR/$upgrade_backup_index.tar" -- "$upgrade_backup_path" \
        || fail "failed to back up $upgrade_backup_path"
      chmod 0600 "$UPGRADE_BACKUP_DIR/$upgrade_backup_index.tar"
      record_backup_digest "$upgrade_backup_index.tar" || fail "failed to checksum backup"
    else
      : >"$UPGRADE_BACKUP_DIR/$upgrade_backup_index.absent"
      record_backup_digest "$upgrade_backup_index.absent" || fail "failed to checksum absent-file record"
    fi
  done <"$UPGRADE_BACKUP_DIR/paths"
  printf '%s\n' "$upgrade_backup_index" >"$UPGRADE_BACKUP_DIR/count"
  record_backup_digest count || fail "failed to checksum backup count"
  UPGRADE_BACKUP_DIGEST="$(calculate_sha256 "$UPGRADE_BACKUP_DIR/checksums")"
  [ "${#UPGRADE_BACKUP_DIGEST}" -eq 64 ] || fail "failed to checksum backup manifest"
  printf '%s  checksums\n' "$UPGRADE_BACKUP_DIGEST" >"$UPGRADE_BACKUP_DIR/checksums.sha256"
  printf '%s\n' 'Backup complete; restore data before starting the previous executable.' \
    >"$UPGRADE_BACKUP_DIR/status"
  UPGRADE_BACKUP_READY=1
  tty_println "Compatible rollback backup: $UPGRADE_BACKUP_DIR"
}

begin_upgrade() {
  TMP_MANIFEST="$(mktemp "${TMPDIR:-/tmp}/nodelite-upgrade-manifest.XXXXXX")"
  TMP_OLD_MANIFEST="$(mktemp "${TMPDIR:-/tmp}/nodelite-old-manifest.XXXXXX")"
  if [ "$existing_install" -eq 1 ]; then
    if [ -z "$CURRENT_INSTALL_ROOT" ] || [ ! -f "$CURRENT_CONFIG_PATH" ]; then
      fail "existing files require a readable server config and WorkingDirectory before replacement"
    fi
    read_upgrade_manifest "$CURRENT_INSTALL_ROOT" "$CURRENT_CONFIG_PATH" "$TMP_OLD_MANIFEST"
    UPGRADE_OLD_READY_URL="$(sed -n 's/^ready_url=//p' "$TMP_OLD_MANIFEST")"
    upgrade_previous_state="$(timeout --kill-after=2 5 systemctl show \
      --property=ActiveState --value "$SERVICE_NAME.service")" \
      || fail "could not determine the previous service state"
    case "$upgrade_previous_state" in
      active|activating|reloading) UPGRADE_WAS_ACTIVE=1 ;;
      inactive|failed|deactivating) UPGRADE_WAS_ACTIVE=0 ;;
      *) fail "unexpected previous service state: $upgrade_previous_state" ;;
    esac
    UPGRADE_STARTED=1
    stop_upgrade_service || fail "could not stop the existing service for a consistent backup"
    # Settings may have changed while downloads were in progress.
    read_upgrade_manifest "$CURRENT_INSTALL_ROOT" "$CURRENT_CONFIG_PATH" "$TMP_OLD_MANIFEST"
    UPGRADE_OLD_READY_URL="$(sed -n 's/^ready_url=//p' "$TMP_OLD_MANIFEST")"
  else
    UPGRADE_STARTED=1
  fi
  mkdir -p "$INSTALL_ROOT"
  prepare_upgrade_config
  backup_upgrade_files
}

replace_install_file() {
  TMP_REPLACE="$(mktemp "$2.new.XXXXXX")"
  install -o root -g root -m "$3" "$1" "$TMP_REPLACE"
  mv -f "$TMP_REPLACE" "$2"
  TMP_REPLACE=""
}

wait_for_server_ready() {
  upgrade_probe_expected="$1"
  upgrade_probe_url="$2"
  upgrade_probe_deadline=$(( $(date +%s) + READINESS_TIMEOUT ))
  upgrade_probe_previous_pid=""
  [ -z "$TMP_READY_HEADERS" ] || rm -f "$TMP_READY_HEADERS"
  TMP_READY_HEADERS="$(mktemp "${TMPDIR:-/tmp}/nodelite-ready-headers.XXXXXX")" || return 1
  while [ "$(date +%s)" -lt "$upgrade_probe_deadline" ]; do
    upgrade_probe_pid="$(timeout --kill-after=2 5 systemctl show \
      --property=MainPID --value "$SERVICE_NAME.service")" || upgrade_probe_pid=0
    case "$upgrade_probe_pid" in ''|0|*[!0-9]*) upgrade_probe_pid=0 ;; esac
    upgrade_probe_code=""
    if [ "$upgrade_probe_pid" != 0 ] && \
        timeout --kill-after=2 5 systemctl is-active --quiet "$SERVICE_NAME.service"; then
      upgrade_probe_code="$(curl -sS --noproxy '*' --connect-timeout 1 --max-time 2 \
        --max-filesize 1048576 -D "$TMP_READY_HEADERS" -o /dev/null -w '%{http_code}' \
        "$upgrade_probe_url" 2>/dev/null)" || upgrade_probe_code=""
    fi
    if [ "$upgrade_probe_code" = 200 ]; then
      upgrade_probe_version="$(awk '
        tolower($1) == "x-nodelite-version:" { sub(/\r$/, "", $2); print $2 }
      ' "$TMP_READY_HEADERS")"
      if [ -n "$upgrade_probe_expected" ] && [ "$upgrade_probe_version" != "$upgrade_probe_expected" ]; then
        printf '%s\n' "install-server: ready endpoint version does not match $upgrade_probe_expected" >&2
        rm -f "$TMP_READY_HEADERS"
        return 1
      fi
      if [ "$upgrade_probe_previous_pid" = "$upgrade_probe_pid" ]; then
        rm -f "$TMP_READY_HEADERS"
        return 0
      fi
      upgrade_probe_previous_pid="$upgrade_probe_pid"
    else
      upgrade_probe_previous_pid=""
    fi
    sleep 1
  done
  rm -f "$TMP_READY_HEADERS"
  printf '%s\n' "install-server: service did not become ready within $READINESS_TIMEOUT seconds" >&2
  return 1
}

verify_upgrade_backup() {
  [ "$UPGRADE_BACKUP_READY" -eq 1 ] || return 1
  [ "$(calculate_sha256 "$UPGRADE_BACKUP_DIR/checksums")" = "$UPGRADE_BACKUP_DIGEST" ] \
    || return 1
  while read -r upgrade_verify_digest upgrade_verify_file; do
    [ "$(calculate_sha256 "$UPGRADE_BACKUP_DIR/$upgrade_verify_file")" = "$upgrade_verify_digest" ] \
      || return 1
  done <"$UPGRADE_BACKUP_DIR/checksums"
}

restore_upgrade_entry() {
  upgrade_restore_path="$(cat "$UPGRADE_BACKUP_DIR/$1.path")" || return 1
  # Removing absent SQLite sidecars is as important as restoring the main database.
  rm -f -- "$upgrade_restore_path" || return 1
  if [ -f "$UPGRADE_BACKUP_DIR/$1.tar" ]; then
    tar -xpPf "$UPGRADE_BACKUP_DIR/$1.tar" || return 1
  else
    [ -f "$UPGRADE_BACKUP_DIR/$1.absent" ] || return 1
  fi
}

restore_upgrade_files() {
  verify_upgrade_backup || return 1
  upgrade_restore_count="$(cat "$UPGRADE_BACKUP_DIR/count")" || return 1
  # An older executable is only restored after every compatible data file succeeds.
  for upgrade_restore_phase in data executable; do
    upgrade_restore_index=1
    while [ "$upgrade_restore_index" -le "$upgrade_restore_count" ]; do
      upgrade_restore_path="$(cat "$UPGRADE_BACKUP_DIR/$upgrade_restore_index.path")" || return 1
      case "$upgrade_restore_path" in
        "$BIN_PATH"|"$UNIT_PATH"|"$UPGRADE_BIN_TARGET"|"$UPGRADE_UNIT_TARGET") upgrade_entry_phase=executable ;;
        *) upgrade_entry_phase=data ;;
      esac
      if [ "$upgrade_restore_phase" = "$upgrade_entry_phase" ]; then
        restore_upgrade_entry "$upgrade_restore_index" || return 1
      fi
      upgrade_restore_index=$((upgrade_restore_index + 1))
    done
  done
}

recover_upgrade() {
  [ "$UPGRADE_STARTED" -eq 1 ] && [ "$UPGRADE_COMMITTED" -eq 0 ] || return 0
  if [ "$UPGRADE_MUTATED" -eq 1 ]; then
    stop_upgrade_service || return 1
    restore_upgrade_files || return 1
    timeout --kill-after=5 20 systemctl daemon-reload || return 1
    printf '%s\n' 'Rollback restored compatible data, configuration and executable.' >&2
  fi
  if [ "$UPGRADE_WAS_ACTIVE" -eq 1 ]; then
    UPGRADE_READY_URL="$UPGRADE_OLD_READY_URL"
    timeout --kill-after=5 25 systemctl start "$SERVICE_NAME.service" || return 1
    # Previous releases do not expose a version header.
    wait_for_server_ready "" "$UPGRADE_OLD_READY_URL" || return 1
    printf '%s\n' 'Previous NodeLite service is ready again; the upgrade failed.' >&2
  fi
  if [ "$UPGRADE_BACKUP_READY" -eq 1 ]; then
    printf '%s\n' 'Rolled back; original service activation state restored.' >"$UPGRADE_BACKUP_DIR/status" \
      || return 1
  fi
}
