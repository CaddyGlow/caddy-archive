# archive-core

`compatibility::inventory()` exposes source-linked native and planned option
behavior. `selection::NameSelection` matches stored name bytes with bounded
component wildcards or literal patterns and an explicit case policy. Its matches
never grant permission to publish a filesystem path.

`options::DeflateOptions` configures native effort levels 0–9 for
`deflate_stream_with_options`; default effort remains 6. This is backend tuning,
without a claim of identical 7-Zip output. Existing creation APIs remain intact.

`update` supplies all seven pairing states, add/update/delete/freshen/synchronize
action sets, repeated `-u` grammar and anti-item gating. Callers supply time
comparisons after applying archive precision/timezone/range rules. Classification
and parsing do not execute edits or prove content equality.

With `zip`, `zip_edit::plan` validates simultaneous rename/delete, modification-time
and encryption operations and
metadata preservation before `execute` streams unchanged packed data into an
empty provisional writer. Plans expose dry-run decisions; cancellation is checked
between bounded copies. Callers keep input stable, discard failed output and own
publication. Comments, attributes, known extras and ciphertext survive the
supported profile; unsupported preservation fails explicitly. No payloads are
decoded or authenticated by copying. `execute_with_options` accepts separate borrowed
credentials and caller randomness; changed encryption payloads are decoded and
verified before compressed-stream transformation. `validate_credentials` supports
preflight without output. ZIP UT seconds are authoritative, with a UTC-derived DOS
fallback; existing NTFS modification times are updated and other times preserved.

With `sevenz`, `sevenz_edit::edit` preserves raw file properties and unchanged packed
streams while editing timestamps or wrapping/removing AES at the compressed-stream
boundary. Encryption operates on complete compression groups; solid subsets fail.
Encrypted-header partial password changes fail without separate credentials. The
caller controls header encryption and supplies old/new passwords and randomness.
See [implementation scope](../../docs/archive-options-implementation.md).

Portable archive containers over `Read`, `Seek`, and `Write`, using
`ms-compress` codecs. Filesystem publication, path policy, scheduling and OS
randomness belong to callers (`archive-fs` and `archive-cli`), not this crate.

See [the capability matrix](../../docs/archive-capabilities.md) for supported
profiles, exclusions, license obligations and validation evidence. A format
name is not a promise that every variant can be decoded.

## API

`Archive::open(reader, limits)` indexes a seekable input. `open_as` selects an
explicit interpretation (required for signatureless `.lzma`); gzip, XZ,
and BZip2 automatically recognize decoded TAR unless explicitly selected as raw streams.
`entries()` returns stable IDs; `extract`, `read_entry(id, maximum)`, and `test`
decode and check available integrity information. `read_entry` allocates the
whole selected entry within its explicit maximum.
`entry_metadata(id)` exposes bounded optional source metadata for ZIP, TAR, 7z,
CAB, and gzip:
timestamps with explicit DOS-local/Unix semantics, mode/ownership/link bytes
where available, and container fields. TAR values reflect its stored header,
not PAX timestamp overrides. The WIM adapter exposes selected-image modification
times and read-only status; other backends currently return unknown fields.
`CreateOptions::entry_metadata` supplies metadata parallel to creation entries;
ZIP, TAR and compressed TAR, 7z, CAB and gzip retain the fields they support.
CAB creation takes local DOS timestamps. `create_stream_with_metadata` provides
the equivalent metadata input for forward-only creation. Native filesystem
application belongs to archive-fs; the portable core never changes ownership or
permissions on a caller's filesystem.

`extract_selected` decodes selected 7z solid folders once. With `parallel`,
`extract_selected_parallel` uses independent folders within worker/workspace
limits and reports sequential fallback. It does not split a single LZMA2
decoder into workers. Observer APIs preserve these algorithms and report
unknown physical/shared decode work as unknown.
`extract_selected_cancellable` and `test_cancellable` check a caller-provided
callback between decoded chunks; wrap input and output I/O to cancel opening,
source reads, and creation as well.

