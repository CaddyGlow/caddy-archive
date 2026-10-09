# archive-rs

Reusable archive libraries and tools, extracted from windows-uup.

- `archive-core`: bounded archive parsing, creation, and verification.
- `archive-fs`: filesystem extraction adapter.
- `archive-cli`: the `arc` command-line tool.
- `archive-wasm`: browser/Web Worker interface.
- Sibling `cabinet` repository: CAB reader/writer and optional makecab/cabextract tools.
- `../ms-package`: standalone `ms-package` crate for portable APPX/MSIX
  and MSI inspection, including its patched MSI reader.

Published dependencies resolve from crates.io; sibling checkouts are optional
for development of those dependencies. Compression, WIM, and the libmkiso library
come from their own repositories.
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
[capabilities](docs/archive-capabilities.md), [CAB](https://github.com/CaddyGlow/ms-cabinet), and
[browser validation](crates/archive-wasm/README.md). Host tests do not establish
Windows filesystem behavior, bootability, or native installation correctness.
Retained evidence describes its original runs. Build caches and historical
campaign artifacts remain at their original windows-uup locations.

The [options/update implementation](docs/archive-options-implementation.md)
records the current subset of the [7-Zip compatibility and editing plan](docs/archive-options-update-plan.md).
`arc capabilities --json` reports the source-linked inventory, build capabilities,
and supported editing profile. ZIP rename/delete and ZIP/7z timestamp and encryption
edits support dry runs and guarded Unix publication. Browser Worker editing returns
a new byte artifact. 7z can encrypt filenames; ZIP supports per-entry passwords.

CI uses published crates.io dependencies and retained local regression fixtures.
Fixture provenance and reproduction instructions are stored alongside the samples;
source releases can be built and tested without sibling checkouts.

## Publication

The crates.io packages are `caddy-archive-core`, `caddy-archive-fs`,
`caddy-archive-cli`, and `caddy-archive-wasm`. Rust library names remain
`archive_core`, `archive_fs`, and `archive_wasm`; the CLI remains `arc`.
The repository is https://github.com/CaddyGlow/caddy-archive.
Push a tag matching the workspace version to run validation and platform release
tests. The workflow publishes core/fs in dependency order, verifies standalone
source and crate packages, then publishes CLI/WASM and creates the GitHub Release.
Already published versions are skipped. Manual Release dispatch remains available
to bootstrap core/fs independently.

MSI media integration uses published `ms-package` 0.2.2 with `caddy-msi` 0.10.2.
The CLI and Worker accept explicit media bytes/paths for embedded, external,
loose, partitioned and mixed layouts. Published package types use the separately
aliased registry `caddy-archive-core` 0.2.1; workspace archive types keep their
path identity. See the [migration record](docs/msi-media-refactoring-plan.md).
