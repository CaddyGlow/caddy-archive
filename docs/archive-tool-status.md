# Archive tool implementation evidence

The objective remains the complete scope of `archive-tool-plan.md`. This record
tracks implemented profiles and unpassed gates; adding a crate is not evidence
that a phase is complete.

## Architecture

Five workspace members separate portable archive operations, native filesystem
policy, CLI rendering, read-only package interpretation, and browser bindings:
`archive-core`, `archive-fs`, `archive-cli`, `archive-wasm`.
The `ms-package` crate is a dependency from `../ms-package`.
The filesystem and CLI crates must not enter the browser dependency graph.
Package APIs expose no writer, signer, installer, or custom-action execution.

## Existing dependency audit

| Component | License | Current observation |
| --- | --- | --- |
| ms-compress | See crate manifest and codec notices | Portable codec foundation; container policy stays in archive-core |
| cabinet | MIT and retained decoder notices | std Read/Seek CAB foundation; spanning and workspace limits need adapter review |
| wim-format | LGPL-2.1-or-later OR GPL-3.0-or-later | Optional std and portable allocation code; browser gate has not passed |
| libmkiso | See crate manifest | Portable ISO9660 reader; native UDF writer behind native-writer feature |
| wasm-bindgen | MIT OR Apache-2.0 | Browser ABI facade; compiling bindings alone does not prove browser execution |

The current worktree removes wim-memory through unrelated ongoing work. Audit
the current dependency graph rather than assuming the plan's historical
wim-memory dependency is still present.

## Specification Pins

Binary implementations must cite the version used, not just an overview page.

| Format | Reference |
| --- | --- |
| ZIP | PKWARE APPNOTE 6.3.10, revised 2022-11-01: https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT |
| WinZip AES | Encryption Specification AE-1 and AE-2, document 1.04, January 30, 2009: https://www.winzip.com/en/support/aes-encryption/ |
| XZ | XZ file format specification 1.2.1, 2024-04-08: https://tukaani.org/xz/xz-file-format.txt |
| ISO9660 | ECMA-119 5th edition, December 2024: https://ecma-international.org/wp-content/uploads/ECMA-119_5th_edition_december_2024.pdf |
| UDF writer foundation | ECMA-167 2nd edition, December 1994, as cited by libmkiso |

WinZip's AES specification was retrieved on 2026-10-05. AE-1 CRC handling,
AE-2 zero CRC, method 99, extra field 0x9901,
salt lengths and authentication framing were checked against version 1.04.

## Completion Gates

All eight phases remain subject to their gates in the plan. In particular,
encrypted ZIP/7z, XZ, optical readers, WIM/ESD selectors, package parity,
incremental browser range I/O and cancellation, independent interoperability,
parallel worker comparisons, fuzzing, and measured progress overhead require
explicit evidence. No broad format or phase completion claim follows from an
initial native round trip or a WASM compile check.

Reference tools observed locally during the audit: `7z` is available; standalone
`wasm-bindgen`, `xorriso`, and `genisoimage` commands were not found. The
`wasm32-unknown-unknown` Rust target is installed.

## Verified Development Evidence

Current implementation evidence, not a claim of complete phases:

- CLI `compress`/`decompress` round trips cover fourteen single-file codecs:
  DEFLATE/gzip/zlib, LZMA/LZMA2, XZ, BZip2/Brotli, XPRESS Huffman/plain,
  LZX, LZMS, LZNT1, and Quantum. Tests cover required raw output sizes,
  resource limits, short XPRESS blocks, magic precedence, binary stdout, and
  failure without publication. Raw formats have no integrity checksum; Windows
  codec files contain one block, with codec-specific size limits. CAB creation
  selects stored/MSZIP/LZX/Quantum, ZIP selects stored/DEFLATE (including both
  encryption modes), and 7z adds DEFLATE. Independent native 7z extraction passes
  for stored ZIP, DEFLATE 7z, and LZX/Quantum CAB; independently created DEFLATE
  7z extracts through arc. Affected archive/codec host suites and targeted Clippy
  pass, as do archive-core builds with no features, streams only, and 7z only.
  WIM creation and browser/runtime validation of these new paths remain unpassed.
  Workspace-wide Clippy is blocked by an unrelated unfulfilled lint expectation
  in wintrust's portable chain validation.

- CLI compact tar forms (`xzvf`, `czvf`, `xJvf`, `cjvf`, `cavf`, `tf`) are
  covered by host creation/list/extraction tests, including `-C` destinations
  and verbose names on stderr with JSON on stdout. Metadata round trips cover
  ZIP, 7z, TAR and compressed TAR, gzip modification time, CAB DOS modification
  time/read-only status, encrypted ZIP/7z, and forward-only TAR stdin. Directory
  times and permissions are restored after descendants. Independent 7z extraction
  of newly created ZIP and 7z archives preserves the tested timestamp and Unix
  mode. Archive-fs checks for x86_64-pc-windows-msvc; Windows runtime metadata
  behavior remains an unpassed gate. Timestamps currently have whole-second
  precision (DOS timestamps have two-second precision), and PAX overrides,
  ownership restoration, ACLs, extended attributes, and special permission bits
  remain outside this metadata path.

- CLI creation infers supported formats from output extensions, including
  compound TAR suffixes and case-insensitive aliases. Regression tests cover
  explicit-format precedence, unknown/stdout output errors, content detection
  despite misleading extensions, renamed WIM/MSI files, gzip/zlib/raw DEFLATE
  stream commands, and forward-only TAR stdin without a format option. BZip2
  content probing recognizes an inner TAR while an explicit BZip2 interpretation
  retains the raw stream. All seven format-detection tests pass on the host.

