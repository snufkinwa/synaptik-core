#!/usr/bin/env bash
set -euo pipefail

# --- paths (edit if needed)
BIN_DIR="/usr/local/bin"
CONF_DIR="/etc/synaptik"
UNIT_DIR="/etc/systemd/system"

# --- inputs
# Determine repository root relative to this script (handles invocation from other dirs)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$SCRIPT_DIR/.."
WORKSPACE_DIR="$REPO_ROOT/synaptik-workspace"

# Built artifacts live under synaptik-workspace/target/release
AGENT_BIN="$WORKSPACE_DIR/target/release/synaptik-agent"
SIGNER_BIN="$WORKSPACE_DIR/target/release/contracts-signer"
CONF_SRC="synaptik-workspace/examples/synaptik-agent.toml"
SERVICE_SRC="deploy/systemd/synaptik-agent.service"
TIMER_SRC="deploy/systemd/synaptik-agent.timer"

echo "> Using workspace: $WORKSPACE_DIR"
echo "> Expecting agent bin: $AGENT_BIN"
echo "> Expecting signer bin: $SIGNER_BIN"

# --- build if missing
if [ ! -x "$AGENT_BIN" ] || [ ! -x "$SIGNER_BIN" ]; then
  echo "> Building synaptik-agent and contracts-signer (release)"
  (cd synaptik-workspace && cargo build -p synaptik-agent -p contracts-signer --release)
fi

# --- install binaries/config
sudo install -D -m 0755 "$AGENT_BIN" "$BIN_DIR/synaptik-agent"
sudo install -D -m 0755 "$SIGNER_BIN" "$BIN_DIR/contracts-signer"
sudo install -D -m 0644 "$CONF_SRC" "$CONF_DIR/synaptik-agent.toml"

# --- init local registry (file://)
sudo mkdir -p /var/lib/synaptik/registry/packs
sudo tee /var/lib/synaptik/registry/registry.jsonl >/dev/null <<'JSONL'
{"t":"2025-09-27T00:00:00Z","op":"PUBLISH","channel":"alpha","uri":"file:///var/lib/synaptik/registry/packs/pack-alpha-1.json"}
{"t":"2025-09-27T02:00:00Z","op":"PROMOTE","from":"alpha","to":"beta","version":"alpha#1"}
{"t":"2025-09-27T06:00:00Z","op":"PROMOTE","from":"beta","to":"stable","version":"alpha#1"}
JSONL

# drop an initial dummy pack (replace with real pack later)
sudo tee /var/lib/synaptik/registry/packs/pack-alpha-1.json >/dev/null <<'JSON'
{"version":"alpha#1","files":[],"blobs":{},"signature":null}
JSON

# --- wire systemd
sudo install -D -m 0644 "$SERVICE_SRC" "$UNIT_DIR/synaptik-agent.service"
sudo install -D -m 0644 "$TIMER_SRC" "$UNIT_DIR/synaptik-agent.timer"

# --- ensure system user & ownership (idempotent)
if ! getent group synaptik >/dev/null 2>&1; then
  echo "> Creating system group synaptik"
  sudo groupadd --system synaptik
fi
if ! id -u synaptik >/dev/null 2>&1; then
  echo "> Creating system user synaptik"
  sudo useradd \
    --system \
    --gid synaptik \
    --home-dir /var/lib/synaptik \
    --shell /usr/sbin/nologin \
    --comment "Synaptik Agent" \
    synaptik
fi

sudo mkdir -p /var/lib/synaptik
sudo chown -R synaptik:synaptik /var/lib/synaptik

echo "> User and directory ownership prepared"

sudo systemctl daemon-reload
sudo systemctl enable --now synaptik-agent.service
sudo systemctl enable --now synaptik-agent.timer

echo "✅ synaptik-agent installed and scheduled. Registry: file:///var/lib/synaptik/registry/registry.jsonl"

