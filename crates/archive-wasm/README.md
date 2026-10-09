# archive-wasm

The initial facade accepts bounded byte inputs and exposes archive indexing,
entry decoding, integrity testing, and single-file creation. It has no native
filesystem adapter dependency. `ByteArchive` copies its input into WASM memory;
returned payloads are copied across the JavaScript boundary. This adapter is for
small archives. ZIP has a separate bounded Blob range API; other formats do not
yet provide that range API.

Run operations in a dedicated Web Worker. ByteArchive calls are synchronous.
The `js/range-worker.js` ZIP helper supplies incremental decoding, asynchronous
Blob reads, awaited output backpressure, progress and AbortSignal cancellation.
Its Worker protocol acknowledges chunks before continuing. Bytes remain
provisional until the operation's integrity check succeeds.
The optional `crypto` feature supplies ZIP AES encryption using browser Web Crypto
randomness. The optional `packages` feature supplies single APPX/MSIX integrity
validation and embedded MSI payload reading. Browser password byte arrays and
JavaScript strings remain under caller ownership; their complete erasure is not
guaranteed by the facade.

Build with:

```sh
cargo build -p caddy-archive-wasm --target wasm32-unknown-unknown --locked
```

A successful build is only a compilation gate. Real browser execution remains
a separate gate.

The Chromium Worker regression harness passes ZIP, TAR, TAR.GZ, CAB, 7z, XZ,
TAR.XZ and explicit gzip/zlib/raw-DEFLATE/LZMA/BZip2/Brotli round trips,
BZip2/Brotli TAR wrappers, entry allocation limits,
ZIP and 7z AES-256 round trips and fresh randomness,
wrong passwords, native/browser MSIX and MSI payload parity, and block-map
corruption. It also exercises bounded 16 MiB ZIP Blob extraction and cooperative
cancellation. Generate bindings and package fixtures in temporary storage:

```sh
cargo build -p caddy-archive-wasm --features packages,crypto,sevenz,bzip2,brotli --target wasm32-unknown-unknown --locked
wasm-bindgen "$CARGO_TARGET_DIR/wasm32-unknown-unknown/debug/archive_wasm.wasm" --target web --out-dir /tmp/archive-browser/pkg
mkdir -p /tmp/archive-browser/pkg/fixtures
cp crates/archive-wasm/tests/fixtures/package/* /tmp/archive-browser/pkg/fixtures/
node scripts/archive-browser-server.mjs
node scripts/check-archive-browser.mjs
```

The static server uses port 8786 and the Chromium observer uses debugging port
9786 by default. Override the latter with `ARCHIVE_CDP_PORT`.

`edit_zip(bytes, operations_json, max_input_bytes, max_metadata_bytes,
max_decoded_bytes, max_output_bytes)` returns a new ZIP artifact for rename and
delete operations. For example, use
`[{"operation":"rename","from":"old.txt","to":"new.txt"}]` or
`[{"operation":"delete","name":"old.txt"}]`. Directory operations use names
ending in `/` and match descendants at path boundaries. Operations refer to
original names and run simultaneously. Input, operation JSON, archive metadata,
declared decoded sizes and resulting bytes have explicit budgets.

This initial profile preserves packed bytes, ciphertext, comments, attributes
and supported timestamp/encryption extras. It rejects split archives, SFX,
trailing data, duplicate names, unknown/name-dependent extras and unsupported
codecs before output. It requires no password for packed editing, so success
provides structural validation and does not authenticate encrypted payloads.
Open the resulting bytes and test them separately when verification is required.

Run this synchronous call in a dedicated Worker. It has no incremental steps or
AbortSignal cancellation; terminating that Worker abandons unfinished work.
Publication remains the caller's responsibility. Editing byte inputs copies the
result into WASM memory and then across the JavaScript boundary; use the output
budget to cap allocation. The Worker harness covers unencrypted and AES ZIP
rename/delete, unchanged packed ciphertext, source preservation and budget errors.

The facade also rejects recognized APPX/MSIX manifests, block maps, bundle
manifests, package signatures and signed JAR metadata. Those containers need an
explicit package/signature editing policy. The byte-slice and JSON arguments
are copied into WASM by generated bindings before Rust can enforce budgets;
callers must bound those JavaScript inputs before calling, as with `ByteArchive`.

`edit_zip` also accepts `{"operation":"modified","name":"file","modified_unix_seconds":1700000001}`.
With `crypto`, `edit_zip_with_passwords(bytes, operations_json, old_password,
new_password, max_input_bytes, max_metadata_bytes, max_decoded_bytes,
max_output_bytes)` accepts separate optional byte-array credentials. Use
`{"operation":"encryption","name":"file","encrypted":true}` to encrypt/rekey
with AES-256, or `false` to decrypt. Source payloads undergoing encryption changes
are verified; untouched ciphertext is copied exactly. ZIP filenames remain visible.

With `sevenz,crypto`, `edit_7z(bytes, operations_json, old_password, new_password,
encrypt_headers, max_input_bytes, max_metadata_bytes, max_decoded_bytes,
max_output_bytes)` accepts the same modified/encryption JSON forms. An omitted
name selects all entries; named operations require exact decoded names. Header
policy is an optional boolean: `true` hides names, `false` removes that protection,
and omission preserves it. Whole compression groups can be transformed without
recompression. Solid subsets, explicitly selected empty payloads, and partial
password changes under encrypted headers fail explicitly. Raw untouched 7z
metadata, including timestamp precision, is preserved.

Both APIs use fresh Web Crypto randomness, keep passwords out of JSON, and clear
owned WASM credential copies on return. Callers must erase their JS credential
arrays and bound all JS inputs before generated bindings copy them. Run editing
in a dedicated Worker and terminate it to cancel. The real browser checks cover
timestamp changes, rekeying, decryption, wrong passwords and hidden 7z filenames.
`ByteArchive.entry_metadata_json(id)` exposes stored times and format metadata.
