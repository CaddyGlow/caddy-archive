# Archive Completion Audit

Independent implementation/evidence audit, 2026-10-06. The plan remains the
full requested objective. Supported profiles and passing tests are useful
evidence, but do not make all phases complete.

## Highest-Priority Remaining Requirements

1. **Bounded browser steps beyond ZIP.** `archive-wasm/README.md` and
   `js/range-worker.js` describe an incremental 64 KiB ZIP decoder only.
   Other `ByteArchive` formats still copy whole inputs and run synchronous
   indexing/extraction. Moving them into a Worker does not satisfy the plan's
   requirement to service progress/cancellation during one huge entry. Each
   advertised large-file profile needs a resumable decoder/range contract,
   backpressure and an actual huge-entry cancellation/memory test, or a clear
   small-input-only restriction until that implementation exists.
2. **Eligible intra-stream LZMA2 parallelism.** `sevenz_backend` schedules
   independent folders and reports solid fallback; `parallel_measure` and the
   independent encrypted/filter test validate that profile. They do not test
   independent dictionary-reset blocks inside one eligible LZMA2 stream.
   Pipeline eligibility, reset-block planning, shared workspace accounting,
   ordering/checksums and worker 1/2/4 comparisons remain unimplemented gates.
   Existing folder performance samples are not evidence for block parallelism.
3. **Progress acceptance measurements.** `archive-cli/examples/progress-bench.rs`
   distinguishes baseline/disabled/snapshots/terminal and explicitly records
   `browser_measured:false`. Real Worker correctness does not establish the
   native/WASM sustained throughput target (no disabled regression, at most 1%
   median snapshot overhead), variance, latency and real terminal cost on the
   same workload. A non-TTY terminal-adapter run cannot count as terminal
   rendering. Actual counters for physical/shared work are often unknown;
   unknown markers are honest but not an operation-level accounting benchmark.

These are implementation/acceptance gaps, not claims that an intentionally
unsupported format extension must suddenly be advertised.

## Common API Review

| Plan Item | Current Evidence / Gap |
| --- | --- |
| 1. Bounded probing, detected/requested format | Seekable signature fill, explicit raw selectors, optical system-area probe and regression coverage exist |
| 2. Byte I/O and range adapters | Read/Write/Seek and synchronous RangeSource adapter exist; asynchronous browser orchestration is ZIP-specific |
| 3. Sequential versus indexed | SequentialTar and forward DEFLATE/gzip/zlib APIs exist; indexed compressed TAR still buffers decoded TAR; no forward compressed-TAR enumeration adapter |
| 4. Metadata | Entry retains raw/display names and sizes; new bounded entry_metadata exposes ZIP DOS local time/mode/CRC/AES fields and TAR stored-header time/mode/ownership/link bytes; PAX overrides and other backends' timestamps/attributes remain incomplete |
| 5. List/sink/test/create and explicit Vec bound | Implemented; callers must keep provisional bytes unpublished until success |
| 6. Container selectors | WIM index/name and optical selected profiles exist; generic archive volume/spanning resolver remains absent |
| 7. Capability queries | Feature-sensitive profiles exist; verify each new backend/variant rather than equating format names with universal support |
| 8. Checked u64 and bounded WASM | Existing range-index checks and budget regressions cover ZIP; large non-ZIP browser workflows still require the bounded-step work above |
| 9. Structured errors | Implemented categories; wrong-password/corruption distinction remains intentionally non-universal |

## Phase Gates

| Phase | Gate Evidence / Remaining Limits |
| --- | --- |
| 0 | Profile/license documentation and real Worker baseline exist; broader source metadata contracts remain partial |
| 1 | Native/Worker profile round trips, safe filesystem staging and independent tests exist; larger/streaming non-ZIP browser gate remains limited |
| 2 | ZIP AES/legacy independent interoperability, wrong-password/no-plaintext and browser crypto tests exist; retain per-variant stored/descriptor/ZIP64 matrix rather than infer all combinations |
| 3 | Direct 7z Copy/LZMA/LZMA2, filters/encryption, folders 1/2/4 and fallback tests exist; eligible intra-stream blocks and their measured gate do not |
| 4 | XZ checks/multiblock/concat and independent tools exist; filter variants require their individual fixtures, and indexed TAR.XZ remains buffered |
| 5 | ISO/UDF selected profiles and WIM image selectors exist; Microsoft production ESD and broader optical variants are not established by synthetic fixtures |
| 6 | Single/bundle package parity and corruption regressions exist; independent Microsoft-tool acceptance must be recorded separately from native/WASM agreement |
| 7 | Portable MSI/media mapping and malformed-schema fix exist; independent Windows table/stream/payload acceptance and each external/multiple-media profile need explicit evidence |
| 8 | Capabilities/licenses, fixture provenance, bounded fuzz automation and benchmark examples exist; scheduled workflow configuration is not remote execution, short campaigns are not sustained fuzzing, and the three priorities above remain open |

## Implemented During This Audit

Added `Archive::entry_metadata(EntryId)` without breaking existing Entry
constructors. It does not decode payloads: ZIP uses metadata captured during
indexing, TAR reads one 512-byte stored file header. DOS timestamps preserve
local-wall-time semantics rather than inventing UTC. Unknown backend fields
remain `None`; TAR PAX timestamp overrides are explicitly not represented by
this stored-header API. Tests verify source timestamps, permissions/ownership,
link targets, ZIP fields, invalid IDs and extraction after metadata access.
This narrows Common API item 4 but does not mark it fully complete.
