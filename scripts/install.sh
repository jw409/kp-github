#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# An extracted consumer archive already contains its verified native payload.
if [[ -f "$PROJECT_DIR/MANIFEST.json" && -x "$PROJECT_DIR/bin/kp-github-mcp" ]]; then
    BINARY="$PROJECT_DIR/bin/kp-github-mcp"
else
    echo "Building kp-github-mcp..."
    RUST_MIN_STACK=16777216 cargo build --locked --release --manifest-path "$PROJECT_DIR/Cargo.toml"
    BINARY="${CARGO_TARGET_DIR:-$PROJECT_DIR/target}/release/kp-github-mcp"
fi

exec python3 "$SCRIPT_DIR/install_local_http.py" --binary "$BINARY" "$@"
