# CAB extraction benchmark — 2026-10-03

The latest native reader beats installed p7zip 17.05 and cabextract 1.11 on all
four measured LZX cases in both a seven-run session and a fifteen-run confirmation.
The confirmed large cases take 3–6% less time than p7zip and 4–18% less than
cabextract. ISE takes about 31% less time than p7zip. These are warm-cache,
process-level CAB extraction results, not a claim about every LZX archive or
current 7-Zip releases. The original native baseline took roughly four times
as long on Hello Face and 2.6 times as long on Speech. WIM speed was not measured.

The decoder uses an 11-bit canonical Huffman lookup with a long-code fallback.
CAB decoding now writes directly into the history window, then copies each
completed frame into caller output once. Intel E8 translation changes only the
output, preserving raw history. Match bookkeeping advances once per match;
contiguous repeated patterns use prefix doubling, distance-one matches use fill,
and ring-wrapping matches copy bounded spans. The shared Huffman lookup also
applies to WIM, but the direct-history changes apply to CAB framing.

## Fixed-width Huffman, reusable buffers, and block runs (latest)

CAB symbol reads now pass their already-validated code width (16 or 7 bits)
to an inlined nonempty lookup path. Empty-code checks remain at tree creation
and required length/aligned-code use. The generic WIM reader preserves its
existing empty-code behavior. CAB member readers reuse compressed and decoded
frame buffers instead of allocating and clearing fresh buffers for every frame.
Finally, the compressed symbol loop computes its block/frame end once and
updates the remaining block size after the run, replacing repeated header/type
checks and per-symbol remaining-size stores. Match bounds and truncation checks
remain enforced. No unsafe code or architecture-specific instructions were added.

A short-match scalar-copy experiment was slower and was discarded. A separate
experiment with deferred literal checks and split-slice memcpy regressed Speech
and was also discarded. The intermediate fixed-width/buffer-only benchmark was
approximately tied with p7zip on the large cases; block-run processing established
the measured lead. Intermediate evidence:
[benchmark-lzx-fixed-buffer-2026-10-03.json](fixtures/cab-reader/benchmark-lzx-fixed-buffer-2026-10-03.json).

| Selected member | Native (ms) | 7z (ms) | cabextract (ms) | Less time than 7z |
|---|---:|---:|---:|---:|
| LZX ISE manifest | 13.124 | 19.061 | 15.236 | 31.2% |
| LZX Hello manifest | 204.779 | 217.850 | 249.493 | 6.0% |
| LZX Hello payload | 197.808 | 206.248 | 238.698 | 4.1% |
| LZX Speech manifest | 537.589 | 554.717 | 559.711 | 3.1% |

Fifteen interleaved measured runs after verified warmups; all three outputs
matched size/SHA-256 and all CAB source hashes remained unchanged. Every native
LZX median was lower than both references in both independent sessions:
[seven-run report](fixtures/cab-reader/benchmark-lzx-runs-2026-10-03.json)
and [fifteen-run confirmation](fixtures/cab-reader/benchmark-lzx-runs-confirmed-2026-10-03.json).
The confirmation is the current comparison; earlier sessions remain retained.
The small Speech lead should be interpreted within this host and workload,
without generalizing to cold-cache reads or whole-archive extraction.

The host/codec suites, all 39 CAB integration tests, and denied-warning Clippy
passed. New regressions compare fixed-width reads with generic reads for every
known canonical codeword and ensure a corrupt short block after a full frame
cannot expose old bytes or allow reader reuse. Existing tests cover stored,
MSZIP, Quantum, LZX, spanning CFDATA, overlapping history, and frame boundaries.
All 94 independently selected members from 27 real CABs matched installed 7z;
source hashes were unchanged:
[lzx-runs-uup-comparison-2026-10-03.json](fixtures/cab-reader/lzx-runs-uup-comparison-2026-10-03.json).

## Earlier word-based CAB checksum