- Native review regressions assert the exact codec stored by all six unencrypted
  7z creation selectors, archive-wide total/entry/path-depth limits before sparse
  ZIP indexing returns ready, and 100-file ZIP and stdin TAR extraction under a
  64-descriptor process limit. Batched CLI extraction uses one anonymous spool
  on the destination filesystem and retains whole-batch verification before
  regular-file publication. Interleaved and empty outputs, incomplete-batch
  cleanup, and late integrity failure are covered by host tests. This evidence
  does not establish Windows filesystem behavior.

- Instrumented bounded archive/optical fuzz targets completed 10,001 iterations
  each without reported crashes/timeouts. The broadened package campaign found
  a SIGABRT after 935 iterations; the unchanged input now reproduces a checked
  error through the local msi 0.10.0 fork, with a retained regression test.
  The instrumented post-fix package campaign completed 10,001 iterations with
  zero reported crashes/timeouts, using the unchanged crash as a seed. Evidence
  is recorded in `fuzz/README.md`. Weekly/manual artifact-retaining CI is
  configured but has not run remotely. These seconds-long campaigns are not
  sustained fuzzing or sanitizer gates.

- Independent filtered/encrypted 7z worker comparisons at 1/2/4 pass, including
  solid fallback. Actual first-byte, elapsed-time and process-memory samples
  for 32 MiB corpora with four independent folders are recorded in
  [archive-parallel-measurements.md](archive-parallel-measurements.md); timing
  does not show a universal parallel speedup.

- Initial ZIP/TAR/TAR.GZ/CAB library round trips and corruption/budget regressions
  pass. ISO9660 parsing now belongs to renamed `libmkiso`; archive-core
  contains only its metadata/error adapter. Optical reader/writer native tests
  pass, including independent 7-Zip extraction. The over-4-GiB gate remains ignored.
- Borrowed WIM adapter image-selection and metadata-budget tests pass on the
  existing XPRESS resource fixture and an independently generated wimlib 1.14.4
  LZMS solid ESD fixture. Microsoft production ESD validation remains open.
- UDF 1.02 single-physical-partition short-allocation reader matches the existing
  writer's payloads and independent 7-Zip extraction. Other revisions, maps,
  allocation types, sparse files and links fail explicitly.
- RustCrypto ZIP AES-256 creation and AES-128/192/256 decoding, authentication
  failures and explicit ZipCrypto interoperability tests pass against 7-Zip.
- XZ and TAR.XZ tests include independent `xz` decoding, concatenated streams,
  multiblock/index/footer checks, CRC32/CRC64/SHA256 and x86/Delta fixtures.
  Broader BCJ interoperability and fuzzing remain release gates. Raw gzip/zlib/
  LZMA tests cover independent gzip/xz tools, concatenation and bounded headers;
  forward-only TAR tests cover PAX, bounded metadata and truncated payloads.
- Optional pure Rust BZip2/Brotli raw and TAR-wrapper tests pass, including
  low-workspace limits and truncation; independent `bzip2` decodes created data.
  Brotli decoding uses a shared capped allocator. Forward-only raw DEFLATE,
  gzip and zlib APIs have round-trip/output-limit regressions. These are narrow
  profile checks, not universal filter, framing or malicious-input coverage.
- Chromium executes the WASM in a real Worker: fifteen profiles including raw
  DEFLATE, BZip2/Brotli, their TAR wrappers and ZIP/TAR/TAR.GZ/CAB/7z/XZ round trips,
  allocation limits, AES fresh randomness/round trip/wrong password, generated
  MSIX and embedded MSI native payload parity, and corrupt block-map rejection.
  A 16 MiB ZIP Blob extracts in bounded chunks; cancellation stops after three
  delivered chunks. This is ZIP range I/O evidence, not a universal range reader.
  Direct 7z BZip2/Brotli fixture reads and all five selectable 7z writer codecs
  (Copy, LZMA, LZMA2, BZip2, Brotli) also execute in that Worker harness.
- Native filesystem regression checks cover traversal, symlink redirection,
  collisions, existing-file preservation, failed verification and cancellation.
  Seven Windows filesystem runtime tests pass on a disposable Windows 11 VM,
  including reparse rejection and pinned-parent rename denial.
- Thirty-seven compatibility/security cases adapted from sevenz-rust2 0.23.0 pass
  against the direct archive-core backend, including six CLI extraction cases.
  Independent 7-Zip read/write interoperability and parallel scheduler regressions
  also pass. The corpus and Apache-2.0 notice are retained under
  archive-core/tests/fixtures/sevenz-upstream. Direct container replacement is
  implemented without the upstream runtime dependency.
- Required workspace Clippy with all targets/features and warnings denied passed
  after the initial rename. Subsequent changes require another final run.
  The latest six archive/optical crate check and full-workspace all-target,
  all-feature check both pass. An earlier undeclared `archives` feature guard
  failure in windows-mpsp no longer occurs in the current worktree.
  Full workspace tests also encounter a SIGSEGV in wim's test_support_guard;
  neither failure is counted as passing archive release evidence.

Browser bindings were generated using wasm-bindgen-cli 0.2.129 installed under
`~/.cache/archive-wasm-tools`. The reproducible Worker harness and commands live
in `crates/archive-wasm/README.md`. Generated bindings and fixtures stay in `/tmp`.
