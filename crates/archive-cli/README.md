# arc

Native archive and read-only Windows package operations:

```text
arc list input.zip --json
arc test input.zip
arc extract input.zip --output destination
arc create --format zip --input source-directory --output new.zip
arc create --input source-directory --output new.tar.gz
arc create --format zip --encrypt --password-file password-source --input source-directory --output encrypted.zip
arc list install.wim --image 1
arc list disc.iso --view udf
arc list - --format tar --json
arc create --format tar --input source-directory --output -
```

All options retain their long names and accept these short forms:

| Short | Option | Short | Option |
| --- | --- | --- | --- |
| `-f` | `--format` | `-i` | `--input` |
| `-o` | `--output` | `-t` | `--threads` |
| `-c` | `--compression` | `-e` | `--encrypt` |
| `-H` | `--encrypt-headers` | `-z` | `--zip-encryption` |
| `-j` | `--json` | `-p` | `--password-file` |
| `-b` | `--bundle-entry` | `-m` | `--media` |
| `-I` | `--image` | `-n` | `--image-name` |
| `-v` | `--view` | `-M` | `--max-input-bytes` |
| `-P` | `--progress` | `-h` | `--help` |
| `-s` | `--archive-folder` (extract) | `-d` | `--output` (extract) |

For example: `arc create -i source-directory -o new.zip` and
`arc extract new.zip -o destination -t 4`. `-V` prints the version.

Commands accept the familiar aliases `a` (create), `l` (list), `t` (test), and
`x` (extract). Extraction defaults to the current directory. Attached output
arguments (`-odestination`) and unzip-style `-d destination` are supported.
Use `-s` / `--archive-folder` to append the archive's name to the output directory,
with its extension removed, including compound suffixes such as `.tar.gz`.
Alternatively, a `*` path component in the output is replaced by that name,
as in 7z. Quote wildcard arguments to prevent shell expansion:

```sh
arc a -i source-directory -o backup.zip
arc l backup.zip
arc t backup.zip
arc x backup.zip -s                 # ./backup/
arc x backup.tar.gz '-o*'           # ./backup/
arc x backup.zip -o destination -s  # destination/backup/
arc x backup.zip '-odestination/*'  # destination/backup/
arc x backup.zip -d destination     # destination/
```

Archive-named folders require a filename; extraction from stdin uses `-o DIR`.

Leading tar-style compact options are supported, with or without a dash:
`c` creates, `x` extracts, `t` lists, `f` takes the archive filename, and `v`
prints entry names to stderr. `z` selects gzip TAR, `J` selects XZ TAR, `j`
selects BZip2 TAR, and `a` infers creation compression from the output extension.
Creation takes one source directory after the archive filename. Extraction
accepts `-C DIR` as another output-directory spelling; existing arc options such
as `-s` and `-j` can follow the tar arguments.

```sh
arc xzvf backup.tar.gz -C destination
arc -xJvf backup.tar.xz -s
arc czvf backup.tar.gz source-directory
arc cavf backup.tar.xz source-directory
arc tf backup.tar
```

Creation and extraction preserve modification times and ordinary file and
directory permissions for ZIP, 7z, TAR, and compressed TAR. ZIP also retains
stored local DOS dates when Unix timestamp fields are absent. Gzip stores the
modification time; CAB stores a local DOS timestamp and read-only status.
APPX/MSIX extraction restores its ZIP metadata, and WIM extraction restores
available modification times and read-only status. Directory metadata is applied
after all children are written. TAR owner/group identifiers are stored during
creation; extraction restores only ordinary read/write/execute permission bits.
Ownership, ACLs, extended attributes, and special permission bits are not restored.
This CLI currently preserves timestamps at whole-second precision (two seconds
for CAB/DOS timestamps); TAR PAX timestamp overrides are not yet applied.

Passwords are read from the explicitly selected file, with one terminal newline
removed. No password-valued argument exists. The CLI clears its password buffer on
drop; this does not clear the source file or copies retained by format backends.
Native encryption randomness comes from the operating system. AES-256 is the ZIP
creation default; `--zip-encryption zipcrypto` explicitly selects legacy compatibility.

