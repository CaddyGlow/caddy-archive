# Cooperative File/Blob range worker

`range-worker.js` imports the generated wasm-bindgen module URL supplied in `init`.
The caller supplies a `Blob`/`File` and explicit input, metadata, and decoded limits.
ZIP metadata indexing awaits exact source ranges. Rust never invokes a synchronous
browser callback to fetch bytes. The sparse metadata cache is released after the
index succeeds; payloads are fetched in at most 64-KiB slices.

Each decoder step consumes/produces at most 64 KiB. A worker yields between steps,
allowing cancellation during a huge entry. Cancellation is independent of progress.
Chunks are provisional: publish only after a `complete` message with `verified`.
The worker waits for an `ack` after every transferred output chunk, which bounds
pending output even when consumers are slow. `cancel` releases a pending ack wait
and prevents successful completion. Error/disconnect consumers must cancel and then
terminate the worker if its caller has been discarded.

Protocol: `init` (`moduleUrl`, `wasmUrl`), `open` (`operation`, `archive`, `blob`,
`limits`), `extract` (`operation`, `archive`, `entry`), `ack`/`cancel` (`operation`),
and `close` (`archive`). Responses are `ready`, `index`, `chunk`, `complete`, or
`error`. Only one extraction with a given operation ID may be active.

Initial cooperative profile: regular, unencrypted stored/DEFLATE ZIP entries,
including containers whose metadata index is supported by archive-core. This does
not by itself validate APPX block maps. TAR/CAB/7z/XZ/package readers currently retain
their separate synchronous profiles and explicitly reject this incremental entry
contract. Their synchronous small-input APIs must not be described as bounded
cooperative large-file operations. Encrypted cooperative streams need their own
incremental authentication and publication contract.

Memory consists of bounded metadata ranges during indexing, the decoder workspace,
one JS input slice, at most one decoded chunk in Rust/JS transfer, and caller-owned
provisional output. No duplicate whole archive or whole entry is held by this helper.
Blob indexing rejects offsets beyond JavaScript's exact-integer range; Rust keeps
offsets and cumulative sizes as u64 and JS boundary values as BigInt/decimal strings.
