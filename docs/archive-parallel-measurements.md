# 7z Parallel Measurements

Development snapshot on 2026-10-05, Linux 6.18.53 x86_64, Intel i7-13700K
(24 logical CPUs), p7zip 17.05. This is a small warm-cache workload, not a
throughput guarantee. Three fresh executable processes per worker count used
the release example `archive-core/examples/parallel_measure.rs`. No output
payload is retained; its sink counts bytes. Linux `VmHWM` measures process peak
resident memory, including baseline/library/thread costs, not exact decoder
allocation or a portable allocator statistic. First-byte time starts after
indexing and includes worker startup and scheduling. Index time was 0.073-0.105
ms in these runs. Other concurrent workspace work can affect timing.

Each corpus contains four independent LZMA2 folders, four 8 MiB files (32 MiB
decoded). Every run reported actual workers 1/2/4, four folder tasks, exactly
33,554,432 decoded and delivered bytes, verified success, and no fallback.

| Corpus | Workers | First Byte Range (ms) | Extraction Range (ms) | Peak RSS Range (KiB) |
| --- | --- | --- | --- | --- |
| Zero-filled, `-mx=1` | 1 | 0.088-0.095 | 11.012-11.113 | 3204-3268 |
| Zero-filled, `-mx=1` | 2 | 0.220-0.256 | 6.755-7.098 | 3544-3748 |
| Zero-filled, `-mx=1` | 4 | 0.252-0.262 | 3.679-3.782 | 3860-3904 |
| Random, explicit 8 MiB dictionary | 1 | 0.077-0.089 | 13.229-17.962 | 11288-11424 |
| Random, explicit 8 MiB dictionary | 2 | 0.237-0.284 | 14.295-16.717 | 20072-20384 |
| Random, explicit 8 MiB dictionary | 4 | 0.206-0.417 | 12.240-17.206 | 33888-36580 |

The random corpus shows the expected increasing actual resident workspace;
additional workers do not show a stable timing improvement for that workload.
The zero corpus is unusually compressible. Neither sample validates worst-case
memory for every coder graph or makes a general performance claim.

## Reproduction

Create four files `0.bin` through `3.bin` using eight 1 MiB blocks each from
`/dev/zero` or `/dev/urandom`. The random corpus is intentionally not bitwise
reproducible; keep its archive hash when comparing runs. Build the archives in
their input directory:

```sh
7z a -t7z -m0=LZMA2 -mx=1 -ms=off measure.7z 0.bin 1.bin 2.bin 3.bin
7z a -t7z -m0=LZMA2:d=8m -mx=1 -ms=off random.7z 0.bin 1.bin 2.bin 3.bin
cargo build --release --locked -p archive-core --example parallel_measure --features sevenz,parallel
```

Run the built example with `ARCHIVE_PATH WORKERS` separately for workers 1, 2,
and 4, repeating at least three times. Locate it under Cargo's configured target
directory. Keep other machine load and build/profile settings comparable.
Snapshot archive SHA-256 values:

- Zero corpus: `70a5ae763a4940085c1d0e9e15e832fee1f30110594122205d8f434c271c861b`
- Random corpus: `f28c994dad807dabc2763e6cea8857a8dea63bd4823585adf355972565548201`

## Correctness Gates

`sevenz_parallel_interop.rs` independently generates BCJ + LZMA2 archives with
AES payload/header encryption using p7zip, both solid and non-solid. The ignored
test was explicitly executed and passed at worker requests 1/2/4: independent
folders use the requested worker count; one solid folder reports one worker
with a solid fallback reason; exact payload bytes and decoded work agree.

```sh
cargo test --locked -p archive-core --all-features --test sevenz_parallel_interop -- --ignored
```

Existing backend regressions separately cover cancellation, consumer failure,
worker failure and aggregate workspace/pending-output limits. The new test and
example pass all-target/all-feature Clippy with warnings denied. Windows timing,
larger corpora, broader filter/encryption combinations and exact peak allocator
tracking remain unmeasured here.