The current Speech extraction profile recorded 7,679,756,694 instructions.
CAB reading/checksum work remained a useful target alongside bit decoding;
long Huffman fallback remained small (about 0.38%). The pre-change function
profile and raw data are retained in
[lzx-word-checksum-profile-before-2026-10-03.txt](fixtures/cab-reader/lzx-word-checksum-profile-before-2026-10-03.txt)
and [callgrind data](fixtures/cab-reader/lzx-word-checksum-profile-before-2026-10-03.callgrind).
The post-change profile recorded 6,776,641,656 instructions, 11.8% fewer for
this identical Speech extraction, consistent with the paired wall-time improvement.
The matched profiling builds used release optimization with debug information.
Post-change evidence:
[lzx-word-checksum-profile-after-2026-10-03.txt](fixtures/cab-reader/lzx-word-checksum-profile-after-2026-10-03.txt)
and [callgrind data](fixtures/cab-reader/lzx-word-checksum-profile-after-2026-10-03.callgrind).
Instruction counts are not elapsed times.

CFDATA checksum calculation now XORs complete little-endian four-byte words
using a fixed-size chunk iterator, allowing compiler vectorization instead of
assembling each word byte by byte. The trailing one-to-three bytes still use
CAB's big-endian folding rule. This applies to all CAB compression methods;
the LZX codec itself is unchanged in this round.

A bounded 32-bit refill experiment passed correctness checks but did not improve
measured time, so it was rejected. The original bit reader remains in use.
Additional bit-order and truncation regression coverage was retained.

| Selected member | Native (ms) | 7z (ms) | cabextract (ms) |
|---|---:|---:|---:|
| LZX ISE manifest | 15.821 | 19.381 | 15.596 |
| LZX Hello manifest | 236.359 | 218.423 | 249.914 |
| LZX Hello payload | 227.237 | 211.379 | 238.298 |
| LZX Speech manifest | 607.061 | 554.661 | 559.933 |

Seven interleaved runs, identical warm-up outputs, unchanged source hashes:
[benchmark-lzx-word-checksum-2026-10-03.json](fixtures/cab-reader/benchmark-lzx-word-checksum-2026-10-03.json).
A separate seven-pair comparison alternated the preserved previous release
binary and the new binary:

| Selected member | Previous native (ms) | New native (ms) | Less elapsed time |
|---|---:|---:|---:|
| LZX ISE manifest | 16.075 | 14.542 | 9.5% |
| LZX Hello manifest | 294.820 | 236.060 | 19.9% |
| LZX Hello payload | 284.110 | 228.061 | 19.7% |
| LZX Speech manifest | 686.535 | 607.468 | 11.5% |

Raw paired samples and binary hashes:
[lzx-word-checksum-paired-2026-10-03.json](fixtures/cab-reader/lzx-word-checksum-paired-2026-10-03.json).
The complete host/codec suites and denied-warning Clippy passed. Checksum
regressions cover every tail length, unaligned input slices, explicit endian
vectors and large blocks against an independent bytewise calculation. Bit-reader
regressions cover reads of 0–17 bits and truncated/odd-length word streams.
Independent 7z comparisons again matched 94 selected members and 26 manifests
from 27 CABs, with source hashes unchanged:
[lzx-word-checksum-uup-comparison-2026-10-03.json](fixtures/cab-reader/lzx-word-checksum-uup-comparison-2026-10-03.json).

## Earlier common-path optimization

Profiling the direct-history Hello manifest extraction with Callgrind 3.26.0
recorded 4,720,011,345 instructions. Long Huffman fallback took about 0.25%;
the scalar E8 scan accounted for about 8.5%. Common lookup and bit handling
were substantially larger. Instruction shares are not elapsed-time shares.
The annotated pre-change profile and raw callgrind data are retained as
[lzx-direct-history-profile-2026-10-03.txt](fixtures/cab-reader/lzx-direct-history-profile-2026-10-03.txt)
and [callgrind data](fixtures/cab-reader/lzx-direct-history-profile-2026-10-03.callgrind).

The shared Huffman reader now probes a fixed 11-bit prefix directly. Short-code
entries are replicated across unused suffixes; only fallback reads the full
variable-length prefix. This removes a variable shift from the common path.
CAB E8 scanning probes eight bytes at once and skips a word only when it contains
no E8 byte. Input refill and truncation checking remain intact. No secondary
Huffman table was added because fallback's measured share was small.

| Selected member | Native (ms) | 7z (ms) | cabextract (ms) |
|---|---:|---:|---:|
| LZX ISE manifest | 15.680 | 18.675 | 15.146 |
| LZX Hello manifest | 294.754 | 217.275 | 249.306 |
| LZX Hello payload | 285.521 | 206.429 | 237.176 |
| LZX Speech manifest | 686.173 | 553.606 | 558.985 |

