#!/usr/bin/env bash
# Qualify the published authoring dependency against the archive Worker reader.
set -euo pipefail
if [[ $# -ne 2 ]]; then
  echo "usage: $0 QUALIFIED_BROWSER_AUTHORING_CRATE ARCHIVE_WASM_JS_DIRECTORY" >&2
  exit 2
fi
source_crate=$(realpath "$1")
archive_bindings=$(realpath "$2")
qualified_revision=b05c857887f01ecc51d820ffdfb7ade5f2b28d83
if [[ $(git -C "$source_crate" rev-parse HEAD) != "$qualified_revision" ]]; then
  echo "authoring test source does not match the qualified revision" >&2
  exit 2
fi
if ! git -C "$source_crate" diff --quiet HEAD -- .; then
  echo "qualified authoring test source has local modifications" >&2
  exit 2
fi
archive_checkout=$(cd "$(dirname "$0")/.." && pwd)
authoring_root=$(mktemp -d /tmp/archive-authoring-worker.XXXXXX)
server_pid=
cleanup() {
  if [[ -n "$server_pid" ]]; then kill "$server_pid" 2>/dev/null || true; fi
  echo "authoring Worker evidence: $authoring_root" >&2
}
trap cleanup EXIT
cp -R "$source_crate" "$authoring_root/crate"
python3 - "$authoring_root/crate/Cargo.toml" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text()
old = 'ms-package = { path = "../..", features = ["write"] }'
if text.count(old) != 1:
    raise SystemExit('qualified authoring test facade manifest changed')
path.write_text(text.replace(old, 'ms-package = { version = "=0.2.2", features = ["write"] }'))
PY
cp "$archive_checkout/crates/archive-wasm/tests/authoring-Cargo.lock" "$authoring_root/crate/Cargo.lock"
cargo build --manifest-path "$authoring_root/crate/Cargo.toml" --target wasm32-unknown-unknown --locked
target_root=$(cargo metadata --manifest-path "$authoring_root/crate/Cargo.toml" --no-deps --format-version 1 --locked | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
wasm-bindgen "$target_root/wasm32-unknown-unknown/debug/ms_package_authoring_worker_check.wasm" --target web --out-dir "$authoring_root/authoring"
cargo run --manifest-path "$authoring_root/crate/Cargo.toml" --example native --locked -- "$authoring_root/native.msix"
cp "$archive_checkout/crates/archive-wasm/tests/authoring-browser.html" "$authoring_root/browser.html"
cp "$archive_checkout/crates/archive-wasm/tests/authoring-worker.js" "$authoring_root/authoring-worker.js"
port=${ARCHIVE_AUTHORING_PORT:-8796}
export ARCHIVE_CDP_PORT=${ARCHIVE_CDP_PORT:-9796}
node "$archive_checkout/scripts/archive-browser-server.mjs" "$authoring_root" "$port" "$archive_bindings" >"$authoring_root/server.log" 2>&1 &
server_pid=$!
node "$archive_checkout/scripts/check-archive-browser.mjs" "http://127.0.0.1:$port" | tee "$authoring_root/result.json"
