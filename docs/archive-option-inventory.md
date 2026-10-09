# Option compatibility inventory

The canonical machine-readable seed inventory lives in
[`archive_core::compatibility`](../crates/archive-core/src/compatibility.rs).
`inventory()` returns serializable schema version 1 without allocating registry
rows. Its reference pin matches the [options/update plan](archive-options-update-plan.md).
It covers every roadmap area and current format family; it is not an exhaustive
inventory of upstream properties or proof of command compatibility.

Native spellings and native scoped behavior have separate fields from future
`arc 7z` spellings and status. The `frontend.read-subset` row documents the implemented read-only namespace.
Other compatibility rows remain `planned` or `unavailable`; native selection
support does not establish full upstream selector semantics. In particular, native `a` still means create-only, native `-t`
means threads, native `-m` supplies MSI media, and native `-p` selects a password
file. These spellings must not acquire upstream semantics accidentally.

The statuses are `implemented`, `accepted-no-effect`, `planned`, `extension`,
and `unavailable`. No row claims an accepted upstream no-op before its specific
handler and pinned executable behavior have been verified. An implemented native
row describes its stated profile with required features enabled; it does not
assert all format variants, browser exposure, or active-build feature availability.
The feature prerequisites describe native profiles and must be combined with
runtime capability queries. Optional submethods can require additional features.

Each row records grammar, units/range, a native default policy, source reference,
platform limits, existing regression suites, and planned acceptance requirements.
`reference_default: null` means the upstream default remains unresolved, rather
than inheriting a native default or guessing a value. Aggregate rows deliberately
leave detailed upstream property ranges open until the handler inventory expands.
Existing test references identify native evidence; planned requirements are not
claims that those tests already exist or have passed.

The source pin was checked against upstream
[command-line parsing](https://github.com/ip7z/7zip/blob/9128b80e3a471108678e2b3ec3c985861c8d7b0f/CPP/7zip/UI/Common/ArchiveCommandLine.cpp)
and [common handler properties](https://github.com/ip7z/7zip/blob/9128b80e3a471108678e2b3ec3c985861c8d7b0f/CPP/7zip/Archive/Common/HandlerOut.cpp).
Handler references provide starting points for further inventory work; they do
not establish encoder availability in a particular executable build.

P0 remains open until every aggregate property family has individual rows with
resolved defaults, grammar/ranges, effects and concrete differential cases.
The pinned executable version, build configuration, SHA-256 and provenance also
remain unverified (`reference_executable_verified: false`). An installed `7z`
with another version can produce observations but cannot close the pinned gate.
Follow-on milestones must update this registry before advertising their behavior.

The implemented read subset accepts `arc 7z l`, `t`, and `x`, attached format/output
switches, immediate include/exclude patterns, explicit literal mode, ASCII case
policy, and switch termination. It retains native no-overwrite publication and
resource limits. Native wildcards are component-local byte patterns; this is an
explicit compatibility difference. Compatibility-namespace editing, flat extraction, method
properties, inline passwords/prompts and CLI listfiles fail before output.
The portable selection API also exposes strict bounded UTF-8 listfile decoding;
that helper is not wired into the CLI and does not enable upstream encodings.

The native JSON extension accepts either `arc --json 7z l archive.zip` or
`arc 7z l archive.zip --json` (also `-j`). It preserves the native JSON schema
and error codes. This does not add arbitrary native global switches before the
namespace. Unsupported frontend errors use constant messages, so inline password
and unknown-option values are not included in diagnostics or JSON error records.

Native `arc edit` adds separately scoped ZIP/7z timestamp and encryption rows.
These do not enable upstream editing command grammar. ZIP entry passwords and
7z whole compression-group/header encryption are distinct capabilities; solid
subsets and partial encrypted-header rekeying remain explicitly unsupported.
