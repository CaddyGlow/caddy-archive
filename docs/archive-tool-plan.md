# Portable archive and Windows package tooling plan

Status: proposed implementation plan, 2026-10-05. This document does not claim that the new crates or formats are implemented.

## Objective

Build a reusable Rust archive library and a native command-line tool with listing, extraction, creation, and integrity testing. The library must also run in browser WASM without requiring a host filesystem, subprocesses, or threads. Add read-only Windows package support for listing, validation, and payload extraction using the archive infrastructure. APPX, MSIX, and MSI creation, modification, repacking, and signing are out of scope.

Requested scope: ZIP and 7z with password encryption; CAB; TAR and compressed TAR; ISO/UDF; WIM/ESD; APPX/MSIX and MSI listing, validation, and extraction only. Support is declared by specific capabilities and tested format variants, rather than by file extension alone.

## Existing foundations and gaps

| Component | Reuse | Work still required |
| --- | --- | --- |
| `ms-compress` | Microsoft codecs, DEFLATE/zlib/gzip, LZMA1/LZMA2, filters and checksums; `no_std` plus `alloc` | Archive containers, cryptography, resource-budget integration |
| `cabinet` | CAB reading/writing, stored/MSZIP/LZX/Quantum | Portable I/O audit, adapter, extraction policy |
| `wim-format`, `wim` | WIM container and image operations, solid resource foundations | ESD fixtures, portable adapter, WASM dependency audit |
| `libmkiso` | Existing UDF 1.02 writer with ISO9660 boot discovery | General ISO9660/UDF reader; broader image creation profile |
| Windows trust code | Potential signature primitives and validation infrastructure | Review suitability for optional signature verification and WASM |

`wim-format` currently depends on `wim-memory`; removing that dependency from `ms-compress` did not remove it from WIM. Keep WIM optional until its ownership, allocator, and target assumptions have been audited. The current optical writer is not a general ISO reader.

## Crate boundaries

Provisional local names; check registry availability before publishing.

- `archive-core`: format implementations and common metadata, errors, I/O contracts, limits, codec configuration, and optional encryption. Start with portable `std`; preserve interfaces that allow a later `no_std + alloc` subset. WASM support does not itself require `no_std`.
- `archive-fs`: native filesystem enumeration, extraction, permissions, cancellation, temporary outputs, and atomic publication. Depend on `archive-core`.
- `archive-cli`: native binary, provisionally `arc`; commands and human/JSON output. Depend on the library and filesystem adapter.
- `ms-package`: read-only APPX/MSIX and MSI parsers, manifests and database models, package validation, and payload extraction. Reuse ZIP and CAB readers through library APIs; expose no package writer or signer.
- `archive-wasm`: small `wasm-bindgen` facade over portable APIs; typed browser-facing entry metadata, incremental operations, progress, cancellation, and randomness integration. No native dependency by default.

Do not make Windows package dependencies mandatory for ordinary ZIP/TAR callers. Feature-gate format backends and crypto. Keep `ms-compress` responsible for codecs; archive headers and package policy belong above it. Prefer maintained crypto primitives over new AES, hash, or key-derivation implementations. Audit licenses and target support before choosing dependencies or vendored code.

## Common API and I/O design

