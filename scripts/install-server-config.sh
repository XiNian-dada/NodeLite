#!/bin/sh
# Sourced only after the version-bound release checksum is verified.
# These settings and functions share variables with install-server.sh.
# shellcheck disable=SC2034,SC2153,SC2154

SERVER_STALE_AFTER_SECS="20"
SERVER_PING_INTERVAL_SECS="10"
SERVER_MAX_MESSAGE_BYTES="65536"
SERVER_HISTORY_QUERY_CONCURRENCY="4"
SERVER_HISTORY_READ_CACHE_KIB="512"
SERVER_HISTORY_WRITER_BATCH_MAX="128"
SERVER_HISTORY_WRITER_FLUSH_INTERVAL_MS="100"
SERVER_HELLO_TIMEOUT_SECS="10"
SERVER_MAX_OUTSTANDING_PINGS="32"
SERVER_INSECURE_TRANSPORT_WARN_INTERVAL_SECS="900"
SERVER_MAX_SANITIZED_DISKS="64"
SERVER_MAX_SANITIZED_STRING_BYTES="256"
SERVER_METRIC_ANOMALY_SESSION_LIMIT="5"
SERVER_SQLITE_BUSY_TIMEOUT_SECS="5"
SERVER_TOKEN_VERIFY_MAX_PARALLELISM="4"
WS_MAX_TOTAL_CONNECTIONS="1024"
WS_MAX_CONNECTIONS_PER_IP="32"
WS_AUTH_FAIL_WINDOW_SECS="300"
WS_AUTH_FAIL_MAX_ATTEMPTS="12"
WS_AUTH_BLOCK_SECS="900"
METRICS_EXPORT_NODE_RESOURCE_METRICS="false"
METRICS_EXPORT_NODE_DISK_METRICS="false"
AUDIT_ENABLED="true"
AUDIT_RETENTION_DAYS="90"
AUDIT_WRITER_BATCH_MAX="128"
AUDIT_WRITER_FLUSH_INTERVAL_MS="100"
AUDIT_LOG_SUCCESSFUL_AUTH="true"
AUDIT_LOG_FAILED_AUTH="true"
AUDIT_LOG_TOKEN_EVENTS="true"
AUDIT_LOG_RATE_LIMIT="true"
UI_REFRESH_INTERVAL_SECS="5"
IGNORED_FILESYSTEMS_RAW='["tmpfs", "devtmpfs", "overlay"]'
ALERTS_ENABLED="false"
ALERTS_SMTP_ENABLED="false"
ALERTS_SMTP_PORT="587"
ALERTS_SMTP_TRANSPORT="start_tls"
ALERTS_SMTP_SEND_RESOLVED="true"
ALERTS_WEBHOOK_ENABLED="false"
ALERTS_WEBHOOK_SEND_RESOLVED="true"
ALERTS_INSPECTION_ENABLED="false"
ALERTS_INSPECTION_LOCAL_TIME="09:00"
ALERTS_INSPECTION_LOOKBACK_HOURS="24"
ALERTS_INSPECTION_DELIVERY='["smtp"]'
ALERTS_INSPECTION_OFFLINE_GRACE_MINUTES="10"
ALERTS_INSPECTION_LATENCY_WARN_MS="250"
ALERTS_INSPECTION_CPU_WARN_PERCENT="85"
ALERTS_INSPECTION_MEMORY_WARN_PERCENT="90"