`--max-input-bytes` defaults to 1 GiB. UDF and WIM/ESD use seekable source
reads and stream extracted file data. WIM/ESD listing without a selector reports container
images; `--image` is one-based and `--image-name` selects an exact XML name.
Testing/extraction require one selector, and the two selectors conflict.
APPX/MSIX bundle listing reports declared nested-package identities. Testing or
extracting requires `--bundle-entry EXACT_FILENAME`; no architecture is selected
implicitly. Selected package block hashes are verified, not signer trust.
MSI external cabinets and loose files require explicit repeated
`--media NAME=PATH` mappings. Names match resolver requests exactly; the CLI never
searches the installer directory automatically. Mapped reads are bounded by the
operation input limit, and MSI files remain provisional until all files succeed.
List, test, and extraction detect input signatures by default, including renamed
WIM and MSI inputs. Compressed TAR inside gzip, XZ, and BZip2 is detected too.
Raw LZMA, Brotli, and DEFLATE have no reliable signature: recognized `.lzma`,
`.br`, `.tar.br`, and `.deflate` suffixes supply a fallback interpretation;
otherwise specify `--format`. An explicit format overrides automatic detection.
Creation infers its format from the output suffix when `--format` is omitted.
Suffix matching is case-insensitive and recognizes compound extensions such as
`.tar.gz`, `.tar.xz`, `.tar.bz2`, and `.tar.br`, plus `.tgz`, `.txz`, and `.tbz2`.
Unknown suffixes and stdout require an explicit format.
Archive creation accepts `-c` / `--compression` according to its format:

| Format | Codecs | Default |
| --- | --- | --- |
| 7z | copy, deflate, lzma, lzma2, bzip2, brotli | lzma2 |
| ZIP | copy, deflate | deflate |
| CAB | copy, mszip, lzx, quantum | mszip |

Incompatible choices are rejected. WIM remains read-only through `arc`.
Explicit archive codec selection requires an output file. Raw DEFLATE
can also be selected explicitly with `--format deflate`.
`arc deflate --input INPUT --output OUTPUT` and `arc inflate --input INPUT
--output OUTPUT` infer compression from `.deflate`, `.gz`, or `.zlib` output
suffixes. Inflate detects gzip/zlib headers and falls back to raw DEFLATE;
`--format deflate|gzip|zlib` explicitly selects a wrapper. Either path can be `-`
for forward-only stdin or binary stdout; compression to stdout needs `--format`. File
outputs are published only after successful completion, without overwriting.
Raw DEFLATE has no integrity checksum; stdout cannot be rolled back on failure.
`--format` selects an explicit interpretation for list, test, and extraction.
TAR stdin is detected automatically and read forward-only; indexed formats require seekable input. Streaming
creation profiles support binary stdout, which cannot be combined with JSON.
`--view udf` explicitly chooses the UDF filesystem of a hybrid optical image;
the default ISO reader does not silently choose a different filesystem.

Extraction rejects traversal, ambiguous platform names, links, special files,
duplicate normalized paths, and existing destination files. Provisional regular
files from ordinary archives are staged until the entire selected extraction batch
verifies. Ordinary archives, stdin TAR, and APPX/MSIX/MSI batches use one anonymous
spool on the destination filesystem, then copy and publish each verified file in
turn. Open file handles stay bounded as entry counts grow. This adds a disk copy
and requires space for the full decoded spool alongside the published output.
Unix operations use retained
directory handles and no-follow lookups. Windows operations retain directory handles
that deny rename/deletion and reject reparse points. Ctrl-C requests cancellation
independently of progress rendering. Regular outputs stay provisional until all
required entry reads and verification succeed. Already verified outputs may remain
if a later output publication fails. Creation never replaces an existing archive.

APPX/MSIX validation checks package metadata and block-map hashes separately from
signature verification. MSI external media is denied unless explicit `--media`
mappings or a library resolver are supplied. Package creation and signing are unavailable.

JSON output uses `schema_version: 1` and `ok`. Success records contain `operation`
and operation-specific fields. Failure records contain `error.code` and
`error.message`. Diagnostic output and progress are separate from stdout.

Exit codes: 0 success; 1 I/O or operation failure; 2 argument error; 3 unsupported
core operation; 4 malformed input or failed integrity; 5 resource limit;
6 password required; 130 cancellation. Format extraction I/O wrappers can retain
code 1 while their message identifies the underlying integrity failure.

Optional `progress` enables indicatif rendering on stderr. `--progress auto`
enables it only for interactive, non-JSON operations; `never` disables it;
`always` requires the compiled feature. `parallel` is independent of rendering.
The renderer consumes a bounded coalescing slot at 10 Hz outside codec execution.
Observed selected-output bytes are separate from physical reads and shared decoder
work; unavailable counters are explicitly marked unknown.

Run the reporting benchmark with:

```text
cargo run --release -p caddy-archive-cli --example progress-bench
```

