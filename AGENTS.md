# Repository guidelines

This Rust 2024 workspace owns archive-core, archive-fs, archive-cli (arc),
and archive-wasm. The ms-package crate and patched MSI reader live in
the sibling ms-package repository.
Preserve crate names, licensing notices, test fixtures, and historical evidence.
Keep the sibling cabinet, ms-compress, wim-rs and mkiso-rs checkouts available.

Use rustfmt defaults. Run `cargo test --workspace --all-features --locked`,
`cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`,
and `cargo test --manifest-path fuzz/Cargo.toml --locked` for host validation.
Compile archive-wasm for wasm32-unknown-unknown and run the browser Worker
checks when changing browser integration. Fuzz campaigns, Windows filesystem
behavior and firmware boot validation remain separate gates. Preserve source
media, regression samples, and fuzz failure evidence.
