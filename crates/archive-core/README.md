# archive-core

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

Output is provisional until the operation succeeds. `ExtractReport::verified`
means expected size and available container checks passed, not authenticity;
TAR and ISO do not provide payload authentication. Callers must stage output
and publish only after success. Links/special files are not extracted.

`create` accepts a seekable output. `create_stream` accepts `Write` only for
TAR, gzip, zlib, LZMA, XZ, optional BZip2/Brotli and compressed TAR. `SequentialTar` consumes `Read`
only, requiring each entry to be copied or skipped before advancing.
Indexed compressed TAR currently buffers the decoded TAR under the output
budget; raw compressed streams and forward creation avoid that full buffer.

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
