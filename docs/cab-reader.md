# Native CAB reader

`windows-uup inspect --uup-dir UUPs` reads each CAB's root `update.mum`
automatically. `--cab-tool 7z` selects the previous external extractor explicitly.
The built-in implementation in `src/cab.rs` parses CFHEADER, CFFOLDER, CFFILE,
and CFDATA and streams members without extracting archive-defined filesystem
paths. No CAB library or external extractor is linked or launched by the default
path.

```sh
cargo run --locked -- cab-list --input package.cab
cargo run --locked -- cab-extract --input package.cab \
  --member update.mum --output update.mum
cargo run --locked -- inspect --uup-dir UUPs --output inventory.json
```

`cab-list` emits member names, sizes, raw DOS timestamps/attributes, compression,
folder indexes, and offsets as JSON. `cab-extract` writes a single named member
to an explicit output path, streams through a temporary file, and publishes only
after successful decoding. Existing output files are preserved. The default
uncompressed-member limit is 64 MiB; `--limit` accepts a different byte limit.
Inspect's manifest limit is 16 MiB and is checked before decoding. A member's
archive name is never used to choose its destination, including names containing
`..`, drive letters, or separators.

## Supported formats and boundaries

- CAB version 1.3, including header/folder/data reserve areas and signed
  archives with trailing data beyond `cbCabinet`.
- Complete multi-cabinet sets opened from any volume: previous/next links,
  set IDs and consecutive indexes, continued member directories and folders,
  split CFDATA reassembly, and persistent compression state. Neighbor names are
  ASCII case-insensitive filenames in the same directory; missing volumes,
  ambiguous names, symlinks, path traversal, and conflicting metadata fail.
  The full set is validated before listing or extraction.
- Stored data, MSZIP with cross-block dictionary history, and cabinet LZX with
  32 KiB through 2 MiB windows, persistent Huffman tables, recent offsets, block
  continuation, circular history, and frame-relative Intel E8 translation.
- Strict UTF-8 member names with the UTF-8 attribute, or ASCII legacy names.
  Legacy non-ASCII names require a code page and return an unsupported error.
- ASCII case-insensitive member matching with slash/backslash equivalence.
  Duplicate normalized names are rejected; a nested `update.mum` does not replace
  a missing root manifest.
- Directory and compressed-data bounds, member ranges, folder overlap, frame
  sizes, and nonzero CFDATA checksums are validated. Decoder errors invalidate
  the current member reader. Zero checksums mean the producer supplied no block
  checksum, as allowed by CAB.

Quantum supports dictionary orders 10 through 21, persistent arithmetic models
and history, and per-frame arithmetic restart. CAB
reading does not decompress PSF/PSFX patches or WCP-compressed component XML,
validate catalogs/signatures, establish package applicability, or verify Windows
installed state. Reading a member validates its decoded prefix of the solid
folder; unread members and folders have no payload-integrity claim. Inputs must
remain stable during inspection; this is not an atomic image snapshot.

For `inspect`, missing or invalid `update.mum` and unsupported archives generate
per-file warnings and leave that payload's `metadata_verified` false. Other CABs
continue to be inspected. A successful manifest parse populates identity,
parent/prerequisite expressions, and the existing XML hash; it leaves
`applicability_verified` and download checksum verification independent.

## Codec reuse and provenance

The cabinet LZX decoder lives in
`../ms-compress/src/lzx/cabinet.rs` and shares canonical-code and
code-length decoding and offset tables with the existing WIM decoder. WIM's
independent-chunk framing, padding behavior, and fixed E8 transform are preserved.
The existing codec and the cabinet extension retain LGPL-2.1-or-later; see
`../ms-compress/src/LZX-NOTICE.md` and its license files.

