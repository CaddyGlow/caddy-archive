#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
output=${UDF_FUZZ_OUTPUT:-"$root/target/udf-fuzz/run-$(date -u +%Y%m%dT%H%M%SZ)-$$"}
iterations=${UDF_FUZZ_ITERATIONS:-10000}
[[ "$output" = /* ]] || { printf 'output must be absolute\n' >&2; exit 2; }
[[ "$iterations" =~ ^[1-9][0-9]*$ ]] || { printf 'iterations must be positive\n' >&2; exit 2; }
mkdir -p "$output/corpus" "$output/logs" "$output/workspace" "$output/tmp"
cd "$root"
cargo run --manifest-path fuzz/Cargo.toml --locked --bin seed_udf -- "$output/corpus"
find "$output/corpus" -type f -exec sha256sum '{}' \; > "$output/seeds.sha256"
sha256sum fuzz/Cargo.lock > "$output/lock.sha256"
cargo hfuzz version > "$output/tool-version.txt" 2>&1
rustc -Vv >> "$output/tool-version.txt"
export CARGO_TARGET_DIR="$root/target/honggfuzz"
export HFUZZ_WORKSPACE="$output/workspace" TMPDIR="$output/tmp"
export HFUZZ_BUILD_ARGS=--locked
export RUSTC_WRAPPER= CARGO_INCREMENTAL=0 CC=gcc NIX_HARDENING_ENABLE=
cd "$root/fuzz"
for target in udf udf_roundtrip; do
    cap=4194304
    [[ "$target" != udf_roundtrip ]] || cap=65536
    export HFUZZ_INPUT="$output/corpus/$target"
    export HFUZZ_RUN_ARGS="-n 1 -t 5 -N $iterations -F $cap --exit_upon_crash"
    printf '%s\n' "$HFUZZ_RUN_ARGS" > "$output/logs/$target.args"
    cargo hfuzz run "$target" 2>&1 | tee "$output/logs/$target.log"
    if ! grep -Eq 'Summary iterations:[0-9]+ .*crashes_count:0 timeout_count:0 ' "$output/logs/$target.log"; then
        printf 'fuzz target %s reported a finding or incomplete summary; evidence: %s\n' "$target" "$output" >&2
        exit 1
    fi
    completed=$(sed -n 's/^Summary iterations:\([0-9]*\) .*/\1/p' "$output/logs/$target.log" | tail -n 1)
    if [[ -z "$completed" ]] || (( completed < iterations )); then
        printf 'fuzz target %s did not reach requested iterations\n' "$target" >&2
        exit 1
    fi
done