1. Probe format signatures with bounded reads; treat extensions as hints. Report detected format separately from requested interpretation.
2. Define byte-oriented read/write/seek traits usable by slices, buffers, files, and caller-provided range sources. Provide `std::io` adapters; avoid a compulsory async runtime.
3. Separate sequential readers from indexed readers. TAR can enumerate sequentially; ZIP needs central-directory access for a complete index; 7z solid folders share decoding work. Expose those differences instead of buffering everything implicitly.
4. Expose `EntryId`, raw stored name, display name, entry kind, sizes, timestamps, attributes, compression, encryption state, and format-specific metadata. Preserve non-UTF-8 names where the format permits them.
5. Provide list/index, stream an entry, extract to a caller sink, test integrity, and create APIs. Require an explicit maximum for conveniences that return whole entries as `Vec<u8>`.
6. Add container selectors: WIM image index/name, archive volume, and supported filesystem view for optical images. A WIM image is not an ordinary directory entry.
7. Publish a capability query for each backend: read/write, seek requirements, encryption, solid decoding, volumes, links, and target availability. Unsupported codec/filter combinations produce structured errors.
8. Use `u64` file sizes and offsets, checked arithmetic, and explicit conversion failures on 32-bit WASM. WASM users can process large files through bounded range reads without allocating the archive size.
9. Define structured errors for malformed input, unsupported feature, password required, integrity failure, resource limit, cancellation, and I/O. Do not promise to distinguish wrong passwords from all possible corruption cases.

## Resource and extraction policy

Configure entry count, filename/metadata bytes, nesting depth, dictionary size, per-entry and total decoded bytes, concurrent workspaces, pending output, and password-derivation work. Enforce actual bytes consumed and produced, not just declared sizes. Report which budget was exceeded.

The native extractor rejects absolute paths, parent traversal, drive/UNC paths, and collisions caused by destination normalization. Define duplicate-entry handling explicitly. Prevent links or concurrently changed destination directories from redirecting writes outside the extraction root. Links and special files require explicit policy; regular-file extraction is the initial default. Use platform-appropriate directory-handle operations where necessary.

Do not publish partially verified output as a successful extraction. Write temporary native outputs and commit after required checks. Streaming clients receive completion/integrity status separately from provisional bytes. Clean up operation-owned temporary files on failure while preserving user inputs.

## Format roadmap

| Format | Initial supported profile | Follow-up |
| --- | --- | --- |
| CAB | Existing supported read/write compression profiles | Spanning behavior through adapter; interoperability matrix |
| TAR | USTAR and PAX, regular files/directories | GNU variants, sparse files, links under explicit policy |
| gzip / zlib / `.lzma` | Single-stream operations | Gzip concatenation and header metadata profiles |
| `.tar.gz`, `.tgz` | TAR over gzip | Streaming pipeline and concatenation rules |
| ZIP | Stored/DEFLATE, ZIP64, descriptors, UTF-8 names | AES and legacy encryption; method 14 LZMA after format-specific validation |
| 7z | Copy/LZMA/LZMA2, checksums, bounded folder graphs, parallel extraction of independent folders | Solid archives, supported BCJ/delta pipelines, AES, header encryption, eligible independent LZMA2 blocks |
| XZ / `.tar.xz` | New XZ wrapper over LZMA2, supported checks and index validation | Multiple blocks/streams and supported filter chains |
| WIM/ESD | Image listing, selected-image file access, extraction | Creation/export, split/pipable/solid profiles after independent validation |
| ISO/UDF | ISO9660 baseline plus explicitly supported UDF revisions | Joliet, Rock Ridge, El Torito; broader creation profiles |
| APPX/MSIX | Package listing, manifest/block-map validation, payload extraction | Bundle inspection/extraction, optional signature verification |
| MSI | Read-only compound storage and Installer database, embedded CAB extraction | External media resolution, more metadata/table profiles |

Do not claim universal support for these formats. Deflate64 is not DEFLATE; bzip2, Zstandard, LZ4, PPMd, and RAR codecs are not supplied by current codec work. Additional inexpensive containers could include CPIO and Unix `ar`; `.deb` then requires explicit package conventions over `ar` and TAR. JAR and other ZIP-based formats can be opened as ZIP without claiming their application-level semantics. LZIP requires its own wrapper. Nonstandard combinations such as TAR plus LZMS are not interoperability targets.

## Multithreaded 7z extraction

Provide an optional native `parallel` feature and an extraction worker limit. Schedule independent 7z folders concurrently when the input supports independent range reads. Within an eligible LZMA2 stream, reuse independent dictionary-reset block decoding only after proving that the entire coder/filter pipeline permits it. A dictionary reset alone does not establish that encryption, filters, or other coders can be split safely. Single dependent solid streams and LZMA1 streams retain sequential decoding; listing several files inside one solid folder does not make those files independent decode jobs.

