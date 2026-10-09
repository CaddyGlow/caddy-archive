# Options and editing implementation scope

This records the implemented subset of the
[options/update plan](archive-options-update-plan.md). It does not close all
P0–P8 milestones or assert full 7-Zip parity.

| Plan area | Implemented behavior | Remaining work |
| --- | --- | --- |
| P0 | Typed source-linked seed inventory, JSON capability query, reference executable/version/hash/configuration capture and baseline differential probes | Individual upstream property rows, resolved defaults and comprehensive differential acceptance/effect coverage |
| P1 | Bounded byte-name selection shared by native indexed list/test/extract; limited `arc 7z l/t/x`; native option collisions preserved | Full selector/listfile/recursion grammar, password prompts, effective typed options across formats |
| P2 | Automatic classic ZIP/ZIP64 headers, descriptors, individual central fields and end records; validated native streaming DEFLATE effort | Other codec tuning and upstream default resolution |
| P3 | Seven update states, five default action sets, repeated `-u` suffix parsing, anti-item gating; Unix provisional filesystem transactions | Integrated add/update plans, timestamp normalization, scratch providers and Windows replacement |
| P4 | ZIP packed-copy rename/delete, modification times, per-entry AES encryption/rekey/decryption, validated simultaneous dry runs and optional retained-payload verification | Add/replace/transcode and broader lossless ZIP profiles |
| P5–P7 | 7z raw metadata timestamp reconstruction, compressed-stream AES transforms and header encryption; unsupported profile gates | TAR/CAB reconstruction, broader 7z reconstruction and solid subset updates, missing upstream codecs/properties and WIM authoring/update |
| P8 | Bounded synchronous Worker ZIP rename/delete and ZIP/7z timestamp/encryption editing into a new artifact | Incremental browser editing/cancellation, remaining profiles, volumes/SFX and final platform/parity audit |

The canonical inventory lives in
[`compatibility.rs`](../crates/archive-core/src/compatibility.rs) and is serialized
by `arc capabilities --json`. Feature prerequisites and scoped implementation
statuses are distinct from pinned-reference verification. `reference_default:
null` means unresolved. CLI package inspection uses its separate read-only facade.

ZIP sizing reserves ZIP64 local fields at `0xF8000000` for compressed/encrypted
entries and at `0xFFFFFFFF` for unencrypted Stored entries. Final central fields
use their individual sentinels, including offset-only ZIP64. End records upgrade
for counts of 65,535 or size/offset sentinels. Sparse test I/O exercises the real
writer at the offset boundary without allocating a multi-GiB payload.

The editing profile requires single-disk, prefix-free, contiguous Copy/DEFLATE
or supported AES/ZipCrypto ZIPs with UTF-8 or ASCII names and known extras. Unknown extras,
Unicode-path extras, duplicate names, conflicting operations, unsupported flags,
signatures, padding and trailing data are rejected. Name edits concern archive
names; they never authorize filesystem extraction. Source metadata is rechecked
before execution, and native transactions retain handles and check source identity
through publication. Packed bytes are never interpreted as authenticated merely
because they were copied. Caller-owned core I/O requires stable input throughout.

`UpdateTransaction` uses retained no-follow Unix directory traversal, adjacent
provisional files, advisory parent locks, source identity checks and atomic rename
for replacement. All writers must cooperate; final check plus rename is not a
universal compare-and-swap. New-output publication is no-clobber. Windows is
explicitly gated. Browser byte editing enforces separate input, metadata, declared
decoded-byte and output budgets; its synchronous operation can only be abandoned
by terminating the Worker. It does not claim native publication or package validity.

Run the differential harness against a built CLI and the pinned reference:

```sh
python3 scripts/check-archive-compatibility.py \
  --arc /path/to/arc --reference /path/to/7zz \
  --edit-probes \
  --reference-source-commit 9128b80e3a471108678e2b3ec3c985861c8d7b0f \
  --report /tmp/archive-compatibility.json
python3 -m unittest discover -s scripts/tests
```

