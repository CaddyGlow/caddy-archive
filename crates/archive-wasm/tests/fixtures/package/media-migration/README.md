# Synthetic MSI media migration fixtures

These unsigned, test-only packages contain three generated text payloads. They
exercise explicit caller media through published `ms-package 0.2.2` and
`caddy-msi 0.10.2`; no installer execution, signing or trust is implied.
Existing package fixtures remain unchanged.

`manifest.json` records all 15 complete MSI/sidecar byte lengths and SHA-256
hashes, declared file IDs, target paths, source names, sequences, cabinet
assignments and exact payload hex. Native CLI and browser Worker tests share
these artifacts. Profiles cover one embedded cabinet, one external cabinet,
loose source files, two external cabinet partitions, mixed embedded/external
cabinets, and mixed embedded/external/loose media.

The mixed-loose package uses explicit compressed attributes on its first two
files, an uncompressed attribute on the third, and an uncompressed SummaryInfo
default. Its declared `SourceMedia/third.txt` source name differs from the
`MediaFixture/third.txt` target path. The external cabinet retains its original
third member, which is not the declared source for the loose third file.

`generate.rs` builds with published authoring APIs, then reads every payload back
through the published reader and a bounded explicit resolver. The raw table edit
for mixed-loose also uses the published `InstallerEditor`. The production CLI
and WASM dependencies do not enable authoring to run these read-only tests.

Registry source provenance from each crate's `.cargo_vcs_info.json`:

| Crate | Version | Git revision |
| --- | --- | --- |
| ms-package | 0.2.2 | `b05c857887f01ecc51d820ffdfb7ade5f2b28d83` |
| caddy-msi | 0.10.2 | `1d9a8fe80766ed81136ab2b8c3b0574f59784d83` |
| ms-cabinet | 0.1.4 | `6eaa28d396810bfe8183ad2f6c79b519daee88b3` |

For a standalone generator crate, copy `generate.rs` to `src/main.rs` and
`generation-Cargo.lock` to `Cargo.lock`, then use this `Cargo.toml`:

```toml
[package]
name = "archive-msi-media-fixtures"
version = "0.0.0"
edition = "2024"

[dependencies]
ms-package = { version = "=0.2.2", features = ["write"] }
msi = { package = "caddy-msi", version = "=0.10.2", features = ["media"] }
serde_json = "1"
sha2 = "0.10"
```

From the archive-rs working directory, run its Nix shell with the standalone
manifest path and a separate output directory:

```sh
nix develop --no-write-lock-file . -c cargo run \
  --manifest-path /tmp/archive-msi-media-generator/Cargo.toml --locked \
  -- /tmp/archive-msi-media-regenerated
```

Two independent generator runs produced identical manifests and every complete
MSI/sidecar byte artifact. The retained lockfile records registry checksums and
all generator dependency versions; it is fixture provenance, not a workspace
lockfile or a production dependency override.