Build an extraction plan before dispatch: identify selected entries, shared folders, required predecessor output, coder dependencies, and estimated workspace costs. Decode each required solid folder once and route its selected file ranges to their sinks. Avoid nested worker pools by sharing one operation-level concurrency budget across folder and block work. If input access or the supported pipeline prevents parallelism, use the sequential path and expose the reason in operation metadata.

Bound total active decoder workspaces, queued compressed data, pending decoded data, and temporary output across all workers. A per-worker memory limit alone is insufficient. Preserve each entry's byte order and the requested metadata listing order; extraction completion order may differ. Aggregate errors and progress safely, cancel queued work on failure, join workers, and publish each output only after its required verification succeeds. Report verified completion separately from bytes decoded.

Gate: identical extracted bytes and integrity outcomes with one, two, and four workers, covering independent folders, solid folders, eligible LZMA2 blocks, filter pipelines, encrypted archives, unsupported parallel cases, cancellation, early drop, and worker failures. Benchmark wall time and peak memory against the sequential path; report actual parallel tasks rather than claiming a speedup merely because multiple workers were requested.

## Encryption

### ZIP

Implement WinZip AES AE-1/AE-2 reading and AE-2 writing, including AES-128/192/256 where needed for interoperability; default creation to AES-256. Follow specified salts, password derivation, verifier, authentication code, extra fields, and compression-method representation. Test stored and DEFLATE entries, ZIP64, and descriptors with encryption. Legacy ZipCrypto is an explicitly selected compatibility mode, not the default. Ordinary WinZip AES does not hide ZIP filenames.

### 7z

Implement the format's AES-256 pipeline, password encoding, key derivation, salt/IV properties, and encrypted-header handling. Bound attacker-controlled derivation work before invoking crypto. Support both encrypted payloads with visible headers and encrypted headers. 7z encryption does not provide the same authentication construction as ZIP AES; expose the actual checksum/integrity guarantees without calling them interchangeable.

### Shared interfaces

Inject a cryptographically secure randomness provider for creation. Native defaults use operating-system randomness; the browser adapter uses Web Crypto. Never derive salts/IVs from deterministic build seeds or reuse them for reproducible encrypted output. Keep passwords out of debug output, JSON diagnostics, command history examples, and logs. Native CLI password input uses a terminal prompt or explicitly selected input source rather than a normal argument. Clear sensitive temporary buffers where supported and document the limits of clearing browser strings.

Use known-answer fixtures, independent-tool interoperability, wrong-password tests, corrupted authentication tags/headers, truncation, and resource-exhaustion cases. Verify required authentication before final successful publication.

## WASM execution

- Baseline target: browser `wasm32-unknown-unknown`, single-threaded, no host filesystem or subprocesses. Optional WASI checks are separate from browser support.
- Offer in-memory APIs for small archives and incremental/range APIs for large files. Bridge browser `File`/`Blob` slices through asynchronous orchestration rather than blocking browser callbacks inside synchronous Rust reads.
- Run substantial work in a Web Worker. Bound each operation step so progress and cancellation can be serviced; yielding only between entire entries is insufficient for one huge entry.
- Keep thread-backed LZMA2 decoding native and optional. Browser threads are a later opt-in profile requiring shared-memory deployment support; sequential decoding must remain functional.
- Avoid holding duplicate JS and Rust copies of whole archives. Document WASM memory limits, copies, and whether a given adapter requires seeking.
- APPX/MSIX/MSI readers must not require Windows APIs. Optional signature verification and host trust-store integration are separate capabilities; package signing and installation are out of scope.

## APPX/MSIX extraction

Implement a read-only package profile over ZIP. List payloads and package metadata; parse manifests and content types; validate declared block-map hashes against decoded payloads. Report package integrity separately from signature verification and publisher trust. Do not treat successful ZIP extraction as complete package validation.

