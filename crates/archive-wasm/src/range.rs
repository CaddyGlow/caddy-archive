//! Browser range orchestration and bounded cooperative decoder bindings.
use archive_core::{
    EntryId, Limits,
    incremental::{EntryDecoder, IndexPoll, RangeIndex},
};
use wasm_bindgen::prelude::*;

/// Sparse metadata index; source bytes are supplied only after asynchronous reads.
#[wasm_bindgen]
pub struct RangeArchive {
    index: RangeIndex,
}
#[wasm_bindgen]
impl RangeArchive {
    /// Length and budgets remain BigInt-compatible 64-bit values.
    #[wasm_bindgen(constructor)]
    pub fn new(
        length: u64,
        max_input: u64,
        max_metadata: u64,
        max_decoded: u64,
    ) -> Result<Self, JsValue> {
        let limits = Limits {
            max_input_bytes: max_input,
            max_metadata_bytes: max_metadata,
            max_entry_bytes: max_decoded,
            max_total_bytes: max_decoded,
            ..Limits::default()
        };
        Ok(Self {
            index: RangeIndex::new(length, limits).map_err(js_error)?,
        })
    }
    /// Return `ready` or an exact missing range, without invoking browser I/O.
    pub fn poll_json(&mut self) -> Result<String, JsValue> {
        let value = match self.index.poll().map_err(js_error)? {
            IndexPoll::Ready => serde_json::json!({"status":"ready"}),
            IndexPoll::NeedRange(range) => {
                serde_json::json!({"status":"range","offset":range.offset.to_string(),"length":range.length})
            }
        };
        serde_json::to_string(&value).map_err(|e| JsValue::from_str(&e.to_string()))
    }
    /// Supply one awaited File/Blob slice; no whole-archive copy occurs.
    pub fn supply(&mut self, offset: u64, bytes: &[u8]) -> Result<(), JsValue> {
        self.index.supply(offset, bytes).map_err(js_error)
    }
    /// Indexed metadata is available only after the ready state.
    pub fn entries_json(&self) -> Result<String, JsValue> {
        serde_json::to_string(self.index.entries().map_err(js_error)?)
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }
    /// Compressed-data range with decimal-string offsets for lossless JS parsing.
    pub fn entry_range_json(&self, id: usize) -> Result<String, JsValue> {
        let range = self.index.entry_range(EntryId(id)).map_err(js_error)?;
        Ok(serde_json::json!({"offset":range.offset.to_string(),"compressed_bytes":range.compressed_bytes.to_string(),"decoded_bytes":range.decoded_bytes.to_string(),"method":range.method}).to_string())
    }
    /// Start a cooperative decoder without retaining source bytes.
    pub fn decoder(&self, id: usize) -> Result<StepDecoder, JsValue> {
        Ok(StepDecoder {
            decoder: self.index.decoder(EntryId(id)).map_err(js_error)?,
        })
    }
    /// Metadata cache is zero once the index is complete.
    pub fn cached_bytes(&self) -> u64 {
        self.index.cached_bytes()
    }
}
fn js_error(error: archive_core::Error) -> JsValue {
    JsValue::from_str(&error.to_string())
}

/// Bounded stateful entry decoder, serviced between individual steps.
#[wasm_bindgen]
pub struct StepDecoder {
    decoder: EntryDecoder,
}
#[wasm_bindgen]
impl StepDecoder {
    /// Process at most 64KiB of input and decoded output; output is provisional.
    pub fn step(
        &mut self,
        input: &[u8],
        max_output: usize,
        eof: bool,
    ) -> Result<DecoderStep, JsValue> {
        let step = self
            .decoder
            .step(input, max_output, eof)
            .map_err(js_error)?;
        Ok(DecoderStep {
            consumed: step.consumed,
            verified: step.verified,
            done: step.done,
            output: step.output,
        })
    }
    /// Stop before the next decoding step independently of progress rendering.
    pub fn cancel(&mut self) {
        self.decoder.cancel();
    }
    /// Exact cumulative decoded work.
    pub fn decoded_bytes(&self) -> u64 {
        self.decoder.decoded_bytes()
    }
}
/// One bounded provisional output chunk and final verification flag.
#[wasm_bindgen]
pub struct DecoderStep {
    pub consumed: usize,
    pub verified: bool,
    pub done: bool,
    output: Vec<u8>,
}
#[wasm_bindgen]
impl DecoderStep {
    /// Move the chunk out; a second call returns an empty array.
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.output)
    }
}
