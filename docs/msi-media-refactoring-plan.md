# archive-rs MSI media backend migration plan

Status: implemented consumer integration for published ms-package 0.2.2 and published archive-rs 0.3.0. The 0.3.0 workspace version boundary resolved the equal-version dependency alias publication conflict. Version 0.3.1 repairs release qualification by adding reproducible media fixture checksums and preserving sidecar bytes in Windows autocrlf checkouts. ms-package 0.2.1 updated the package-core dependency but did not contain the media migration.

## Scope

Consume the compatible ms-package wrappers backed by caddy-msi media primitives.
Keep archive readers, CLI explicit media input and WASM caller-media behavior
stable. MSI database/media semantics belong in caddy-msi; cabinet algorithms stay
in ms-cabinet. Do not introduce installer execution or host media discovery.

## Work batches

1. Record current package reader/CLI/WASM behavior and dependency aliases. Audit
   workspace and standalone fuzz manifests for package version/core compatibility;
   the existing package-core alias must resolve to the registry core compatible
   with the selected ms-package release.
2. After backend publication and package wrapper qualification, update registry
   package requirements and affected lockfiles together. Keep archive-core and
   package-core identities intentional; do not replace both aliases with a local
   workspace patch that creates duplicate-name dependency conflicts.
3. Preserve CallerMedia/MediaResolver bridges, explicit CLI media options and
   ByteInstaller signatures. Add adapters only where necessary for unchanged
   package APIs. Test bounded media reads and explicit missing-media errors.
4. Extend tests around embedded, external, loose, partitioned and mixed media.
   Keep create/edit native/Worker parity and full sidecar artifact comparison,
   including budget and failure behavior. Preserve existing fixtures/evidence.
5. Coordinate source-release and CI dependency refs with ms-package. Development
   reconciliation modifies isolated integration copies; published dependencies
   resolve from crates.io. Record the exact qualified Git revisions and hashes.

## Required gates

Run rustfmt; cargo test --workspace --all-features --locked;
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings;
and cargo test --manifest-path fuzz/Cargo.toml --locked. Exercise retained CLI
MSI media regressions and explicit source resolution behavior.

Build archive-wasm with packages for wasm32-unknown-unknown and run both baseline
and authoring browser Worker suites. Require payload and native/browser artifact
parity, media limits, missing-media errors and existing Blob cancellation behavior.
WASM compilation alone does not qualify browser behavior. Match wasm-bindgen
bindings and CLI versions. Preserve unrelated format/encryption regressions.

## Completion

Compatible published dependency versions are locked, public CLI/WASM contracts
are unchanged, media integration and baseline Worker gates pass, and reproducible
source bundles use the same qualified revisions. Native MSI install/repair/
uninstall qualification remains coordinated with ms-package; browser extraction
never establishes installation, signing or trust.

## Qualified dependencies and source provenance

The root, standalone fuzz, and isolated browser-authoring lockfiles resolve the
same published package versions. `.github/release-dependencies.json` records
full Git commits and registry archive SHA-256 values; CI validates all three
locks. Cached `.crate` bytes and embedded `.cargo_vcs_info.json` were checked
against that record on 2026-10-09. No unpublished dependency is required.

| Published package | Qualified Git commit |
| --- | --- |
| ms-package 0.2.2 | b05c857887f01ecc51d820ffdfb7ade5f2b28d83 |
| caddy-msi 0.10.2 | 1d9a8fe80766ed81136ab2b8c3b0574f59784d83 |
| ms-cabinet 0.1.4 | 6eaa28d396810bfe8183ad2f6c79b519daee88b3 |
| ms-compress 0.1.2 | b50f1264985d1db62b09beab905269ce5827c02c |
| registry caddy-archive-core 0.2.1 | cb2394c1bc5faf518a28b6edaa739c11e816cb2d |

The registry package-core bridge and local workspace archive-core deliberately
retain separate package identities. Use `cargo test --manifest-path
crates/archive-core/Cargo.toml --all-features --locked` to select the workspace
crate; a name/version selector is ambiguous. Publishing also selects each
workspace manifest explicitly.

Upstream qualification at the ms-package commit above is preserved in its
`docs/msi-media-refactoring-validation-20261009.md` and evidence directory.
It covers 63 identical artifact comparisons and Windows lifecycle checks for
the qualified producer. Archive consumer tests add explicit external, loose,
partitioned, mixed, missing, truncated and budget-limited media behavior.
Baseline and authoring Chromium Worker suites passed against published 0.2.2;
the authoring suite compares all native/browser sidecar artifacts and reopens
outputs through the downstream reader. This does not add a production browser
authoring API or establish installer signing or trust.

CI checks out the qualified ms-package commit for its browser-authoring test
crate, then runs `scripts/check-archive-authoring-browser.sh` with the source
crate and archive bindings paths. The runner verifies Git HEAD and clean
tracked source files, uses an isolated copy, and builds published dependencies
with the checked `crates/archive-wasm/tests/authoring-Cargo.lock`. Historical
fixtures and upstream evidence remain unchanged.

## Reproducible source release

`scripts/source-release.py --output /tmp/archive-source.tar.gz` requires a clean
checkout and packages its immutable Git HEAD. `--allow-dirty` explicitly marks a
local preview and includes untracked, nonignored integration evidence. The
bundle retains licenses, fixtures and the authoring lock; its source manifest
records every file hash, all lock hashes and the qualified dependency record.
Tar ownership, ordering, modes and gzip timestamps are normalized.

`scripts/source-release.py --verify /tmp/archive-source.tar.gz --build` verifies
extracted file hashes and dependency provenance, then runs locked workspace and
fuzz tests from the extracted sources. Release CI performs this source rebuild.
The bundle does not vendor crates or the separate upstream authoring test
checkout: registry/cache access and the pinned checkout remain required. It
makes no offline or browser-authoring rebuild claim.

Published `.crate` provenance is authoritative. In particular, the upstream
source dependency record references a later ms-compress commit than the
published 0.1.2 archive; this consumer records the archive's embedded commit
b50f1264985d1db62b09beab905269ce5827c02c and never substitutes that source tree.

## Publication version and package qualification

The user authorized a coordinated workspace release at 0.3.0. The package-core
bridge remains the exact published registry caddy-archive-core 0.2.1, while the
workspace crates and their path dependency requirements move to 0.3.0.
This minor-version boundary preserves distinct package identities after Cargo
normalizes path dependencies for publication.

The original workspace 0.2.1 package failed verification because both aliases
normalized to the same registry package under different names. An isolated
registry-source resolver check also rejected a prospective 0.2.2 workspace:
Cargo cannot select semver-compatible ^0.2.2 and =0.2.1 simultaneously. The
same check built ^0.3.0 and =0.2.1 successfully. A patch bump therefore would
not resolve the publication conflict.

All four normalized crate packages must pass Cargo package verification before
publication is considered qualified. Package creation with `--no-verify` alone
is insufficient. Release publication proceeds in dependency order: archive-core,
archive-fs, then archive-cli and archive-wasm. The retained package-core registry
checksum and qualified producer revisions do not change with this workspace
version bump. Final registry publication receipts are recorded by the release
run; workspace and source-build results alone do not establish publication.

Release 0.3.0 qualification passed full `cargo package --workspace --all-features
--locked` verification for all four crates, with the normalized registry aliases
remaining distinct. Required host gates, WASM build, baseline and authoring Worker
suites, dependency provenance checks and all 16 Python script tests also passed.