The source declaration is recorded separately from executable version verification.
Reports capture hashes, build configuration, generated fixture provenance,
command results and cross-extracted payload hashes. Baseline profiles cover ZIP
Copy/DEFLATE, 7z Copy/LZMA2 and TAR; compressed byte equality is not required.
`--edit-probes` adds exact/directory ZIP rename/delete comparisons, expected
payload hashes and archive-comment preservation. These cover the declared subset.
Missing references and version mismatches exit 2. An explicitly allowed mismatch
produces unpinned evidence, never pinned conformance. Fixture/campaign history and
separate Windows, sustained fuzzing and firmware gates remain preserved.

Validation on 2026-10-09 passed the required workspace all-feature tests,
all-target/all-feature Clippy with warnings denied, and fuzz-manifest host tests.
The WASM feature build and real Chromium Worker gate passed, including encrypted
ZIP packed-copy editing and its input/metadata/decoded/output limits. Ten harness
unit tests passed. Nine baseline/edit profiles passed against the pinned source
build of 7-Zip 26.04; encrypted rename interoperability also passed separately.

The reference was built from the plan's source commit using the local Nix shell,
`make -f makefile.gcc -j8 CC=clang CXX=clang++` in `CPP/7zip/Bundles/Alone2`,
with default multithreading and no assembly option. Downloaded source archive
SHA-256: `81cbf3d080dde52ab0ca019cee849d2da8cc1dcc8b1b2c260fd2f667f141f8a4`.
Reference executable SHA-256:
`bc0ab48d2c1471ba6c39c3ba3ddcbaa3ea0a63e396531a4f7340fb565f993733`.
The local JSON report is `/tmp/archive-compatibility-pinned-edits.json`; rerun the
harness to establish evidence for another CLI build. These results do not close
unimplemented milestones or the separate platform/fuzz/firmware gates.

The subsequent timestamp/encryption tranche adds `arc edit`, separately supplied
old/new password files, authoritative ZIP UT modification times with UTC-derived
DOS fallback, consistent NTFS modification updates, and checked 7z FILETIME edits.
Untouched times and ciphertext are retained. Transformed encrypted payloads are
verified before bounded decrypt/encrypt of compressed streams; no recompression
is needed. ZIP allows individual entry passwords and does not hide names. 7z
supports filename-encrypted headers and complete compression-group transforms;
solid subsets and partial rekeying with encrypted headers are rejected. Global
7z operations report entries without payload separately; explicitly selecting an
empty payload for encryption is unsupported. Native 7z discard preflight repeats
the bounded transform before any temporary artifact is created. Whole-archive
CLI decryption clears header encryption, while selected decryption preserves it.

Worker APIs provide matching bounded timestamp/encryption artifacts using Web
Crypto randomness and separate credential buffers, with owned WASM copies
zeroized on return. Browser inputs still need a JS-side bound before binding
copies. The prior reference report above covers the original name-edit tranche;
new feature evidence is recorded by the tests and follow-up validation run.

Final timestamp/encryption validation on 2026-10-09 passed 306 workspace tests
(with four separate existing/reference gates ignored in the aggregate run),
all-target/all-feature Clippy with warnings denied, fuzz-manifest host tests,
and rustfmt checks. The WASM feature build and real Chromium Worker suite passed
including modification times, AES rekey/decryption, hidden 7z names, wrong-password
rejection and edit budgets. Both newly ignored reference tests were run explicitly
against the pinned 26.04 executable: six ZIP Copy/DEFLATE encryption-state cases
and 7z solid timestamp/encrypt/header/decrypt cases passed independent listing,
test and extraction checks. Windows publication, sustained fuzzing and firmware
remain separate gates.

Reproduce the additional reference tests after setting the two executable paths:

```sh
ARCHIVE_REFERENCE_7ZIP=/path/to/pinned/7zz cargo test -p caddy-archive-core@0.2.1   --all-features --locked --test zip_edit pinned_7zip -- --ignored
ARCHIVE_7Z_REFERENCE=/path/to/pinned/7zz cargo test -p caddy-archive-core@0.2.1   --all-features --locked --test sevenz_edit pinned_reference -- --ignored
```
