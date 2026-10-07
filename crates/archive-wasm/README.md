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
cargo run --manifest-path ../ms-package/Cargo.toml --example browser_fixtures -- /tmp/archive-browser/pkg/fixtures
node scripts/archive-browser-server.mjs
node scripts/check-archive-browser.mjs
```

The static server uses port 8786 and the Chromium observer uses debugging port
9786 by default. Override the latter with `ARCHIVE_CDP_PORT`.
