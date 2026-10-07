# Archive fuzzing

The archive, package, optical, cab, udf, and udf_roundtrip targets live here.
Run `cargo test --manifest-path fuzz/Cargo.toml --locked` for regression checks.
From the repository root, `scripts/archive-fuzz-smoke.sh` and
`scripts/udf-fuzz-smoke.sh` generate seeds and run bounded instrumented campaigns.
They require honggfuzz 0.5.62 and the Nix development shell's native dependencies.
Run `cargo run --manifest-path fuzz/Cargo.toml --locked --bin replay -- TARGET FILE`
to reproduce inputs. Keep failures and historical campaign evidence.
The windows-uup fuzz package re-exports these harnesses for existing commands.