Seven interleaved runs, hash-verified warmups, unchanged sources. Raw report:
[benchmark-lzx-common-path-2026-10-03.json](fixtures/cab-reader/benchmark-lzx-common-path-2026-10-03.json).
Seven alternating pairs against the preserved direct-history binary isolate
this optimization:

| Selected member | Previous native (ms) | New native (ms) | Less elapsed time |
|---|---:|---:|---:|
| LZX ISE manifest | 18.551 | 15.447 | 16.7% |
| LZX Hello manifest | 335.722 | 295.339 | 12.0% |
| LZX Hello payload | 320.908 | 284.281 | 11.4% |
| LZX Speech manifest | 773.862 | 687.297 | 11.2% |

Paired samples and binary hashes:
[lzx-common-path-paired-2026-10-03.json](fixtures/cab-reader/lzx-common-path-paired-2026-10-03.json).
The codec and host suites and denied-warning Clippy passed. New regressions
cover all unused short-code suffixes without extra bit consumption and compare
the word E8 scan against a scalar reference at every byte lane and near frame
ends, including negative and out-of-range addresses. All 94 selected members
from 27 CABs again matched independent 7z; source hashes remained unchanged:
[lzx-common-path-uup-comparison-2026-10-03.json](fixtures/cab-reader/lzx-common-path-uup-comparison-2026-10-03.json).
WIM gains the shared lookup change; these timing measurements are CAB-only.

## Direct-history results

| Selected member | Native (ms) | 7z (ms) | cabextract (ms) |
|---|---:|---:|---:|
| LZX ISE manifest | 18.102 | 18.274 | 15.371 |
| LZX Hello manifest | 335.109 | 216.915 | 248.347 |
| LZX Hello payload | 322.615 | 206.386 | 236.766 |
| LZX Speech manifest | 773.588 | 555.476 | 560.315 |

These seven-run interleaved results are retained in
[benchmark-lzx-direct-history-2026-10-03.json](fixtures/cab-reader/benchmark-lzx-direct-history-2026-10-03.json).
A separate seven-pair run alternated the preserved previous binary and the new
binary to isolate this change from timing drift:

| Selected member | Previous native (ms) | Direct history (ms) | Less elapsed time |
|---|---:|---:|---:|
| LZX ISE manifest | 19.048 | 17.879 | 6.1% |
| LZX Hello manifest | 348.266 | 335.930 | 3.5% |
| LZX Hello payload | 334.671 | 320.421 | 4.3% |
| LZX Speech manifest | 826.288 | 774.672 | 6.2% |

Paired samples and binary hashes:
[lzx-direct-history-paired-2026-10-03.json](fixtures/cab-reader/lzx-direct-history-paired-2026-10-03.json).
The paired ISE improvement is smaller than a comparison between separate
sessions suggests; use the paired measurements for this change's speedup.

The full codec and host suites and both denied-warning Clippy checks passed.
Regression coverage compares direct-history matches with a bytewise reference
across every supported window size, overlap, exact-window distances, and both
ring boundaries. Independent 7z comparison again matched all 94 selected members
and 26 manifests from 27 CABs, with unchanged source hashes:
[lzx-direct-history-uup-comparison-2026-10-03.json](fixtures/cab-reader/lzx-direct-history-uup-comparison-2026-10-03.json).

## Earlier Huffman optimization

| Selected member | Native before (ms) | Native after (ms) | Speedup | 7z after (ms) | cabextract after (ms) |
|---|---:|---:|---:|---:|---:|
| LZX ISE manifest | 32.514 | 25.861 | 1.26× | 19.291 | 17.003 |
| LZX Hello manifest | 810.945 | 348.637 | 2.33× | 217.213 | 249.178 |
| LZX Hello payload | 778.701 | 333.664 | 2.33× | 206.942 | 236.928 |
| LZX Speech manifest | 1396.853 | 829.197 | 1.68× | 555.663 | 560.606 |

