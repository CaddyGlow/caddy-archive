// File/Blob I/O is awaited outside Rust. Each decoding turn is bounded to 64KiB.
const CHUNK_BYTES = 65536;
const turn = () => new Promise(resolve => setTimeout(resolve, 0));
const check = signal => {
  if (signal?.aborted) throw new DOMException("Operation cancelled", "AbortError");
};
const offsetNumber = value => {
  const offset = BigInt(value);
  if (offset > BigInt(Number.MAX_SAFE_INTEGER)) throw new RangeError("Blob offset exceeds exact browser indexing");
  return Number(offset);
};

export async function openBlobArchive(wasm, blob, limits, { signal } = {}) {
  const index = new wasm.RangeArchive(BigInt(blob.size), BigInt(limits.maxInput), BigInt(limits.maxMetadata), BigInt(limits.maxDecoded));
  try {
    while (true) {
      check(signal);
      const request = JSON.parse(index.poll_json());
      if (request.status === "ready") return { index, blob, entries: JSON.parse(index.entries_json()) };
      const offset = offsetNumber(request.offset);
      const bytes = new Uint8Array(await blob.slice(offset, offset + request.length).arrayBuffer());
      check(signal);
      index.supply(BigInt(request.offset), bytes);
      await turn();
    }
  } catch (error) {
    index.free();
    throw error;
  }
}

// onChunk must accept provisional bytes. Commit output only after this resolves.
// Awaiting the sink provides backpressure; there is never an unbounded chunk queue.
export async function extractBlobEntry(archive, id, { signal, onChunk, onProgress } = {}) {
  if (typeof onChunk !== "function") throw new TypeError("A provisional output sink is required");
  const range = JSON.parse(archive.index.entry_range_json(id));
  const end = BigInt(range.offset) + BigInt(range.compressed_bytes);
  let position = BigInt(range.offset);
  let pending = new Uint8Array(0);
  const decoder = archive.index.decoder(id);
  let decoded = 0n;
  try {
    while (true) {
      check(signal);
      if (!pending.length && position < end) {
        const count = Number(end - position > BigInt(CHUNK_BYTES) ? BigInt(CHUNK_BYTES) : end - position);
        pending = new Uint8Array(await archive.blob.slice(offsetNumber(position), offsetNumber(position) + count).arrayBuffer());
        if (pending.length !== count) throw new Error("Source changed or range was truncated");
      }
      const step = decoder.step(pending, CHUNK_BYTES, position + BigInt(pending.length) === end);
      const consumed = step.consumed;
      const done = step.done;
      const verified = step.verified;
      const output = step.take_output();
      step.free();
      position += BigInt(consumed);
      pending = pending.subarray(consumed);
      decoded += BigInt(output.length);
      if (output.length) await onChunk(output);
      check(signal);
      onProgress?.({ decodedBytes: decoded, verified, terminal: done });
      if (done) {
        if (!verified) throw new Error("Decoder ended without verification");
        return { decodedBytes: decoded, verified: true };
      }
      await turn();
    }
  } catch (error) {
    decoder.cancel();
    throw error;
  } finally {
    decoder.free();
  }
}

// Optional dedicated Worker protocol. A chunk requires an acknowledgement before
// the next chunk is decoded, bounding transferred output even with a slow client.
if (typeof WorkerGlobalScope !== "undefined" && self instanceof WorkerGlobalScope) {
  const archives = new Map();
  const operations = new Map();
  const acknowledgements = new Map();
  let wasm;
  self.onmessage = async ({ data }) => {
    if (data.type === "cancel") {
      operations.get(data.operation)?.abort();
      acknowledgements.get(data.operation)?.();
      return;
    }
    if (data.type === "ack") {
      acknowledgements.get(data.operation)?.();
      acknowledgements.delete(data.operation);
      return;
    }
    try {
      if (data.type === "init") {
        wasm = await import(data.moduleUrl);
        await wasm.default(data.wasmUrl);
        self.postMessage({ type: "ready" });
      } else if (data.type === "open") {
        const controller = new AbortController();
        operations.set(data.operation, controller);
        const archive = await openBlobArchive(wasm, data.blob, data.limits, { signal: controller.signal });
        archives.set(data.archive, archive);
        operations.delete(data.operation);
        self.postMessage({ type: "index", operation: data.operation, archive: data.archive, entries: archive.entries });
      } else if (data.type === "extract") {
        const controller = new AbortController();
        operations.set(data.operation, controller);
        const result = await extractBlobEntry(archives.get(data.archive), data.entry, {
          signal: controller.signal,
          onChunk: bytes => new Promise(resolve => {
            acknowledgements.set(data.operation, resolve);
            self.postMessage({ type: "chunk", operation: data.operation, bytes }, [bytes.buffer]);
          }),
        });
        operations.delete(data.operation);
        self.postMessage({ type: "complete", operation: data.operation, decodedBytes: result.decodedBytes.toString(), verified: result.verified });
      } else if (data.type === "close") {
        archives.get(data.archive)?.index.free();
        archives.delete(data.archive);
      }
    } catch (error) {
      operations.delete(data.operation);
      acknowledgements.delete(data.operation);
      self.postMessage({ type: "error", operation: data.operation, error: String(error) });
    }
  };
}
