# Compression dependency update (2026-10-07)

Active lockfiles now resolve the shared ms-compress crate to the published
`ms-compress-ruzstd` 0.9.1 fork (MIT). Existing sibling paths to ms-compress
remain; the fork itself is fetched from crates.io with a recorded checksum.
Source licenses, fixture files, and historical evidence are unchanged.

Local Linux host validation used Rust 1.99.0 and locked dependencies:

* Rust formatting check passed.
* Workspace tests with all features passed.
* Strict Clippy for all workspace targets and features passed.
* Fuzz regression tests through fuzz/Cargo.toml passed.

Native Windows equivalence and sustained fuzz campaigns are separate gates.
