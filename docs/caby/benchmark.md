# caby benchmark and validation

Recorded on 2026-10-04 on x86_64 Linux, Intel Core i7-13700K, using Rust 1.96.0
and the workspace release profile (thin LTO). This is a single local run on a
shared development machine, not a performance guarantee.

```sh
cargo bench -p caby --bench cab --locked -- --iterations 5 --size 1048576 --csv "$PWD/docs/caby/benchmark-20261004.csv"
```

Each corpus contains 1 MiB of deterministic bytes. Each method has one warm-up
and verified round trip, followed by five timed writes and five timed reads.
Writer codec initialization and header finalization are included. The output
buffer retains capacity between writes. Readers stream into a sink. Corpus
creation, equality checks, filesystem I/O, and external-reader extraction are
excluded from timing. Throughput uses uncompressed input bytes; ratio is CAB
bytes divided by input bytes, including headers. Allocation is not measured.

| Corpus | Method | CAB bytes | Ratio | Write MiB/s | Read MiB/s |
| --- | --- | ---: | ---: | ---: | ---: |
| repetitive | stored | 1048904 | 1.0003 | 16636.17 | 15815.23 |
| repetitive | mszip | 5800 | 0.0055 | 626.31 | 2312.32 |
| repetitive | lzx-15 | 15626 | 0.0149 | 301.37 | 2579.76 |
| repetitive | lzx-21 | 18186 | 0.0173 | 295.96 | 2012.18 |
| repetitive | quantum-1-18 | 2614 | 0.0025 | 76.04 | 957.13 |
| repetitive | quantum-7-18 | 2614 | 0.0025 | 76.03 | 959.66 |
| mixed | stored | 1048904 | 1.0003 | 16648.46 | 15980.52 |
| mixed | mszip | 532255 | 0.5076 | 133.87 | 586.63 |
| mixed | lzx-15 | 542752 | 0.5176 | 86.83 | 512.01 |
| mixed | lzx-21 | 545312 | 0.5201 | 86.94 | 486.85 |
| mixed | quantum-1-18 | 533770 | 0.5090 | 10.92 | 21.28 |
| mixed | quantum-7-18 | 533770 | 0.5090 | 9.24 | 21.34 |
| random | stored | 1048904 | 1.0003 | 17208.33 | 15809.33 |
| random | mszip | 1049288 | 1.0007 | 67.13 | 4291.93 |
| random | lzx-15 | 1062250 | 1.0130 | 61.64 | 288.25 |
| random | lzx-21 | 1064810 | 1.0155 | 61.23 | 279.57 |
| random | quantum-1-18 | 1059316 | 1.0102 | 5.17 | 10.65 |
| random | quantum-7-18 | 1059316 | 1.0102 | 5.15 | 10.72 |

The full machine-readable results are in [benchmark-20261004.csv](benchmark-20261004.csv).
Synthetic repetitive data strongly favors LZ matches; random data generally
expands once compression metadata is included. Real package performance can
be measured with `--input /absolute/path/to/payload`.

## Correctness gates

`CABY_REQUIRE_7Z=1 cargo test -p caby --locked` requires the independent reader
and passes the reader regression suite, writer round trips, and public-API doc
example. The writer suite checks all supported LZX windows, all Quantum windows
and levels, cross-frame members, empty inputs/members, UTF-8 names, invalid
names/configurations, duplicate names, output failures, embedded archive offsets,
and actual compression of repetitive inputs.

7-Zip/p7zip 17.05 independently extracts stored, MSZIP, LZX, and Quantum output,
including multiframe/random data and Quantum long-distance matches. These host
checks do not establish Windows FDI interoperability. Reader spanning support
is retained; the writer creates one unsigned cabinet and does not emit spanning
sets or preserve caller-selected filesystem metadata yet.