Output is provisional until the operation succeeds. `ExtractReport::verified`
means expected size and available container checks passed, not authenticity;
TAR and ISO do not provide payload authentication. Callers must stage output
and publish only after success. Links/special files are not extracted.

`create` accepts a seekable output. `create_stream` accepts `Write` only for
TAR, gzip, zlib, LZMA, XZ, optional BZip2/Brotli and compressed TAR. `SequentialTar` consumes `Read`
only, requiring each entry to be copied or skipped before advancing.
Indexed compressed TAR buffers the decoded TAR under `max_buffered_bytes`
(default 256 MiB), in addition to decoded output limits; raw compressed streams and forward creation avoid that full buffer.

`create_from_readers` accepts `CreateSource` descriptors and opens one payload
reader at a time for TAR, TAR.gz, TAR.xz, ZIP, 7z, and CAB, including encrypted
ZIP and 7z.
`create_stream_from_readers` supports the three TAR profiles without output
seeking. Readers must produce exactly their declared size. These APIs avoid
retaining all input payloads; the byte-based creation APIs remain available.
7z writes each compressed payload directly to the seekable destination, then
writes bounded archive metadata and patches its start header. Copy, DEFLATE,
LZMA, LZMA2, optional BZip2/Brotli, AES payload encryption, and encrypted headers
use this path. The writer keeps codec state and archive metadata, without a
whole packed-archive buffer or payload scratch file. 7z output still requires
`Write + Seek`; it is not a `create_stream_from_readers` format.
CAB creation uses `ms-cabinet` 0.1.3 reader sources for stored, MSZIP, LZX,
and Quantum folders. It opens one source at a time and uses a shared 32 KiB
input frame; compression retains its codec state. Output requires `Write + Seek`.


`Format::from_str` shares case-insensitive format aliases across callers;
enabled features and operation-specific checks determine supported operations.

The optional `crypto` feature exposes password opening and creation options.
Creation requires a caller-provided cryptographically secure `RandomSource`;
the crate does not obtain OS/browser randomness. ZipCrypto is explicit legacy
compatibility, not modern authenticated encryption.

## Reproducible Unencrypted Checks

From the workspace root with the committed lockfile:

```sh
cargo build --locked -p caddy-archive-core --no-default-features --features zip,tar,gzip,cab,iso,xz,streams
cargo test --locked -p caddy-archive-core --no-default-features --features zip,tar,gzip,cab,iso,xz,streams
cargo clippy --locked -p caddy-archive-core --all-targets --no-default-features --features zip,tar,gzip,cab,iso,xz,streams -- -D warnings
```

Independent-tool tests additionally need `7z`, `xz`, and `gzip`; ignored tests
are not included in ordinary test evidence. Browser runtime commands live in
[archive-wasm](../archive-wasm/README.md). These commands exclude encryption
but do not remove codec license obligations.

With `streams`, `single_stream::{compress, decompress, Codec, Options}` expose
single-file codecs including raw LZMA2 and Windows XPRESS Huffman/plain, LZX,
LZMS, LZNT1, and Quantum. Raw Windows blocks require an exact decoded length;
raw LZMA2 requires an agreed dictionary size. These APIs use bounded buffering
and provisional output, with no custom framing or integrity claim for raw blocks.
`CreateOptions::zip_compression` chooses stored or DEFLATE ZIP entries;
`cab_compression` chooses stored, MSZIP, LZX, or Quantum CAB folders.
`SevenZipCompression::Deflate` adds 7z DEFLATE writing and reading.

### Resource exhaustion and memory

ZIP member ranges must not overlap, including local headers. Declared entry
sizes, aggregate output, entry count, metadata, dictionaries, decoder workspace,
and path depth are limited; ZIP decoding also checks actual output before
writing it, so forged small sizes cannot bypass the decoded-byte limit.
Nested archives are ordinary files, not recursively extracted. Applications
that recurse must enforce a shared byte/work budget and recursion depth across
all archive instances. Repeated calls to `extract` are separate operations.

