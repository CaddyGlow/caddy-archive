# Archive Capabilities

This matrix describes implemented profiles, not universal format support.
Release evidence and remaining gates live in [archive-tool-status.md](archive-tool-status.md).
`capabilities(format)` is feature-sensitive. Its `links` flag means metadata
can be listed, not that links may be materialized. WIM/UDF use standalone
borrowed selectors rather than the generic `Archive::open` dispatch.

| Format / feature | Read profile | Write profile / exclusions |
| --- | --- | --- |
| ZIP / `zip` | Stored, DEFLATE, ZIP64, descriptors | DEFLATE ZIP64; no Deflate64 or strong/header encryption |
| ZIP / `crypto` | WinZip AES AE-1/AE-2 128/192/256, ZipCrypto | AES-256 AE-2 or explicitly selected legacy ZipCrypto |
| TAR / `tar` | USTAR, bounded PAX and GNU long names; link metadata | USTAR/PAX; no sparse/special extraction or link publication |
| gzip / `gzip` or `streams` | Concatenated members with trailer checks; first member header metadata | Deterministic named gzip; TAR wrapping with `gzip` |
| zlib, LZMA / `streams` | zlib checksum; explicit signatureless LZMA selection | Single stream; bounded dictionary/workspace |
| raw DEFLATE / `streams` | Explicit selector only; no framing checksum | Forward-only inflate/deflate or indexed single-entry wrapper |
| BZip2 / `bzip2` | Standard blocks and concatenated members, CRC checks; explicit TAR wrapper | Pure Rust libbz2-rs backend; level 9; TAR output streams |
| Brotli / `brotli` | Standard RFC 7932 window; explicit raw/TAR interpretation | Pure Rust rust-brotli, quality 5/window 22; no shared-dictionary or large-window extension claim |
| XZ / `xz` | Concatenated/multiblock; none/CRC32/CRC64/SHA256 checks; Delta and x86/PPC/IA64/ARM/Thumb/SPARC/ARM64/RISC-V BCJ | LZMA2 with CRC64, optional TAR wrapping; unsupported filter graphs rejected |
| CAB / `cab` | Stored, MSZIP, LZX, Quantum via cabinet | MSZIP; spanning requires a volume resolver and is rejected by this adapter |
| 7z / `sevenz` | Direct archive-core parser and ms-compress codec graph; Copy/LZMA/LZMA2, supported filters and AES under `crypto` | Direct writer; independent-folder parallelism under `parallel`; no universal coder/graph claim |
| ISO9660 / `iso` | Portable libmkiso parser adapter | Read only; supported ISO profile only, not a general optical filesystem implementation |
| UDF / `udf` | Shared libmkiso UDF 1.02–2.60 physical, metadata, VAT and sparable maps; short/long/extended allocations, sparse files, bounded allocation/ICB/file-set chains, embedded data, preallocated tails and backup-anchor recovery | Archive adapter is read only; libmkiso has configurable native UDF authoring. Link metadata retained without following targets; named/system streams exposed separately through explicit stream APIs |
| WIM/ESD / `wim` | Standalone selected-image/resource adapter | Read only; native validation only, no browser support claim |
| APPX/MSIX/MSI | Separate read-only ms-package APIs | No package writer, signer, installation or custom-action execution |

Default features are `zip,tar,gzip,cab,iso,xz,streams`. Encryption, 7z,
parallelism, WIM, UDF, BZip2 and Brotli are opt-in. Disabled formats report unavailable;
`browser` is a portability capability, not evidence of runtime validation for
every profile. Raw names are retained where supplied by the container backend;
CAB names pass through cabinet's decoded strings. General timestamp/attribute
preservation is not implemented by `Entry`; `Archive::entry_metadata` now
provides optional ZIP and TAR stored-header fields, but not PAX timestamp
overrides or equivalent metadata for every backend. See the
[completion audit](archive-completion-audit.md) for remaining full-plan gates.

## Budgets and Streaming

`Limits` bounds input, entry count, metadata, per-entry/total decoded bytes,
dictionary/workspace, nesting depth, workers, pending output and password
iterations. Declared sizes and actual output are checked; callers must also
budget their own sinks and scheduling. Defaults permit large archives and are
not a substitute for application-specific limits. Codec memory estimates are
conservative, not exact process RSS accounting. KDF limits apply before costly
7z password derivation; WinZip PBKDF2 uses its fixed 1,000 iterations.

