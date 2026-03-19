#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

echo "Building kp-github-mcp..."
cd "$PROJECT_DIR"
RUST_MIN_STACK=16777216 cargo build --release

BINARY="$PROJECT_DIR/target/release/kp-github-mcp"
echo "Binary: $BINARY ($(du -h "$BINARY" | cut -f1))"

echo "Registering with Claude Code..."
claude mcp add kp-github --transport stdio -- "$BINARY"

echo ""
echo "Done! kp-github-mcp registered."
echo "To disable the official GitHub plugin, set github@claude-plugins-official: false in ~/.claude/settings.json"
echo "To test: ask Claude to 'list issues in owner/repo using kp-github'"