Support single packages first, then bundle manifests and explicit nested-package selection by architecture/resource identity. Preserve supplied manifest bytes and namespaces. Extraction must not alter, rebuild, or sign the source package. Encrypted APPX/MSIX variants, upload containers, and sparse packages require separate supported-profile decisions; ZIP/7z password encryption is not assumed to decode package-specific encryption.

Acceptance: files and metadata match independent Microsoft tooling; block-map mismatches and malformed manifests fail the requested validation operation; browser and native readers extract the same payload bytes. Package installation, deployment, update, creation, and signing are not completion gates for this read-only scope.

## MSI extraction

Implement or adopt a portable read-only Compound File Binary backend, then parse MSI stream naming, string pools, table schemas, typed rows, and summary information. Expose raw streams separately from logical installed-file payloads.

Resolve File, Directory, Component, and Media relationships to reconstruct declared payload paths and cabinet sequence mappings. Decode embedded CABs through `cabinet`. Accept external cabinets or loose source files only through an explicit caller-provided resolver; never search arbitrary host paths automatically. Report missing media and unsupported profiles explicitly.

Validate bounded sector chains, directory traversal, stream sizes, database references, file ordering, cabinet membership, and declared payload sizes. Do not execute custom actions, install files through Windows Installer, apply transforms/patches implicitly, or simulate installer conditions. Declared destination paths are metadata, not proof of the final installed layout; extraction uses the common safe destination policy.

Acceptance: independent MSI readers and Windows tooling agree on tables, stream bytes, and extracted payloads. Fixtures cover embedded/external media, long/short names, multiple cabinets, missing media, and malformed storage/database structures. Browser and native extraction must agree. MSI authoring, repair, uninstall, rollback, and upgrade behavior are outside this plan.

## CLI

Proposed commands:

```text
arc list archive --json
arc extract archive --output directory
arc extract archive.7z --output directory --threads 4
arc test archive
arc create --format zip --input directory --output archive.zip
arc create --format 7z --encrypt --encrypt-headers --output archive.7z --input directory
arc list install.esd --image 1
arc list app.msix --json
arc test app.msix
arc extract app.msix --output app-files
arc list app.msi --json
arc extract app.msi --output installer-files
```

Reject creation requests for APPX, MSIX, and MSI with an explicit unsupported-operation error. Define stable exit codes and machine-readable schemas. Support stdin/stdout only when the selected operation can stream, otherwise explain the seek requirement. Report progress and cancellation without polluting JSON output. Start creation from new archives; defer in-place update and split-volume writing until transactional behavior is specified.

### Optional indicatif progress bars

Add `progress = ["dep:indicatif"]` to `archive-cli`, with `indicatif` as an optional dependency. Keep it out of `archive-core` and the WASM dependency graph. Feature-disabled builds retain all archive operations and require neither terminal rendering nor indicatif. Keep `progress` and `parallel` independent; disabling bars must not disable multithreaded extraction.

The library emits renderer-independent progress events or snapshots through an optional observer: operation stage, bytes read/decoded/written, entries completed, verification status, and totals when known. Use operation-level counters to avoid counting solid-folder output or shared reads once per selected file. Specify whether counters measure physical reads, decoded work, or selected payload; these quantities must not be combined into a misleading percentage. Unknown totals use a spinner or byte counter. Progress callbacks must not run while internal locks are held and must not expose passwords or sensitive crypto material.

Progress must have near-zero impact on codec throughput. Select the reporting/no-reporting path once per operation, outside codec hot loops. The disabled path performs no progress-related allocations, clock reads, atomic updates, locking, channel sends, or callbacks. Prefer a statically specialized no-op observer where practical; verify generated behavior through benchmarks rather than relying on inlining assumptions.