It compares the direct, compiled-disabled, and coalesced observer paths on identical
32 MiB DEFLATE payloads with rotated ordering and reports every sample. Terminal
rendering and browser overhead are separate measurements. A 20-sample run on this
host measured medians of 112.794 ms direct, 112.730 ms disabled, and 112.989 ms
coalesced snapshots (0.172% enabled overhead). Concurrent system activity produced
outliers up to 165/198/257 ms, respectively; this result establishes one native
workload rather than a universal overhead guarantee.

`--features progress` adds the exact terminal adapter to the benchmark. For a real
terminal measurement, run it under a PTY:

```text
cargo build --release -p caddy-archive-cli --features progress --example progress-bench
script -q -c 'env ARC_BENCH_MIB=128 ARC_BENCH_ROUNDS=6 cargo run --release -p caddy-archive-cli --features progress --example progress-bench' /tmp/arc-progress.log
```

The 128 MiB run showed intermediate byte counters at the configured cadence.
Its six-sample medians were 469.90 ms direct, 464.25 ms disabled, 456.39 ms
coalesced, and 465.91 ms terminal rendering. Samples ranged from 440 to 558 ms;
these differences are within the observed shared-host variation. Both runs used
Linux x86-64 on an Intel Core i7-13700K (24 logical CPUs), with no CPU pinning.

`compress` and `decompress` operate on a single file, including raw Windows blocks:

```text
arc compress -i payload -o payload.lzma
arc decompress -i payload.lzma -o restored
arc compress -f lzma2 -i payload -o payload.lzma2 --dictionary-bytes 8388608
arc decompress -f lzma2 -i payload.lzma2 -o restored --dictionary-bytes 8388608
arc compress -f xpress -i payload -o payload.xpress
arc decompress -f xpress -i payload.xpress -o restored --output-size 4096
arc create -i source-directory -o backup.cab -c lzx
arc create -i source-directory -o backup.7z -c deflate
```

Single-file codecs: deflate, gzip, zlib, lzma, lzma2, xz, bzip2, brotli,
xpress (Huffman/WIM), xpress-plain, lzx (WIM), lzms, lznt1, and quantum.
Compression infers its codec from the output suffix. Decompression checks gzip,
zlib, XZ, and BZip2 signatures first, then uses the input suffix; `-f` overrides
both. A filename or explicit format is needed for signatureless codecs.
Raw LZMA2 requires matching `--dictionary-bytes` (default 8388608), and raw
LZX/Quantum require matching `--window-order` (default 15). All Windows block
codecs require `--output-size` on decompression. XPRESS Huffman is limited to
65536 input bytes per file, Quantum to 32768, and LZX to its window size.
These files contain one raw block, without custom framing, checksums, names,
or filesystem metadata. Non-DEFLATE codecs use bounded input buffering; large
files can exceed the workspace limit. Both paths accept `-` for stdin/stdout;
stdout compression requires `-f` and cannot combine with JSON.

Resource limits apply through global `--max-input-bytes`, `--max-entry-bytes`,
`--max-total-bytes`, and `--max-entries` options (integer bytes/counts). For
example, `arc --max-entry-bytes 67108864 --max-total-bytes 268435456
--max-entries 1000 extract upload.zip --output out` limits entries to 64 MiB
and total decoded output to 256 MiB. Defaults are 1 GiB input, 8 GiB per entry,
32 GiB output and 100,000 entries.

Codec memory follows 7-Zip's `memuse` policy. The default `--memuse=auto`
uses 80% of detected physical RAM for creation/compression and 17/32
(53.125%) for reading/decompression. `--memuse=p80` or `--memuse=80%`
selects a percentage; `--memuse=512m` selects a fixed budget. Integer bytes
and binary `b`, `k`, `m`, `g`, `t` suffixes are accepted. The spellings
`--mmemuse=512m` and `-mmemuse=512m` are also accepted. When native RAM
queries fail, the default is 2 GiB on 64-bit hosts or 1 GiB on 32-bit hosts.
These are codec workspace budgets, not process-wide RSS limits. Explicit
`--max-codec-workspace-bytes` and `--max-dictionary-bytes` can impose tighter
byte ceilings; there is no separate fixed 64 MiB dictionary ceiling by default.

Extraction requests the number of workers reported by the OS by default.
`--threads N` selects a worker upper bound; codecs, archive structure, and
memory admission can reduce the actual worker count. TAR, TAR.gz, TAR.xz,
ZIP (including encrypted ZIP), 7z, and CAB creation open one source payload at a
time instead of retaining every file in memory.

Compressed TAR uses temporary-file storage bounded by decoded output limits.
Library callers choosing in-memory indexing additionally have a 256 MiB
`Limits::max_buffered_bytes` cap. Neither is a process-wide memory limit.
Package members, raw block codecs and some creation paths still buffer data;
see the core README's resource and memory notes.