MSZIP uses the Rust backend of `flate2` for raw DEFLATE; the repository had no
DEFLATE decoder. Cabinet dictionary seeding and framing are implemented locally.
The CAB framing/checksum behavior and small independent LZX test sample were
reviewed against `cab` 0.6.0 (Matthew D. Steele, MIT). Its full notice is retained
in [cab-MIT.txt](licenses/cab-MIT.txt). The external `cab` and `lzxd` crates are
not dependencies. Format reference:
[Microsoft MS-CAB](https://download.microsoft.com/download/4/D/A/4DA14F27-B4EF-4170-A6E6-5B1EF85B1BAA/%5BMS-CAB%5D.pdf).

## Validation — 2026-10-03

- 38 CAB integration tests cover stored/MSZIP/LZX/Quantum extraction, solid dictionaries,
  frame continuation, E8 translation, reserves, valid/invalid checksums,
  truncation, malformed offsets/sizes, ambiguous names, unsupported formats,
  bounded extraction, UTF-8/UTF-16 manifest parsing, default CLI inspection,
  failed-publication cleanup, and preserved inputs/outputs. A preserved Microsoft
  CAB and an independently produced MIT sample are exercised.
- Six new codec tests plus the complete existing `ms-compress` suite passed,
  including differential WIM tests against retained original-C results.
- The full host suite, formatting, and Clippy with warnings denied passed.
  Windows x64 MSVC `cargo check --all-targets` passed; Windows runtime extraction
  was not executed for this change.
- The real 47-file UUP directory was inspected with both native and independent
  7z extraction. All 27 CABs were listed; 26 root package manifests parsed with
  identical metadata. KB5126029 has no root `update.mum` and remains explicitly
  unverified. 94 selected members, including non-manifest payloads, matched 7z
  byte-for-byte. Before/after SHA-256 hashes matched for all 47 source files.
  This samples first/middle/last members from the <=16 MiB population plus root
  manifests; it does not claim every member was decoded.

The [comparison report](fixtures/cab-reader/uup-comparison.json) retains binary,
source, and member hashes, reference version, counts, and warnings. Reproduce it
with the native binary and an independent 7z executable:

```sh
cargo build --release --locked --bin windows-uup
python3 ../windows-uup/scripts/validate-cab-reader.py --uup-dir UUPs \
  --binary target/release/windows-uup --output /tmp/cab-comparison.json
cargo test --locked --test cab
cargo test --locked -p ms-compress
```

## Spanning cabinet validation — 2026-10-03

The [retained five-volume fixture](../tests/fixtures/cab-spanning/README.md)
comes from cabextract’s makecab-generated corpus. Native extraction from each
starting volume matched independent p7zip for all six members: 30 comparisons,
140,128 bytes per complete set. Source hashes were unchanged. The
[comparison report](fixtures/cab-reader/spanning-comparison.json) retains the
reference version and all source/member hashes; reproduce it with
`../windows-uup/scripts/validate-cab-spanning.py`.

Additional tests cover stored/LZX split frames, MSZIP dictionary continuity,
a Huffman-coded LZX block split across three volumes, each fragment’s checksum,
CLI extraction, default manifest inspection, and malformed/missing neighbors.
CFDATA reserves are excluded from checksums to match makecab and independent
readers. Quantum decoding was added in the extension described below.

`Cabinet::open` discovers the full on-disk set. `Cabinet::from_parts` accepts
explicit seekable sources in cabinet-index order and validates their set metadata;
`Cabinet::new` requires a standalone source. Continued member records must agree in
member order, names, offsets, sizes, timestamps, and attributes. Salvaging damaged
or incomplete sets is unsupported. Complete compressed blocks are limited to
38,912 bytes. CLI commands accept any part without new flags:

```sh
cargo run --locked -- cab-list --input tests/fixtures/cab-spanning/split-3.cab
cargo run --locked -- cab-extract --input tests/fixtures/cab-spanning/split-3.cab \
  --member medium2.bin --output /tmp/medium2.bin
```

For a spanning set, `inspect` reads the complete set’s root manifest for each
physical CAB payload; it does not assign an independent package identity to each
fragment. The makecab fixture contains binary members rather than package XML;
spanning-manifest inspection is covered by a synthetic package fixture.

Final checks for this extension: all 32 CAB integration tests and the full host
suite passed, together with formatting, Clippy with warnings denied, and Windows
x64 MSVC all-target compilation. Host copy tests require execution outside the
sandbox’s unmapped filesystem namespace; the initial sandbox-only run failed
those unrelated ownership checks. Windows runtime extraction was not executed.

## Quantum validation — 2026-10-03

The native Quantum decoder in `ms-compress/src/quantum.rs` handles all seven
selectors, 27 length slots, direct offset/length bits, all dictionary orders
10–21, model rescaling and exact reorder semantics, overlapping matches,
dictionary wraparound, solid folders, and spanning cabinet fragments. Compression
levels 1–7 are retained as metadata; they change the producer's choices rather
than the decoding grammar. `cab-list` now represents Quantum as an object with
`level` and `window_order` fields, matching LZX's parameterized representation.

The [fixture corpus](../tests/fixtures/cab-quantum/README.md) includes a real
upstream mixed-compression CAB, generated inputs validated independently with
p7zip, a 128-frame/4 MiB history case with 2 MiB match distances, a synthetic root
package manifest, and three malformed CVE cases. Every member of all 15 valid
cabinets matched 7z: 30 comparisons, including 28 Quantum members. All source
hashes were unchanged. See the [report](fixtures/cab-reader/quantum-comparison.json)
and `../windows-uup/scripts/validate-cab-quantum.py` for reproduction.

Four codec tests cover every truncation of the real frame, failed-decoder state,
window/input/output bounds, allowed zero padding, invalid padding, and malformed
or mutated bitstreams. CAB integration tests additionally verify small reads,
later-member selection, split blocks with persistent models/history, declared
levels, default manifest inspection, and the three CVE archive rejections.
Implicit missing bits, out-of-history matches, and frame overshoots are errors.

The Quantum module retains libmspack/7-Zip attribution and LGPL terms in
`../ms-compress/src/QUANTUM-NOTICE.md`. Quantum is LGPL-2.1-only;
other existing codecs retain LGPL-2.1-or-later, reflected in crate metadata.
No external decoder is required at runtime. The supplied arXiv paper describes
qubit-source compression and does not describe the CAB Quantum format.

```sh
cargo run --locked -- cab-extract \
  --input tests/fixtures/cab-quantum/mszip_lzx_qtm.cab \
  --member qtm.txt --output /tmp/qtm.txt
cargo test --locked --test cab
cargo test --locked -p ms-compress
```

Final Quantum checks passed: all 38 CAB integration tests, four new Quantum codec
tests plus the full codec suite, the full host suite, formatting, and both Clippy
runs with warnings denied. Windows x64 MSVC all-target compilation passed;
Windows runtime extraction was not executed. The source/member hashes and
binary hash for the independent comparisons are retained in the report above.

## Performance

The [release extraction benchmark](cab-benchmark.md) compares the native reader
with installed p7zip 17.05 and cabextract 1.11 on retained fixtures and real UUP
CABs. In a fifteen-run confirmation, native extraction was faster than both
references on all four measured LZX cases: about 3–6% less time than p7zip on
large cases, and 31% less on ISE. These are workload-specific warm-cache results;
Quantum still trails cabextract on the larger fixture. Shared Huffman changes
also apply to WIM, whose performance has not been measured here. The report
includes methodology and raw samples and does not claim pure-codec, cold-cache,
or current-version 7-Zip superiority.
