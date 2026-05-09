#!/bin/sh
set -eu

fail() {
  printf '%s\n' "install-agent: $*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

toml_escape() {
  printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

SERVER=""
NODE_ID=""
NODE_LABEL=""
TOKEN=""
INSTALL_DIR="/usr/local/bin"
CONFIG_DIR="/etc/ximonitor"
BASE_URL="${XIMONITOR_AGENT_BASE_URL:-https://example.invalid/ximonitor/releases/latest/download}"
BINARY_URL="${XIMONITOR_AGENT_BINARY_URL:-}"
SHA256_X86_64="${XIMONITOR_AGENT_SHA256_X86_64:-}"
SHA256_AARCH64="${XIMONITOR_AGENT_SHA256_AARCH64:-}"

calculate_sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | sed 's/[[:space:]].*$//'
    return 0
  fi
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | sed 's/[[:space:]].*$//'
    return 0
  fi
  fail "missing required command: sha256sum or shasum"
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --server)
      [ "$#" -ge 2 ] || fail "--server requires a value"
      SERVER="$2"
      shift 2
      ;;
    --node-id)
      [ "$#" -ge 2 ] || fail "--node-id requires a value"
      NODE_ID="$2"
      shift 2
      ;;
    --node-label)
      [ "$#" -ge 2 ] || fail "--node-label requires a value"
      NODE_LABEL="$2"
      shift 2
      ;;
    --token)
      [ "$#" -ge 2 ] || fail "--token requires a value"
      TOKEN="$2"
      shift 2
      ;;
    --install-dir)
      [ "$#" -ge 2 ] || fail "--install-dir requires a value"
      INSTALL_DIR="$2"
      shift 2
      ;;
    --config-dir)
      [ "$#" -ge 2 ] || fail "--config-dir requires a value"
      CONFIG_DIR="$2"
      shift 2
      ;;
    --base-url)
      [ "$#" -ge 2 ] || fail "--base-url requires a value"
      BASE_URL="$2"
      shift 2
      ;;
    --binary-url)
      [ "$#" -ge 2 ] || fail "--binary-url requires a value"
      BINARY_URL="$2"
      shift 2
      ;;
    --sha256-x86_64)
      [ "$#" -ge 2 ] || fail "--sha256-x86_64 requires a value"
      SHA256_X86_64="$2"
      shift 2
      ;;
    --sha256-aarch64)
      [ "$#" -ge 2 ] || fail "--sha256-aarch64 requires a value"
      SHA256_AARCH64="$2"
      shift 2
      ;;
    --help|-h)
      cat <<'EOF'
Usage:
  sh install-agent.sh \
    --server wss://monitor.example.com/ws \
    --node-id hk-01 \
    --token YOUR_TOKEN \
    --sha256-x86_64 <sha256> \
    --sha256-aarch64 <sha256>

Optional:
  --node-label <label>
  --install-dir <dir>
  --config-dir <dir>
  --base-url <release-base-url>
  --binary-url <exact-binary-url>
EOF
      exit 0
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
done

[ "$(id -u)" -eq 0 ] || fail "please run as root"
[ -n "$SERVER" ] || fail "missing --server"
[ -n "$NODE_ID" ] || fail "missing --node-id"
[ -n "$TOKEN" ] || fail "missing --token"

if [ -z "$NODE_LABEL" ]; then
  NODE_LABEL="$NODE_ID"
fi

need_cmd uname
need_cmd curl
need_cmd sed
need_cmd mkdir
need_cmd install
need_cmd systemctl

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64|amd64)
    TARGET="x86_64-unknown-linux-musl"
    EXPECTED_SHA256="$SHA256_X86_64"
    ;;
  aarch64|arm64)
    TARGET="aarch64-unknown-linux-musl"
    EXPECTED_SHA256="$SHA256_AARCH64"
    ;;
  *)
    fail "unsupported architecture: $ARCH"
    ;;
esac

[ -n "$EXPECTED_SHA256" ] || fail "missing expected sha256 for target $TARGET"

if [ -n "$BINARY_URL" ]; then
  DOWNLOAD_URL="$BINARY_URL"
else
  DOWNLOAD_URL="$BASE_URL/ximonitor-agent-$TARGET"
fi

BIN_PATH="$INSTALL_DIR/ximonitor-agent"
TMP_PATH="$BIN_PATH.tmp"
CONFIG_PATH="$CONFIG_DIR/agent.toml"
UNIT_PATH="/etc/systemd/system/ximonitor-agent.service"

mkdir -p "$INSTALL_DIR" "$CONFIG_DIR"

printf '%s\n' "Downloading $DOWNLOAD_URL"
curl -fsSL "$DOWNLOAD_URL" -o "$TMP_PATH" || fail "failed to download agent binary"
ACTUAL_SHA256="$(calculate_sha256 "$TMP_PATH")"
[ "$ACTUAL_SHA256" = "$EXPECTED_SHA256" ] || fail "downloaded agent checksum mismatch"
chmod 0755 "$TMP_PATH"
mv "$TMP_PATH" "$BIN_PATH"

cat >"$CONFIG_PATH" <<EOF
[agent]
node_id = "$(toml_escape "$NODE_ID")"
node_label = "$(toml_escape "$NODE_LABEL")"
server = "$(toml_escape "$SERVER")"
token = "$(toml_escape "$TOKEN")"
report_interval_secs = 5
EOF
chmod 0600 "$CONFIG_PATH"

cat >"$UNIT_PATH" <<EOF
[Unit]
Description=XiMonitor Agent
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=$BIN_PATH --config $CONFIG_PATH
Restart=always
RestartSec=3
User=root

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable ximonitor-agent.service
systemctl restart ximonitor-agent.service

printf '%s\n' "XiMonitor agent installed and started."
printf '%s\n' "Config: $CONFIG_PATH"
printf '%s\n' "Service: ximonitor-agent.service"