Memory limits are component budgets, not a hard process RSS ceiling. ZIP
extraction uses two 64 KiB codec buffers plus decoder state and the index;
output sent to a file does not accumulate the expanded member in memory.
`read_entry` instead retains the returned entry up to its caller-supplied cap.
Indexed compressed TAR retains up to 256 MiB by default, plus decoder state
and metadata. Its buffer uses fallible allocation and caps requested capacity;
allocator overhead and transient reallocations can still increase peak RSS.
Use `SequentialTar` for forward TAR processing without retaining the full TAR.
`Archive::open_with_scratch` accepts an empty seekable temporary file for indexed
compressed TAR; the CLI uses this path automatically. This spends bounded disk
space instead of retaining the decoded TAR in RAM. The scratch file remains
owned by the archive and is discarded on failure or drop when supplied as an
anonymous temporary file. Auto-detection inspects a bounded decoded prefix,
continues the same decoder into the selected destination, and verifies the
complete compressed stream before opening succeeds.

`wim::FileWimArchive::open_reader` uses seekable input and emits one codec chunk
at a time, verifying SHA-1 at completion. Range boundaries follow codec chunks
to avoid repeatedly decoding a large chunk for small output slices. Metadata and codec chunks remain
bounded allocations. The borrowed-byte `WimArchive` API remains available.
WIM output is provisional until success; the CLI stages each file before
publication. `udf::UdfArchive::open_reader` likewise reads the image by range. Package
adapters can still buffer members, and raw Windows block codecs use bounded whole-block buffers.
Creation APIs accepting `CreateEntry` also retain caller-provided entry data.
Thus this is not yet a no-whole-buffer guarantee for every API/format.
Other defaults include 16 MiB metadata, 64 MiB dictionary, 256 MiB active
workspace and 4 MiB pending parallel output. These are not additive guarantees
of total heap use; dependency allocations and caller-owned input/output count too.

`MemoryUsage::{Auto, Bytes, Percent}` parses a portable memory policy;
`budget(physical_ram, MemoryOperation::{Compress, Decompress})` resolves it to
bytes using RAM supplied by the caller. The core does not query the OS or
implicitly change `Limits`. The automatic policy follows 7-Zip: creation uses
80% of detected RAM, while decompression uses `RAM / 32 * 17` (approximately
53.125%). The automatic RAM base is capped at 1.75 GiB on 32-bit targets.
When RAM detection is unavailable, automatic budgets fall back to 2 GiB on
64-bit targets or 1 GiB on 32-bit targets.
Explicit values accept decimal bytes, binary `b/k/m/g/t` suffixes, and either
`50%` or `p50` percentage notation, case-insensitively. Percentages refer to total
RAM, not currently free RAM. These defaults and spellings follow 7-Zip 26.04's
`CCommonMethodProps::InitCommon` and `ParseSizeString` in
`CPP/7zip/Archive/Common/HandlerOut.{h,cpp}`.

The native CLI applies that policy through `--memuse` (`--mmemuse` and
`-mmemuse=VALUE` also work). It uses detected processor availability as its
worker ceiling and reduces independent 7z folder concurrency to fit estimated
aggregate workspace and pending-output budgets. `--max-codec-workspace-bytes`
and `--max-dictionary-bytes` can impose tighter independent ceilings. CLI
dictionaries otherwise share the resolved workspace ceiling; the portable
`Limits` defaults and bounded browser configuration remain unchanged.
These controls budget codec work and scheduling, not total process RSS.

Output defaults (8 GiB per entry, 32 GiB total) accommodate large archives;
choose smaller `Limits` for untrusted uploads. Absolute byte limits intentionally
allow legitimate highly compressible files. There is no general CPU deadline
or process-wide allocator limit: isolate hostile workloads with OS memory,
time and disk quotas where those guarantees are required.