When enabled, accumulate counters locally per worker and update only at existing coarse I/O/chunk boundaries. Never instrument individual bytes, symbols, or matches. Publish batched snapshots to a bounded/coalescing mechanism; intermediate snapshots may be replaced, and slow renderers must not block decoding or create an unbounded event queue. A separate consumer samples progress at a configurable cadence, initially 5–10 Hz, using clock reads outside decoder loops. Avoid per-update formatting, heap allocation, and cross-worker contention. Final counters and terminal status must be delivered exactly even when intermediate snapshots are coalesced. Cancellation remains an independent mechanism and must not depend on rendering frequency.

Define the observer contract as fast and nonblocking; document that arbitrary synchronous user callbacks can add their own overhead. The CLI and browser adapters consume batched snapshots outside codec execution. Choose batch sizes using measurements, balancing throughput against progress latency, including operations too short to need intermediate updates.

The CLI adapter uses indicatif for an aggregate bar and, where useful, bounded per-task bars. Throttle/coalesce worker updates so rendering does not dominate extraction. Render to stderr with `--progress auto|always|never`; `auto` enables bars only for an interactive terminal and disables them for JSON mode. Preserve clean stdout and stable JSON results. Complete or abandon bars correctly on success, error, and cancellation. When the feature is absent, `auto`/`never` remain usable and `always` produces a clear unavailable-feature error.

Gate: builds with and without `progress`, and with each `progress`/`parallel` combination; fake-observer counter tests; terminal/non-terminal and JSON behavior; failure/cancellation cleanup; no indicatif dependency in the browser build. Browser callers render the same progress information using their own UI. Benchmark no reporting, compiled-but-disabled reporting, enabled snapshot reporting, and actual terminal rendering on identical inputs/settings. Target no measurable disabled-path regression and at most 1% median throughput overhead for enabled snapshot reporting on sustained workloads; these are acceptance targets, not existing results. Report measurement variance, short-operation latency, update cadence, and native/WASM results separately. Investigate reproducible regressions rather than hiding them within noise; report terminal rendering overhead separately from library reporting.

## Implementation phases and completion gates

### Phase 0: design and dependency audit

Inventory actual backend capabilities, licensing, WASM build failures, unsafe/FFI paths, format specifications, crypto candidates, and reference-tool availability. Record supported profiles in a capability document. Agree on I/O, budget, metadata, error, and cancellation contracts. Gate: a tiny browser library harness reads supplied bytes with no filesystem or subprocess dependency.

### Phase 1: first end-to-end release

Create crates and native CLI; implement portable adapters, CAB integration, TAR/PAX, gzip pipelines, and ZIP stored/DEFLATE with ZIP64. Add extraction safeguards and actual decoded-byte budgets immediately. Gate: library and CLI list/create/test/extract their declared profiles; browser Worker does the same on byte inputs; independent readers validate output.

### Phase 2: encrypted ZIP

Add crypto/rand interfaces, WinZip AES and explicit ZipCrypto compatibility. Gate: bidirectional independent interoperability, password/corruption cases, no successful publication on failed authentication, browser encryption round trip.

### Phase 3: 7z

Implement bounded header/folder parsing and simple Copy/LZMA/LZMA2 archives with independent-folder parallel extraction, then solid folders and supported filters, then AES and encrypted headers. Add intra-stream LZMA2 parallelism only for validated pipeline profiles. Gate each profile independently against 7-Zip and compare one/two/four-worker extraction. Benchmark solid versus independent-block extraction, time to first byte, and total memory. Enforce the shared worker/memory budget and test sequential fallback.

### Phase 4: XZ and compressed TAR expansion

Implement XZ framing, checks, indexes, filter profiles, multi-block and concatenation rules. Gate: interoperability with `xz` and `tar`, malformed index/check failures, bounded streaming. Add further codecs only through a separately scoped decision.

### Phase 5: WIM/ESD and optical images

Audit and adapt WIM ownership/I/O for native use and then browser eligibility. Validate real LZMS solid ESD samples and supported image selection. Implement ISO9660 and UDF readers separately from the current writer. Gate: independent listing/extraction comparisons; unsupported revisions and variants fail explicitly. Mark native-only backends honestly until browser gates pass.