Generic indexed reading requires `Read + Seek`. `range::RangeReader` adapts
caller-owned synchronous range access without networking/runtime dependencies.
ZIP's `incremental::RangeIndex` and entry decoder bound cached metadata and
output chunks; this does not make all formats incremental range readers.
`SequentialTar` works with forward-only input. Indexed TAR.GZ/TAR.XZ retain a
bounded decoded TAR buffer. `read_entry` retains the whole selected payload;
streaming extraction does not. Creation consumes the caller's in-memory entry
payloads even when the output pipeline itself streams.
`inflate_stream` and `deflate_stream` instead accept forward-only `Read` and
`Write` for raw DEFLATE, gzip and zlib without an in-memory entry payload.
Raw DEFLATE and Brotli have no payload checksum/authentication; completion and
size validation are not equivalent to authenticated content.

Extraction bytes are provisional until final checks succeed. `verified` means
size and available checksums/authentication passed, not a signature or trusted
publisher. TAR/ISO size checks are not cryptographic verification. AES ZIP
authenticates ciphertext before emitting plaintext. Filesystem staging and
atomic publication are separate native policy.

## Licenses

The workspace wrapper is MIT; that does not relicense its dependencies.
`ms-compress` declares `LGPL-2.1-only AND LGPL-2.1-or-later AND Zlib AND
Apache-2.0`; retain its codec notices under `../ms-compress/src`, including
LZX, Quantum and LZMA notices. `cabinet` and `libmkiso` declare MIT.
Optional `wim-format` declares `LGPL-2.1-or-later OR GPL-3.0-or-later`.
Assess distribution/linking obligations for the actual feature graph.

ZIP metadata crate `zip` is MIT; `tar`, `serde`, `thiserror`, `crc64fast`, and
RustCrypto AES/CBC/CTR/PBKDF2/HMAC/SHA1/SHA2/zeroize declare MIT OR Apache-2.0
(zeroize lists the alternatives in reverse order). `subtle` is BSD-3-Clause.
This is a scoped manifest audit, not a complete transitive license report.
The adapted sevenz-rust2 test corpus retains its Apache-2.0 license and notice
under `crates/archive-core/tests/fixtures/sevenz-upstream`; it is not the
production container implementation. Consult the lockfile and retained notices
before redistributing binaries, WASM or fixtures.

The MSI parser uses a local MIT-licensed fork of `msi` 0.10.0 under
`../ms-package/vendor/msi`. Its required metadata-cell checks reject malformed schema rows
instead of panicking; upstream source provenance and local changes are retained
in `../ms-package/vendor/msi/LOCAL-CHANGES.md`. This fix does not establish that all malformed
MSI or compound-storage inputs are safe; fuzzing remains a separate gate.

Unencrypted reproducible commands are in [archive-core's README](../crates/archive-core/README.md).
Disabling `crypto` does not remove codec LGPL obligations.

BZip2 uses `bzip2` 0.6.1's default Rust `libbz2-rs-sys` backend, not its
optional C `bzip2-sys` backend. The wrapper declares MIT OR Apache-2.0; retain
the backend's `bzip2-1.0.6` license and notices. Brotli uses `brotli` 8
(`BSD-3-Clause AND MIT`) and its decompressor (`BSD-3-Clause/MIT`);
retain their license texts/notices. Specifications pinned
for these profiles are [bzip2 manual 1.0.8](https://sourceware.org/bzip2/manual/manual.html)
and [RFC 7932, July 2016](https://www.rfc-editor.org/info/rfc7932/).
RFC 9841 shared dictionaries are not included in the support claim.
With their corresponding features, 7z also reads/writes BZip2 and Brotli.
Native round trips and the real Chromium Worker cover all five selectable
writer codecs (Copy/LZMA/LZMA2/BZip2/Brotli), plus independent 7z BZip2/Brotli
fixtures. Native tests cover raw/TAR BZip2/Brotli limits/truncation and an
independent bzip2 reader; the Worker also executes these four profiles and raw
DEFLATE. These results do not establish support for arbitrary coder graphs.
