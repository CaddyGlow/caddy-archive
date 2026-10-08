#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
output=${ARCHIVE_FUZZ_OUTPUT:-"$root/target/archive-fuzz/run-$(date -u +%Y%m%dT%H%M%SZ)-$$"}
iterations=${ARCHIVE_FUZZ_ITERATIONS:-10000}
targets=${ARCHIVE_FUZZ_TARGETS:-"archive package optical"}
[[ "$output" = /* ]] || { printf 'output must be absolute\n' >&2; exit 2; }
[[ "$iterations" =~ ^[1-9][0-9]*$ ]] || { printf 'iterations must be positive\n' >&2; exit 2; }
mkdir -p "$output/corpus/archive" "$output/corpus/package" "$output/corpus/optical" "$output/logs" "$output/workspace"
cd "$root"
cargo run --locked -p caddy-archive-core --features bzip2,brotli --example fuzz_seeds -- "$output/corpus"
(cd crates/archive-wasm/tests/fixtures/package && sha256sum --check SHA256SUMS)
find crates/archive-wasm/tests/fixtures/package -type f \( -name '*.msix' -o -name '*.msixbundle' -o -name '*.msi' -o -name '*.cab' \) -exec cp --backup=numbered '{}' "$output/corpus/package/" \;
cargo run --manifest-path fuzz/Cargo.toml --locked --bin seed_iso -- "$output/corpus/optical"
find crates/archive-core/tests/fixtures -type f \( -name '*.7z' -o -name '*.wim' -o -name '*.esd' \) -size -1048577c -exec cp --backup=numbered '{}' "$output/corpus/archive/" \;
# Extend the self-contained smoke corpus with optional local regression evidence.
for extra in ../wim-rs/crates/wim-format/tests/fixtures ../ms-package/tests/fixtures; do
    if [[ -d "$extra" ]]; then
        if [[ "$extra" == *wim-format* ]]; then
            find "$extra" -type f \( -name '*.wim' -o -name '*.esd' \) -size -1048577c -exec cp --backup=numbered '{}' "$output/corpus/archive/" \;
        else
            find "$extra" -type f -name '*.msi' -size -1048577c -exec cp --backup=numbered '{}' "$output/corpus/package/" \;
        fi
    fi
done
find "$output/corpus" -type f -exec sha256sum '{}' \; > "$output/seeds.sha256"
cargo hfuzz version > "$output/tool-version.txt" 2>&1
rustc -Vv >> "$output/tool-version.txt"
export CARGO_TARGET_DIR="$root/target/honggfuzz"
export HFUZZ_WORKSPACE="$output/workspace"
export HFUZZ_BUILD_ARGS=--locked
export RUSTC_WRAPPER= CARGO_INCREMENTAL=0 CC=gcc NIX_HARDENING_ENABLE=
cd "$root/fuzz"
for target in $targets; do
    case "$target" in archive|package|optical) ;; *) printf 'unknown fuzz target: %s\n' "$target" >&2; exit 2 ;; esac
    export HFUZZ_INPUT="$output/corpus/$target"
    export HFUZZ_RUN_ARGS="-n 1 -t 5 -N $iterations -F 1048576 --exit_upon_crash"
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
