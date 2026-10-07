# archive-rs

Reusable archive libraries and tools, extracted from windows-uup.

- `archive-core`: bounded archive parsing, creation, and verification.
- `archive-fs`: filesystem extraction adapter.
- `archive-cli`: the `arc` command-line tool.
- `archive-wasm`: browser/Web Worker interface.
- Sibling `cabinet` repository: CAB reader/writer and optional makecab/cabextract tools.
- `../ms-package`: standalone `ms-package` crate for portable APPX/MSIX
  and MSI inspection, including its patched MSI reader.

Keep sibling `../cabinet`, `../ms-compress`, `../wim-rs`, and `../mkiso-rs` checkouts.
Compression, WIM, and the libmkiso library come from their own repositories.
No windows-uup checkout is required to build this workspace.
Boot-media assembly and the mkiso CLI live in mkiso-rs; servicing and UUP-specific
CAB validation stay in windows-uup. Its consumers and defender-rs depend on this workspace by path.

```sh
cargo run -p caddy-archive-cli --locked -- --help
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --manifest-path fuzz/Cargo.toml --locked
cargo build -p caddy-archive-wasm --features packages,crypto,sevenz,bzip2,brotli --target wasm32-unknown-unknown --locked
```

Run archive/browser/fuzz scripts from this root. See
[capabilities](docs/archive-capabilities.md), [CAB](../cabinet/README.md), and
[browser validation](crates/archive-wasm/README.md). Host tests do not establish
Windows filesystem behavior, bootability, or native installation correctness.
Retained evidence describes its original runs. Build caches and historical
campaign artifacts remain at their original windows-uup locations.

CI checks out the sibling repositories using `CABINET_REPOSITORY`, `MS_COMPRESS_REPOSITORY`,
`WIM_RS_REPOSITORY`, and `MKISO_RS_REPOSITORY`, plus corresponding `_REF`
variables. Defaults select same-owner repositories and their default branches.
Publish matching extractions before using the workflows.

## Publication

The crates.io packages are `caddy-archive-core`, `caddy-archive-fs`,
`caddy-archive-cli`, and `caddy-archive-wasm`. Rust library names remain
`archive_core`, `archive_fs`, and `archive_wasm`; the CLI remains `arc`.
The repository is https://github.com/CaddyGlow/caddy-archive.
For the first publication, manually run the Release workflow to publish core/fs,
then release `ms-package`, then push this repository's version tag to publish the
CLI/WASM packages and create the GitHub Release. Subsequent version tags use the
normal release pipeline. Already published versions are skipped by the publisher.