# 极简的 TOML 取值器:仅支持 `[section]` 下的"键 = 原始值"行,够升级流程用。
toml_get_raw() {
  file="$1"
  section="$2"
  key="$3"

  awk -v section="[$section]" -v key="$key" '
    /^\[/ {
      in_section = ($0 == section)
      next
    }
    in_section {
      line = $0
      sub(/^[[:space:]]+/, "", line)
      if (line ~ "^" key "[[:space:]]*=") {
        sub(/^[^=]+=[[:space:]]*/, "", line)
        print line
        exit
      }
    }
  ' "$file"
}

trim_whitespace() {
  printf '%s' "$1" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//'
}

strip_toml_string_quotes() {
  value="$(trim_whitespace "$1")"
  case "$value" in
    \"*\")
      value="${value#\"}"
      value="${value%\"}"
      ;;
  esac
  printf '%s' "$value"
}

toml_has_key() {
  file="$1"
  section="$2"
  key="$3"

  [ -n "$(toml_get_raw "$file" "$section" "$key")" ]
}

insert_toml_key() {
  file="$1"
  section="$2"
  line="$3"
  target_section="[$section]"
  config_dir="${file%/*}"
  if [ "$config_dir" = "$file" ]; then
    config_dir="."
  fi

  TMP_CONFIG="$(mktemp "$config_dir/server.toml.XXXXXX")" \
    || fail "failed to create temporary server config"
  if ! awk -v target_section="$target_section" -v new_line="$line" '
    BEGIN {
      in_target = 0
      inserted = 0
      seen_target = 0
    }
    {
      current = $0
      sub(/^[[:space:]]+/, "", current)
      sub(/[[:space:]]+$/, "", current)
      if (current ~ /^\[/) {
        if (in_target && !inserted) {
          print new_line
          inserted = 1
        }
        in_target = (current == target_section)
        if (in_target) {
          seen_target = 1
        }
      }
      print
    }
    END {
      if (!inserted) {
        if (!seen_target) {
          print ""
          print target_section
        }
        print new_line
      }
    }
  ' "$file" >"$TMP_CONFIG"; then
    fail "failed to supplement server config"
  fi
  cp "$TMP_CONFIG" "$file" || fail "failed to write supplemented server config"
  rm -f "$TMP_CONFIG"
  TMP_CONFIG=""
  CONFIG_DEFAULTS_ADDED=$((CONFIG_DEFAULTS_ADDED + 1))
}

ensure_toml_default() {
  file="$1"
  section="$2"
  key="$3"
  line="$4"

  if toml_has_key "$file" "$section" "$key"; then
    return 0
  fi
  insert_toml_key "$file" "$section" "$line"
}

# 升级 / 迁移时从已有 server.toml 读出默认值,避免重置用户的自定义配置。
load_existing_server_defaults() {
  config_path="$1"
  [ -f "$config_path" ] || return 0

  listen_value="$(strip_toml_string_quotes "$(toml_get_raw "$config_path" server listen)")"
  if [ -n "$listen_value" ] && [ "$listen_value" != "${listen_value%:*}" ]; then
    LISTEN_HOST_DEFAULT_VALUE="${listen_value%:*}"
    LISTEN_PORT_DEFAULT_VALUE="${listen_value##*:}"
  fi

  public_base_url="$(strip_toml_string_quotes "$(toml_get_raw "$config_path" server public_base_url)")"
  case "$public_base_url" in
    http://*)
      PUBLIC_SCHEME_DEFAULT_VALUE="http"
      PUBLIC_HOST_DEFAULT_VALUE="${public_base_url#http://}"
      ;;
    https://*)
      PUBLIC_SCHEME_DEFAULT_VALUE="https"
      PUBLIC_HOST_DEFAULT_VALUE="${public_base_url#https://}"
      ;;
  esac

  readonly_username="$(strip_toml_string_quotes "$(toml_get_raw "$config_path" auth username)")"
  if [ -n "$readonly_username" ]; then
    READONLY_USERNAME_DEFAULT_VALUE="$readonly_username"
  fi
  readonly_password="$(strip_toml_string_quotes "$(toml_get_raw "$config_path" auth password)")"
  if [ -n "$readonly_password" ]; then
    READONLY_PASSWORD_DEFAULT_VALUE="$readonly_password"
  fi

  value="$(trim_whitespace "$(toml_get_raw "$config_path" server stale_after_secs)")"
  [ -n "$value" ] && SERVER_STALE_AFTER_SECS="$value"
  value="$(trim_whitespace "$(toml_get_raw "$config_path" server ping_interval_secs)")"
  [ -n "$value" ] && SERVER_PING_INTERVAL_SECS="$value"
  value="$(trim_whitespace "$(toml_get_raw "$config_path" server max_message_bytes)")"
  [ -n "$value" ] && SERVER_MAX_MESSAGE_BYTES="$value"
  value="$(trim_whitespace "$(toml_get_raw "$config_path" server token_verify_max_parallelism)")"
  [ -n "$value" ] && SERVER_TOKEN_VERIFY_MAX_PARALLELISM="$value"
  value="$(trim_whitespace "$(toml_get_raw "$config_path" server history_query_concurrency)")"
  [ -n "$value" ] && SERVER_HISTORY_QUERY_CONCURRENCY="$value"
  value="$(trim_whitespace "$(toml_get_raw "$config_path" server history_read_cache_kib)")"
  [ -n "$value" ] && SERVER_HISTORY_READ_CACHE_KIB="$value"
  value="$(trim_whitespace "$(toml_get_raw "$config_path" server history_writer_batch_max)")"
  [ -n "$value" ] && SERVER_HISTORY_WRITER_BATCH_MAX="$value"
  value="$(trim_whitespace "$(toml_get_raw "$config_path" server history_writer_flush_interval_ms)")"
  [ -n "$value" ] && SERVER_HISTORY_WRITER_FLUSH_INTERVAL_MS="$value"
  value="$(trim_whitespace "$(toml_get_raw "$config_path" audit writer_batch_max)")"
  [ -n "$value" ] && AUDIT_WRITER_BATCH_MAX="$value"
  value="$(trim_whitespace "$(toml_get_raw "$config_path" audit writer_flush_interval_ms)")"
  [ -n "$value" ] && AUDIT_WRITER_FLUSH_INTERVAL_MS="$value"
  value="$(trim_whitespace "$(toml_get_raw "$config_path" ws max_total_connections)")"
  [ -n "$value" ] && WS_MAX_TOTAL_CONNECTIONS="$value"
  value="$(trim_whitespace "$(toml_get_raw "$config_path" ws max_connections_per_ip)")"
  [ -n "$value" ] && WS_MAX_CONNECTIONS_PER_IP="$value"
  value="$(trim_whitespace "$(toml_get_raw "$config_path" ws auth_fail_window_secs)")"
  [ -n "$value" ] && WS_AUTH_FAIL_WINDOW_SECS="$value"
  value="$(trim_whitespace "$(toml_get_raw "$config_path" ws auth_fail_max_attempts)")"
  [ -n "$value" ] && WS_AUTH_FAIL_MAX_ATTEMPTS="$value"
  value="$(trim_whitespace "$(toml_get_raw "$config_path" ws auth_block_secs)")"
  [ -n "$value" ] && WS_AUTH_BLOCK_SECS="$value"
  value="$(trim_whitespace "$(toml_get_raw "$config_path" ui refresh_interval_secs)")"
  [ -n "$value" ] && UI_REFRESH_INTERVAL_SECS="$value"
  value="$(trim_whitespace "$(toml_get_raw "$config_path" filters ignored_filesystems)")"
  [ -n "$value" ] && IGNORED_FILESYSTEMS_RAW="$value"
  value="$(trim_whitespace "$(toml_get_raw "$config_path" geoip enabled)")"
  [ -n "$value" ] && GEOIP_ENABLED="$value"
  value="$(strip_toml_string_quotes "$(toml_get_raw "$config_path" geoip provider)")"
  [ -n "$value" ] && GEOIP_PROVIDER="$value"
  value="$(strip_toml_string_quotes "$(toml_get_raw "$config_path" geoip edition)")"
  [ -n "$value" ] && GEOIP_EDITION="$value"
  value="$(strip_toml_string_quotes "$(toml_get_raw "$config_path" geoip database_path)")"
  [ -n "$value" ] && GEOIP_DATABASE_PATH="$value"
  value="$(trim_whitespace "$(toml_get_raw "$config_path" geoip auto_update)")"
  [ -n "$value" ] && GEOIP_AUTO_UPDATE="$value"
  value="$(trim_whitespace "$(toml_get_raw "$config_path" geoip update_interval_days)")"
  [ -n "$value" ] && GEOIP_UPDATE_INTERVAL_DAYS="$value"

  return 0
}

complete_server_config_defaults() {
  config_path="$1"
  audit_db_path="${DATA_DIR}/audit.sqlite3"
  geoip_database_path="${GEOIP_DATABASE_PATH:-${DATA_DIR}/geoip/dbip.mmdb}"

  ensure_toml_default "$config_path" server insecure_allow_http "insecure_allow_http = false"
  ensure_toml_default "$config_path" server trusted_proxies "trusted_proxies = []"
  ensure_toml_default "$config_path" server node_registry_path "node_registry_path = \"$REGISTRY_PATH\""
  ensure_toml_default "$config_path" server history_db_path "history_db_path = \"$DATA_DIR/history.sqlite3\""
  ensure_toml_default "$config_path" server history_query_concurrency "history_query_concurrency = $SERVER_HISTORY_QUERY_CONCURRENCY"
  ensure_toml_default "$config_path" server history_read_cache_kib "history_read_cache_kib = $SERVER_HISTORY_READ_CACHE_KIB"
  ensure_toml_default "$config_path" server history_writer_batch_max "history_writer_batch_max = $SERVER_HISTORY_WRITER_BATCH_MAX"
  ensure_toml_default "$config_path" server history_writer_flush_interval_ms "history_writer_flush_interval_ms = $SERVER_HISTORY_WRITER_FLUSH_INTERVAL_MS"
  ensure_toml_default "$config_path" server snapshot_path "snapshot_path = \"$DATA_DIR/snapshot.json\""
  ensure_toml_default "$config_path" server stale_after_secs "stale_after_secs = $SERVER_STALE_AFTER_SECS"
  ensure_toml_default "$config_path" server ping_interval_secs "ping_interval_secs = $SERVER_PING_INTERVAL_SECS"
  ensure_toml_default "$config_path" server max_message_bytes "max_message_bytes = $SERVER_MAX_MESSAGE_BYTES"
  ensure_toml_default "$config_path" server hello_timeout_secs "hello_timeout_secs = $SERVER_HELLO_TIMEOUT_SECS"
  ensure_toml_default "$config_path" server max_outstanding_pings "max_outstanding_pings = $SERVER_MAX_OUTSTANDING_PINGS"
  ensure_toml_default "$config_path" server insecure_transport_warn_interval_secs "insecure_transport_warn_interval_secs = $SERVER_INSECURE_TRANSPORT_WARN_INTERVAL_SECS"
  ensure_toml_default "$config_path" server max_sanitized_disks "max_sanitized_disks = $SERVER_MAX_SANITIZED_DISKS"
  ensure_toml_default "$config_path" server max_sanitized_string_bytes "max_sanitized_string_bytes = $SERVER_MAX_SANITIZED_STRING_BYTES"
  ensure_toml_default "$config_path" server metric_anomaly_session_limit "metric_anomaly_session_limit = $SERVER_METRIC_ANOMALY_SESSION_LIMIT"
  ensure_toml_default "$config_path" server sqlite_busy_timeout_secs "sqlite_busy_timeout_secs = $SERVER_SQLITE_BUSY_TIMEOUT_SECS"
  ensure_toml_default "$config_path" server token_verify_max_parallelism "token_verify_max_parallelism = $SERVER_TOKEN_VERIFY_MAX_PARALLELISM"

  ensure_toml_default "$config_path" auth enable_2fa "enable_2fa = false"

  ensure_toml_default "$config_path" metrics export_node_resource_metrics "export_node_resource_metrics = $METRICS_EXPORT_NODE_RESOURCE_METRICS"
  ensure_toml_default "$config_path" metrics export_node_disk_metrics "export_node_disk_metrics = $METRICS_EXPORT_NODE_DISK_METRICS"

  ensure_toml_default "$config_path" audit enabled "enabled = $AUDIT_ENABLED"
  ensure_toml_default "$config_path" audit db_path "db_path = \"$audit_db_path\""
  ensure_toml_default "$config_path" audit retention_days "retention_days = $AUDIT_RETENTION_DAYS"
  ensure_toml_default "$config_path" audit writer_batch_max "writer_batch_max = $AUDIT_WRITER_BATCH_MAX"
  ensure_toml_default "$config_path" audit writer_flush_interval_ms "writer_flush_interval_ms = $AUDIT_WRITER_FLUSH_INTERVAL_MS"
  ensure_toml_default "$config_path" audit log_successful_auth "log_successful_auth = $AUDIT_LOG_SUCCESSFUL_AUTH"
  ensure_toml_default "$config_path" audit log_failed_auth "log_failed_auth = $AUDIT_LOG_FAILED_AUTH"
  ensure_toml_default "$config_path" audit log_token_events "log_token_events = $AUDIT_LOG_TOKEN_EVENTS"
  ensure_toml_default "$config_path" audit log_rate_limit "log_rate_limit = $AUDIT_LOG_RATE_LIMIT"

  ensure_toml_default "$config_path" ws max_total_connections "max_total_connections = $WS_MAX_TOTAL_CONNECTIONS"
  ensure_toml_default "$config_path" ws max_connections_per_ip "max_connections_per_ip = $WS_MAX_CONNECTIONS_PER_IP"
  ensure_toml_default "$config_path" ws auth_fail_window_secs "auth_fail_window_secs = $WS_AUTH_FAIL_WINDOW_SECS"
  ensure_toml_default "$config_path" ws auth_fail_max_attempts "auth_fail_max_attempts = $WS_AUTH_FAIL_MAX_ATTEMPTS"
  ensure_toml_default "$config_path" ws auth_block_secs "auth_block_secs = $WS_AUTH_BLOCK_SECS"

  ensure_toml_default "$config_path" ui refresh_interval_secs "refresh_interval_secs = $UI_REFRESH_INTERVAL_SECS"

  ensure_toml_default "$config_path" geoip enabled "enabled = $GEOIP_ENABLED"
  ensure_toml_default "$config_path" geoip provider "provider = \"$GEOIP_PROVIDER\""
  ensure_toml_default "$config_path" geoip edition "edition = \"$GEOIP_EDITION\""
  ensure_toml_default "$config_path" geoip database_path "database_path = \"$geoip_database_path\""
  ensure_toml_default "$config_path" geoip auto_update "auto_update = $GEOIP_AUTO_UPDATE"
  ensure_toml_default "$config_path" geoip update_interval_days "update_interval_days = $GEOIP_UPDATE_INTERVAL_DAYS"

  ensure_toml_default "$config_path" filters ignored_filesystems "ignored_filesystems = $IGNORED_FILESYSTEMS_RAW"

  ensure_toml_default "$config_path" alerts enabled "enabled = $ALERTS_ENABLED"
  ensure_toml_default "$config_path" alerts.smtp enabled "enabled = $ALERTS_SMTP_ENABLED"
  ensure_toml_default "$config_path" alerts.smtp host "host = \"\""
  ensure_toml_default "$config_path" alerts.smtp port "port = $ALERTS_SMTP_PORT"
  ensure_toml_default "$config_path" alerts.smtp username "username = \"\""
  ensure_toml_default "$config_path" alerts.smtp sender "sender = \"\""
  ensure_toml_default "$config_path" alerts.smtp recipients "recipients = []"
  ensure_toml_default "$config_path" alerts.smtp transport "transport = \"$ALERTS_SMTP_TRANSPORT\""
  ensure_toml_default "$config_path" alerts.smtp send_resolved "send_resolved = $ALERTS_SMTP_SEND_RESOLVED"
  ensure_toml_default "$config_path" alerts.webhook enabled "enabled = $ALERTS_WEBHOOK_ENABLED"
  ensure_toml_default "$config_path" alerts.webhook url "url = \"\""
  ensure_toml_default "$config_path" alerts.webhook send_resolved "send_resolved = $ALERTS_WEBHOOK_SEND_RESOLVED"
  ensure_toml_default "$config_path" alerts.inspection enabled "enabled = $ALERTS_INSPECTION_ENABLED"
  ensure_toml_default "$config_path" alerts.inspection local_time "local_time = \"$ALERTS_INSPECTION_LOCAL_TIME\""
  ensure_toml_default "$config_path" alerts.inspection lookback_hours "lookback_hours = $ALERTS_INSPECTION_LOOKBACK_HOURS"
  ensure_toml_default "$config_path" alerts.inspection delivery "delivery = $ALERTS_INSPECTION_DELIVERY"
  ensure_toml_default "$config_path" alerts.inspection offline_grace_minutes "offline_grace_minutes = $ALERTS_INSPECTION_OFFLINE_GRACE_MINUTES"
  ensure_toml_default "$config_path" alerts.inspection latency_warn_ms "latency_warn_ms = $ALERTS_INSPECTION_LATENCY_WARN_MS"
  ensure_toml_default "$config_path" alerts.inspection cpu_warn_percent "cpu_warn_percent = $ALERTS_INSPECTION_CPU_WARN_PERCENT"
  ensure_toml_default "$config_path" alerts.inspection memory_warn_percent "memory_warn_percent = $ALERTS_INSPECTION_MEMORY_WARN_PERCENT"

  chmod 0600 "$config_path"
}

# 把交互或默认得到的变量拼成最终 server.toml 文本。
render_server_config() {
  insecure_allow_http_value="false"
  if [ "$PUBLIC_SCHEME" = "http" ]; then
    insecure_allow_http_value="true"
  fi
  geoip_database_path="${GEOIP_DATABASE_PATH:-${DATA_DIR}/geoip/dbip.mmdb}"
  cat <<EOF
[server]
listen = "${LISTEN_HOST}:${LISTEN_PORT}"
public_base_url = "${PUBLIC_SCHEME}://${PUBLIC_HOST}"
insecure_allow_http = ${insecure_allow_http_value}
node_registry_path = "${CONFIG_DIR}/server.json"
history_db_path = "${DATA_DIR}/history.sqlite3"
history_query_concurrency = ${SERVER_HISTORY_QUERY_CONCURRENCY}
history_read_cache_kib = ${SERVER_HISTORY_READ_CACHE_KIB}
history_writer_batch_max = ${SERVER_HISTORY_WRITER_BATCH_MAX}
history_writer_flush_interval_ms = ${SERVER_HISTORY_WRITER_FLUSH_INTERVAL_MS}
snapshot_path = "${DATA_DIR}/snapshot.json"
stale_after_secs = ${SERVER_STALE_AFTER_SECS}
ping_interval_secs = ${SERVER_PING_INTERVAL_SECS}
max_message_bytes = ${SERVER_MAX_MESSAGE_BYTES}
token_verify_max_parallelism = ${SERVER_TOKEN_VERIFY_MAX_PARALLELISM}

[auth]
username = "${READONLY_USERNAME}"
password = "${READONLY_PASSWORD}"

[ws]
max_total_connections = ${WS_MAX_TOTAL_CONNECTIONS}
max_connections_per_ip = ${WS_MAX_CONNECTIONS_PER_IP}
auth_fail_window_secs = ${WS_AUTH_FAIL_WINDOW_SECS}
auth_fail_max_attempts = ${WS_AUTH_FAIL_MAX_ATTEMPTS}
auth_block_secs = ${WS_AUTH_BLOCK_SECS}

[ui]
refresh_interval_secs = ${UI_REFRESH_INTERVAL_SECS}

[geoip]
enabled = ${GEOIP_ENABLED}
provider = "${GEOIP_PROVIDER}"
edition = "${GEOIP_EDITION}"
database_path = "${geoip_database_path}"
auto_update = ${GEOIP_AUTO_UPDATE}
update_interval_days = ${GEOIP_UPDATE_INTERVAL_DAYS}

[filters]
ignored_filesystems = ${IGNORED_FILESYSTEMS_RAW}
EOF
}