Before and after are separate seven-run sessions using the same cases and
methodology; timings are not simultaneous paired measurements. Reference times
on the large cases remained similar. All three tools produced identical warm-up
bytes and all input CAB hashes were unchanged in the optimized run. The raw
optimized samples are retained in
[benchmark-lzx-optimized-2026-10-03.json](fixtures/cab-reader/benchmark-lzx-optimized-2026-10-03.json).

The complete codec suite, host tests, and denied-warning Clippy checks passed.
Regression tests cover short and long canonical codewords, exact bit consumption,
bulk literal decoding across frame boundaries, and both history ring spans.
An independent real-UUP comparison matched directory metadata for 27 CABs,
26 manifests, and all 94 selected extracted members against installed 7z;
all input hashes were unchanged. Evidence:
[lzx-optimized-uup-comparison-2026-10-03.json](fixtures/cab-reader/lzx-optimized-uup-comparison-2026-10-03.json).

## Original baseline

Host: Intel Core i7-13700K, Linux. References: p7zip 17.05 and cabextract 1.11.
These results describe the installed reference versions, not current 7-Zip releases.

| Selected member | Decoded folder prefix | Native (ms) | 7z (ms) | cabextract (ms) |
|---|---:|---:|---:|---:|
| Quantum real tiny | 0.000 MiB | 0.621 | 5.529 | 1.511 |
| Quantum solid 53 KiB | 0.051 MiB | 1.928 | 6.709 | 2.794 |
| Quantum solid 4 MiB | 4.000 MiB | 9.268 | 11.259 | 8.105 |
| MSZIP spanning | 0.048 MiB | 0.955 | 5.312 | 1.590 |
| LZX ISE manifest | 3.451 MiB | 32.514 | 18.407 | 14.932 |
| LZX Hello manifest | 48.389 MiB | 810.945 | 217.309 | 248.814 |
| LZX Hello payload | 44.924 MiB | 778.701 | 206.411 | 236.875 |
| LZX Speech manifest | 95.921 MiB | 1396.853 | 555.188 | 558.852 |

Seven measured runs per tool/case, interleaved in rotating order after one
byte-verified warm-up. Medians are elapsed wall times, including process startup,
CAB metadata parsing, decoding earlier solid-folder bytes, and streaming the
selected member to `/dev/null`. Every warm-up output matched in size and SHA-256.
All input CAB hashes matched before/after. Spanning archives were copied unchanged
to filenames matching the embedded case for p7zip; all three tools used those copies.

Native runs use the minimal `examples/cab_decode.rs` library adapter built in
release mode. It emits the member to stdout, like `7z e -so` and `cabextract -p`.
The production `cab-extract` command uses a temporary file and durable publication;
that extra file-publication cost is excluded for all tools. This does not measure
cold-cache reads, whole-archive bulk extraction, pure in-process codec speed, or
the complete 27-CAB inventory command. Small cases are substantially affected by
process startup. They do not establish superior decoder throughput.

“Decoded folder prefix” is the member offset plus size, including data that has
to be decompressed and discarded before the selected member. The decoder may
finish the containing frame as well. The Hello payload is 13,633,736 bytes but
requires about 44.9 MiB of solid-folder decoding. The 7,494-byte Hello manifest
requires about 48.4 MiB. MSZIP and Quantum small cases are retained/generated test
fixtures; the LZX cases are real UUP packages from the user’s directory.

The main measured performance gap in the original baseline is native LZX decoding. Large Quantum takes
about 14% longer than cabextract and 18% less time than installed p7zip in this
process-level case; startup prevents interpreting those as pure-codec ratios.

Raw samples, source/member hashes, release-binary hash, tool versions and
throughput estimates are retained in
[benchmark-2026-10-03.json](fixtures/cab-reader/benchmark-2026-10-03.json).

Reproduce with the same reference versions and input folder:

```sh
cargo build --release --locked --example cab_decode --bin windows-uup
python3 ../windows-uup/scripts/benchmark-cab.py \
  --binary target/release/examples/cab_decode \
  --cli target/release/windows-uup --cabextract cabextract \
  --uup-dir ../vmmanager-sh/19045.7727_amd64_en-us_professional_f2a3c168_convert/UUPs \
  --runs 7 --output /tmp/cab-benchmark.json
```

Use Cargo’s configured target directory if it differs. The script requires
identical bytes before collecting timings and refuses to overwrite the report.