### Phase 6: APPX/MSIX

Build read-only manifest/content-type/block-map parsing, listing, payload extraction, and integrity validation; then add bundle inspection/extraction. Gate: independent tooling comparisons and matching browser/native extraction, with corruption regressions. No package writing or signing APIs.

### Phase 7: MSI

Build read-only compound storage and Installer database layers, then file/media mapping and embedded/external CAB extraction. Gate: independent table/stream/payload comparisons, missing-media handling, malformed-input tests, and browser/native parity. No installer authoring or execution.

### Phase 8: release hardening

Publish capability and license matrices, limits and compatibility documentation, fuzz corpora, benchmark results, browser examples, and reproducible unencrypted-build rules. Review remaining unsafe code and crypto integrations. Only advertise variants that passed their gates.

Phases 5–7 can be scheduled independently after their prerequisites, but broad format promises must not delay a usable Phase 1 release. No calendar estimates until Phase 0 reveals backend portability and database parsing and package-integrity work.

## Testing, fuzzing and performance

- Unit/regression tests for checked arithmetic, endian fields, descriptors, codecs, graph validation, path handling, block maps, schemas, and resource limits. Add property tests where they provide independent invariants.
- Bidirectional fixture interoperability for writable archive formats with 7-Zip, ZIP tools, GNU/BSD TAR, `xz`, CAB tools, wimlib, and optical readers. Read-only package interoperability compares APPX/MSIX/MSI metadata and extracted payloads with independent readers and Microsoft tooling; optional signature verification uses signed reference fixtures. Record exact versions, licenses/provenance, hashes, and tested profiles.
- Fuzz format probing, each parser, index/graph validation, entry decoding, crypto property parsing, package metadata, and compound-storage/database parsing. Separate fast parsing targets from expensive password derivation. Seed with encrypted archives, solid folders, ZIP64, multi-block XZ, PAX, optical images, ESD resources, and minimal packages.
- Exercise arbitrary truncation, inconsistent lengths, cyclic graphs, overlapping resources, invalid names, duplicate paths, decompression bombs, oversized dictionaries, and cancellation/early drop. Differential testing compares independent outputs without treating an implementation's acceptance as proof of validity.
- Browser tests execute real WASM in a headless browser: archive list/create/extract and read-only package list/validate/extract, Web Crypto randomness, encrypted round trips, Worker cancellation, bounded copies, and no native imports. Compile checks alone are insufficient.
- CI: formatting; required workspace Clippy with warnings denied; feature combinations; native tests; WASM compilation and browser execution; optional no-std codec checks; scheduled fuzzing; platform-specific package gates.
- Benchmarks cover throughput, archive size, time to first entry/output, peak memory, range-read count, solid random access, encryption overhead, and worker scaling. Compare CPU optimizations with identical inputs/settings. Report host capabilities and do not infer acceleration from feature flags alone.

## Specification references

- [PKWARE ZIP APPNOTE](https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT)
- [WinZip AES specification](https://www.winzip.com/en/support/aes-encryption/)
- [7z format overview](https://www.7-zip.org/7z.html); use the upstream implementation/specification files for detailed coder properties during implementation.
- [XZ file format specification](https://tukaani.org/xz/xz-file-format.txt)
- [Microsoft MakeAppx packaging and encryption](https://learn.microsoft.com/en-us/windows/msix/package/create-app-package-with-makeappx-tool)
- [MSIX block-map update behavior](https://learn.microsoft.com/ka-ge/windows/msix/app-package-updates)
- [MSIX signing overview](https://learn.microsoft.com/en-au/windows/msix/package/signing-package-overview)
- [Windows Installer installation package](https://learn.microsoft.com/en-us/windows/win32/msi/installation-package)

Before implementing each profile, pin its detailed normative specifications and add them to the capability/evidence record. Overview documentation alone is insufficient for binary serialization or cryptography.
