//! Cooperative bounded entry decoding and asynchronous-orchestrated ZIP range indexing.
//!
//! No browser callback is invoked by synchronous Rust I/O. A caller supplies each
//! requested range after its own asynchronous read, and invokes one bounded step.
use crate::{Entry, EntryId, Error, Limits, Result};
use ms_compress::zlib::{Inflate, InflateFlush, Status, crc32::crc32};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    io::{Read, Seek},
    rc::Rc,
};

#[cfg(feature = "zip")]
use std::io::{self, SeekFrom};

/// Maximum input consumed or output produced in one cooperative step.
pub const STEP_BYTES: usize = 65536;

/// Byte range containing a decoded entry's compressed representation.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EntryRange {
    /// Stable archive entry ID.
    pub id: EntryId,
    /// Absolute compressed-data offset.
    pub offset: u64,
    /// Compressed bytes to supply, excluding any descriptor.
    pub compressed_bytes: u64,
    /// Expected decoded size.
    pub decoded_bytes: u64,
    /// ZIP compression method: zero or eight in this profile.
    pub method: u16,
    /// Decoded CRC32 from the archive index.
    pub crc32: u32,
}

impl<R: Read + Seek> crate::Archive<R> {
    /// Obtain a cooperative decoder and immutable compressed range without reading payloads.
    /// Initial profile: regular unencrypted ZIP stored/DEFLATE entries. Other profiles
    /// return an explicit unsupported error instead of buffering an entry implicitly.
    pub fn incremental_entry(&self, id: EntryId) -> Result<(EntryRange, EntryDecoder)> {
        let entry = self
            .entries
            .get(id.0)
            .ok_or_else(|| Error::Malformed("unknown incremental entry ID".into()))?;
        #[cfg(feature = "zip")]
        if let crate::Backend::Zip(_, locations) = &self.backend {
            let location = &locations[id.0];
            if entry.kind != crate::EntryKind::File || location.encrypted {
                return Err(Error::Unsupported(
                    "incremental encrypted or non-file ZIP entry".into(),
                ));
            }
            let range = EntryRange {
                id,
                offset: location.offset,
                compressed_bytes: location.compressed,
                decoded_bytes: entry.size,
                method: location.method,
                crc32: location.crc,
            };
            let decoder = EntryDecoder::new(range.clone(), self.limits)?;
            return Ok((range, decoder));
        }
        let _ = entry;
        Err(Error::Unsupported(
            "this backend has no cooperative incremental profile yet".into(),
        ))
    }
}

/// Provisional bytes and final verification status for one decoder step.
pub struct Step {
    /// Input prefix consumed; retain any remaining bytes for the next call.
    pub consumed: usize,
    /// At most STEP_BYTES provisional decoded bytes.
    pub output: Vec<u8>,
    /// True only after sizes and CRC have passed.
    pub verified: bool,
    /// Whether this operation reached a terminal state.
    pub done: bool,
}

enum Codec {
    Stored,
    Deflate(Box<Inflate>),
}
/// Stateful decoder that never allocates or processes a whole entry in one step.
pub struct EntryDecoder {
    range: EntryRange,
    codec: Codec,
    compressed: u64,
    decoded: u64,
    crc: u32,
    cancelled: bool,
    done: bool,
    failed: bool,
}
impl EntryDecoder {
    /// Create a stored/DEFLATE decoder with explicit resource policy.
    pub fn new(range: EntryRange, limits: Limits) -> Result<Self> {
        if range.decoded_bytes > limits.max_entry_bytes
            || range.decoded_bytes > limits.max_total_bytes
        {
            return Err(Error::ResourceLimit("incremental decoded bytes"));
        }
        if range.compressed_bytes > limits.max_input_bytes {
            return Err(Error::ResourceLimit("incremental compressed bytes"));
        }
        if range.method == 8 && limits.max_dictionary_bytes < 32768 {
            return Err(Error::ResourceLimit("DEFLATE dictionary"));
        }
        let codec = match range.method {
            0 => Codec::Stored,
            8 => Codec::Deflate(Box::new(Inflate::new(false, 15))),
            _ => return Err(Error::Unsupported("incremental codec".into())),
        };
        Ok(Self {
            range,
            codec,
            compressed: 0,
            decoded: 0,
            crc: 0,
            cancelled: false,
            done: false,
            failed: false,
        })
    }
    /// Cancel independently of progress observers; the next step produces no bytes.
    pub fn cancel(&mut self) {
        self.cancelled = true;
    }
    /// Compressed bytes consumed so far.
    pub fn compressed_bytes(&self) -> u64 {
        self.compressed
    }
    /// Decoded work produced so far.
    pub fn decoded_bytes(&self) -> u64 {
        self.decoded
    }
    /// Process a bounded prefix. `eof` means supplied bytes end the compressed range.
    pub fn step(&mut self, input: &[u8], maximum_output: usize, eof: bool) -> Result<Step> {
        if self.cancelled {
            return Err(Error::Cancelled);
        }
        if self.failed {
            return Err(Error::Integrity(
                "incremental decoder previously failed".into(),
            ));
        }
        if self.done {
            if input.is_empty() {
                return Ok(Step {
                    consumed: 0,
                    output: Vec::new(),
                    verified: true,
                    done: true,
                });
            }
            return Err(Error::Malformed(
                "input supplied after stream completion".into(),
            ));
        }
        if maximum_output == 0 || maximum_output > STEP_BYTES {
            return Err(Error::ResourceLimit("incremental step output"));
        }
        let remaining = self
            .range
            .compressed_bytes
            .checked_sub(self.compressed)
            .ok_or_else(|| Error::Integrity("compressed-size overflow".into()))?;
        if input.len() as u64 > remaining {
            self.failed = true;
            return Err(Error::Integrity(
                "compressed bytes exceed entry range".into(),
            ));
        }
        let bytes = &input[..input.len().min(STEP_BYTES)];
        let mut output = vec![0; maximum_output];
        let (consumed, produced, stream_end) = match &mut self.codec {
            Codec::Stored => {
                let count = bytes.len().min(output.len());
                output[..count].copy_from_slice(&bytes[..count]);
                (
                    count,
                    count,
                    self.compressed + count as u64 == self.range.compressed_bytes,
                )
            }
            Codec::Deflate(decoder) => {
                let before_in = decoder.total_in();
                let before_out = decoder.total_out();
                let status = decoder
                    .decompress(bytes, &mut output, InflateFlush::NoFlush)
                    .map_err(|e| {
                        self.failed = true;
                        Error::Integrity(e.as_str().into())
                    })?;
                (
                    usize::try_from(decoder.total_in() - before_in)
                        .map_err(|_| Error::ResourceLimit("incremental input conversion"))?,
                    usize::try_from(decoder.total_out() - before_out)
                        .map_err(|_| Error::ResourceLimit("incremental output conversion"))?,
                    status == Status::StreamEnd,
                )
            }
        };
        output.truncate(produced);
        self.compressed += consumed as u64;
        self.decoded = self
            .decoded
            .checked_add(produced as u64)
            .ok_or(Error::ResourceLimit("incremental decoded bytes"))?;
        if self.decoded > self.range.decoded_bytes {
            self.failed = true;
            return Err(Error::Integrity(
                "incremental decoded size exceeds declaration".into(),
            ));
        }
        self.crc = crc32(self.crc, &output);
        if stream_end {
            self.failed = true;
            if self.compressed != self.range.compressed_bytes
                || self.decoded != self.range.decoded_bytes
                || self.crc != self.range.crc32
            {
                return Err(Error::Integrity("incremental size or CRC mismatch".into()));
            }
            self.failed = false;
            self.done = true;
        } else if consumed == 0 && produced == 0 && eof && bytes.len() == input.len() {
            self.failed = true;
            return Err(Error::Integrity("truncated incremental stream".into()));
        }
        Ok(Step {
            consumed,
            output,
            verified: self.done,
            done: self.done,
        })
    }
}

/// One asynchronous source request. Offsets remain 64-bit on 32-bit WASM.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct RangeRequest {
    pub offset: u64,
    pub length: usize,
}
/// Indexed metadata progress without performing asynchronous calls inside Rust I/O.
pub enum IndexPoll {
    NeedRange(RangeRequest),
    Ready,
}
struct Cache {
    #[cfg(feature = "zip")]
    length: u64,
    ranges: BTreeMap<u64, Vec<u8>>,
    pending: Option<RangeRequest>,
    bytes: u64,
    limit: u64,
    exhausted: bool,
    #[cfg(test)]
    reads: usize,
}
#[cfg(feature = "zip")]
struct CachedReader {
    cache: Rc<RefCell<Cache>>,
    position: u64,
}
#[cfg(feature = "zip")]
impl Read for CachedReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let mut cache = self.cache.borrow_mut();
        #[cfg(test)]
        {
            cache.reads += 1;
        }
        if self.position >= cache.length || output.is_empty() {
            return Ok(0);
        }
        if let Some((&offset, data)) = cache.ranges.range(..=self.position).next_back() {
            let start = usize::try_from(self.position - offset)
                .map_err(|_| io::Error::other("range offset conversion"))?;
            if start < data.len() {
                let count = output.len().min(data.len() - start);
                output[..count].copy_from_slice(&data[start..start + count]);
                self.position += count as u64;
                return Ok(count);
            }
        }
        // Coalesce nearby metadata reads, bounded by the next cached range and
        // the remaining cache budget. Never refetch overlapping cached bytes.
        let next = cache
            .ranges
            .range(self.position..)
            .next()
            .map_or(cache.length, |(&offset, _)| offset);
        let available = next
            .saturating_sub(self.position)
            .min(cache.limit.saturating_sub(cache.bytes));
        if available == 0 {
            cache.exhausted = true;
            return Err(io::Error::other("range metadata cache exhausted"));
        }
        let length = output
            .len()
            .clamp(256, STEP_BYTES)
            .min(available.min(STEP_BYTES as u64) as usize)
            .min(
                usize::try_from((cache.length - self.position).min(STEP_BYTES as u64))
                    .map_err(|_| io::Error::other("range length conversion"))?,
            );
        cache.pending = Some(RangeRequest {
            offset: self.position,
            length,
        });
        Err(io::ErrorKind::WouldBlock.into())
    }
}
#[cfg(feature = "zip")]
impl Seek for CachedReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.position = match pos {
            SeekFrom::Start(n) => n,
            SeekFrom::Current(n) => self
                .position
                .checked_add_signed(n)
                .ok_or_else(|| io::Error::other("range seek overflow"))?,
            SeekFrom::End(n) => self
                .cache
                .borrow()
                .length
                .checked_add_signed(n)
                .ok_or_else(|| io::Error::other("range seek overflow"))?,
        };
        Ok(self.position)
    }
}

/// Bounded sparse ZIP metadata cache, released once indexing succeeds.
pub struct RangeIndex {
    cache: Rc<RefCell<Cache>>,
    limits: Limits,
    entries: Option<Vec<Entry>>,
    plans: Vec<Option<EntryRange>>,
    #[cfg(feature = "zip")]
    preflight: Option<crate::zip_backend::Preflight>,
    #[cfg(feature = "zip")]
    prefetched: u64,
    #[cfg(feature = "zip")]
    parser: Option<crate::zip_backend::Indexer<CachedReader>>,
}
impl RangeIndex {
    /// Start indexing an immutable byte source by its declared length.
    pub fn new(length: u64, limits: Limits) -> Result<Self> {
        if length > limits.max_input_bytes {
            return Err(Error::ResourceLimit("range source input"));
        }
        Ok(Self {
            cache: Rc::new(RefCell::new(Cache {
                #[cfg(feature = "zip")]
                length,
                ranges: BTreeMap::new(),
                pending: None,
                bytes: 0,
                limit: limits.max_metadata_bytes,
                exhausted: false,
                #[cfg(test)]
                reads: 0,
            })),
            limits,
            entries: None,
            plans: Vec::new(),
            #[cfg(feature = "zip")]
            preflight: None,
            #[cfg(feature = "zip")]
            prefetched: 0,
            #[cfg(feature = "zip")]
            parser: None,
        })
    }
    /// Poll the parser. A missing range is returned, never read through a JS callback.
    pub fn poll(&mut self) -> Result<IndexPoll> {
        if self.entries.is_some() {
            return Ok(IndexPoll::Ready);
        }
        {
            let mut cache = self.cache.borrow_mut();
            cache.pending = None;
            cache.exhausted = false;
        }
        #[cfg(feature = "zip")]
        let result = self.advance();
        #[cfg(not(feature = "zip"))]
        let result = Err(Error::Unsupported("ZIP feature unavailable".into()));
        match result {
            Ok(()) => Ok(IndexPoll::Ready),
            Err(error) => {
                if let Some(request) = self.cache.borrow().pending {
                    Ok(IndexPoll::NeedRange(request))
                } else if self.cache.borrow().exhausted {
                    Err(Error::ResourceLimit("range metadata cache"))
                } else {
                    Err(error)
                }
            }
        }
    }
    #[cfg(feature = "zip")]
    fn advance(&mut self) -> Result<()> {
        if self.parser.is_none() {
            let mut reader = CachedReader {
                cache: Rc::clone(&self.cache),
                position: 0,
            };
            if self.preflight.is_none() {
                self.preflight = Some(crate::zip_backend::preflight(&mut reader, self.limits)?);
            }
            let preflight = self
                .preflight
                .as_ref()
                .ok_or_else(|| Error::Malformed("ZIP preflight state".into()))?;
            // Fetch the directory sequentially before asking the dependency to
            // parse it. Each successful read advances this cursor, so even a
            // directory spanning many requests is parsed only once.
            reader.seek(SeekFrom::Start(preflight.directory_start + self.prefetched))?;
            let mut scratch = [0; STEP_BYTES];
            while self.prefetched < preflight.directory_size {
                let count =
                    (preflight.directory_size - self.prefetched).min(STEP_BYTES as u64) as usize;
                let read = reader.read(&mut scratch[..count])?;
                if read == 0 {
                    return Err(Error::Malformed("truncated ZIP central directory".into()));
                }
                self.prefetched += read as u64;
            }
            self.parser = Some(crate::zip_backend::Indexer::new(
                reader,
                self.limits,
                preflight.entries,
            )?);
        }
        let parser = self
            .parser
            .as_mut()
            .ok_or_else(|| Error::Malformed("ZIP parser state".into()))?;
        parser.poll()?;
        let (_, entries, locations) = self
            .parser
            .take()
            .ok_or_else(|| Error::Malformed("ZIP parser state".into()))?
            .finish()?;
        crate::validate_index(&entries, self.limits)?;
        self.plans = entries
            .iter()
            .zip(&locations)
            .map(|(entry, location)| {
                (entry.kind == crate::EntryKind::File
                    && !location.encrypted
                    && matches!(location.method, 0 | 8))
                .then_some(EntryRange {
                    id: entry.id,
                    offset: location.offset,
                    compressed_bytes: location.compressed,
                    decoded_bytes: entry.size,
                    method: location.method,
                    crc32: location.crc,
                })
            })
            .collect();
        self.entries = Some(entries);
        let mut cache = self.cache.borrow_mut();
        cache.ranges.clear();
        cache.bytes = 0;
        Ok(())
    }
    /// Supply exactly the requested immutable source range under the metadata budget.
    pub fn supply(&mut self, offset: u64, bytes: &[u8]) -> Result<()> {
        let mut cache = self.cache.borrow_mut();
        let request = cache
            .pending
            .ok_or_else(|| Error::Malformed("unsolicited source range".into()))?;
        if request.offset != offset || request.length != bytes.len() {
            return Err(Error::Malformed(
                "source range length/offset mismatch".into(),
            ));
        }
        let size = cache
            .bytes
            .checked_add(bytes.len() as u64)
            .ok_or(Error::ResourceLimit("range metadata cache"))?;
        if size > cache.limit {
            return Err(Error::ResourceLimit("range metadata cache"));
        }
        if cache.ranges.insert(offset, bytes.to_vec()).is_some() {
            return Err(Error::Malformed("duplicate cached range".into()));
        }
        cache.bytes = size;
        cache.pending = None;
        Ok(())
    }
    /// Metadata is available after `Ready`.
    pub fn entries(&self) -> Result<&[Entry]> {
        self.entries
            .as_deref()
            .ok_or_else(|| Error::Malformed("range index incomplete".into()))
    }
    /// Obtain a supported incremental decoded-entry contract.
    pub fn entry_range(&self, id: EntryId) -> Result<EntryRange> {
        self.plans.get(id.0).and_then(Clone::clone).ok_or_else(|| {
            Error::Unsupported(
                "incremental entry requires an unencrypted regular stored/DEFLATE ZIP member"
                    .into(),
            )
        })
    }
    /// Create a decoder after indexing, without copying compressed entry data.
    pub fn decoder(&self, id: EntryId) -> Result<EntryDecoder> {
        EntryDecoder::new(self.entry_range(id)?, self.limits)
    }
    /// Retained metadata-cache bytes; zero after indexing succeeds.
    pub fn cached_bytes(&self) -> u64 {
        self.cache.borrow().bytes
    }
}

#[cfg(all(test, feature = "zip"))]
mod tests {
    use super::*;
    fn stored_zip(count: usize) -> Vec<u8> {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for index in 0..count {
            writer
                .start_file(format!("{index:08}-{}", "n".repeat(100)), options)
                .unwrap();
            writer.write_all(&[42; 8192]).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }
    fn supply_all(index: &mut RangeIndex, zip: &[u8]) -> Result<(usize, usize)> {
        let mut requests = 0;
        let mut fetched = 0;
        while let IndexPoll::NeedRange(range) = index.poll()? {
            requests += 1;
            fetched += range.length;
            assert!(range.length <= STEP_BYTES);
            index.supply(
                range.offset,
                &zip[range.offset as usize..range.offset as usize + range.length],
            )?;
        }
        Ok((requests, fetched))
    }
    #[test]
    fn sparse_entry_plans_preserve_ids_across_unsupported_entries() {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.add_directory("directory/", options).unwrap();
        writer.start_file("directory/file", options).unwrap();
        writer.write_all(b"payload").unwrap();
        let zip = writer.finish().unwrap().into_inner();
        let mut index = RangeIndex::new(zip.len() as u64, Limits::default()).unwrap();
        supply_all(&mut index, &zip).unwrap();
        assert!(index.entry_range(EntryId(0)).is_err());
        assert_eq!(index.entry_range(EntryId(1)).unwrap().id, EntryId(1));
        assert!(index.entry_range(EntryId(2)).is_err());
    }
    #[test]
    fn sparse_many_member_index_has_linear_reads_and_coalesces_headers() {
        for count in [400, 1600] {
            let zip = stored_zip(count);
            let mut index = RangeIndex::new(zip.len() as u64, Limits::default()).unwrap();
            let (requests, fetched) = supply_all(&mut index, &zip).unwrap();
            assert_eq!(index.entries().unwrap().len(), count);
            assert!(
                requests < count + 32,
                "{count} entries needed {requests} requests"
            );
            let reads = index.cache.borrow().reads;
            assert!(
                reads < count * 16 + 128,
                "{count} entries needed {reads} parser reads"
            );
            assert!(fetched < count * 512 + 65557, "{fetched} fetched bytes");
            assert_eq!(index.cached_bytes(), 0);
            assert_eq!(
                index.entry_range(EntryId(count - 1)).unwrap().id,
                EntryId(count - 1)
            );
        }
    }
    #[test]
    fn sparse_index_rejects_local_name_and_method_mismatches_after_resumption() {
        for offset in [8, 30] {
            let mut zip = stored_zip(400);
            zip[offset] ^= 1;
            let mut index = RangeIndex::new(zip.len() as u64, Limits::default()).unwrap();
            assert!(matches!(
                supply_all(&mut index, &zip),
                Err(Error::Malformed(_))
            ));
            assert!(index.entries().is_err());
        }
    }
    #[test]
    fn sparse_index_rejects_overlapping_members_and_duplicate_names() {
        let mut overlapping = stored_zip(400);
        let central = overlapping
            .windows(4)
            .position(|v| v == b"PK\x01\x02")
            .unwrap();
        overlapping[central + 20..central + 24].copy_from_slice(&8193u32.to_le_bytes());
        let mut index = RangeIndex::new(overlapping.len() as u64, Limits::default()).unwrap();
        assert!(matches!(supply_all(&mut index, &overlapping),
            Err(Error::Malformed(message)) if message == "overlapping ZIP members"));

        let mut duplicate = stored_zip(400);
        let positions: Vec<_> = duplicate
            .windows(8)
            .enumerate()
            .filter_map(|(index, bytes)| (bytes == b"00000001").then_some(index))
            .collect();
        for position in positions {
            duplicate[position..position + 8].copy_from_slice(b"00000000");
        }
        let mut index = RangeIndex::new(duplicate.len() as u64, Limits::default()).unwrap();
        assert!(matches!(
            supply_all(&mut index, &duplicate),
            Err(Error::Unsupported(_))
        ));
    }
    #[test]
    fn sparse_index_respects_cache_budget_before_requesting_more_bytes() {
        let zip = stored_zip(10);
        let mut index = RangeIndex::new(
            zip.len() as u64,
            Limits {
                max_metadata_bytes: 1024,
                ..Limits::default()
            },
        )
        .unwrap();
        assert!(matches!(
            supply_all(&mut index, &zip),
            Err(Error::ResourceLimit(_))
        ));
        assert!(index.cached_bytes() <= 1024);
    }
    #[test]
    fn range_index_enforces_archive_wide_limits_before_ready() {
        let entries = [
            crate::CreateEntry {
                name: "a/b".into(),
                data: vec![1; 6],
                kind: crate::EntryKind::File,
            },
            crate::CreateEntry {
                name: "c".into(),
                data: vec![2; 6],
                kind: crate::EntryKind::File,
            },
        ];
        let mut zip = io::Cursor::new(Vec::new());
        crate::create(crate::Format::Zip, &entries, &mut zip, Limits::default()).unwrap();
        let zip = zip.into_inner();
        for (limits, expected) in [
            (
                Limits {
                    max_total_bytes: 8,
                    ..Limits::default()
                },
                "total decoded bytes",
            ),
            (
                Limits {
                    max_entry_bytes: 5,
                    ..Limits::default()
                },
                "entry decoded bytes",
            ),
            (
                Limits {
                    max_nesting_depth: 1,
                    ..Limits::default()
                },
                "entry path depth",
            ),
        ] {
            let mut index = RangeIndex::new(zip.len() as u64, limits).unwrap();
            loop {
                match index.poll() {
                    Ok(IndexPoll::NeedRange(range)) => index
                        .supply(
                            range.offset,
                            &zip[range.offset as usize..range.offset as usize + range.length],
                        )
                        .unwrap(),
                    Ok(IndexPoll::Ready) => panic!("accepted archive exceeding {expected}"),
                    Err(Error::ResourceLimit(resource)) => {
                        assert_eq!(resource, expected);
                        assert!(index.entries().is_err());
                        assert!(index.entry_range(EntryId(0)).is_err());
                        break;
                    }
                    Err(error) => panic!("unexpected error: {error}"),
                }
            }
        }
    }
    #[test]
    fn huge_deflate_entry_yields_bounded_chunks_and_cancellation() {
        let data = vec![42u8; 16 * 1024 * 1024];
        let mut compressed = Vec::new();
        crate::codec::deflate(&data, &mut compressed, false).unwrap();
        let range = EntryRange {
            id: EntryId(0),
            offset: 0,
            compressed_bytes: compressed.len() as u64,
            decoded_bytes: data.len() as u64,
            method: 8,
            crc32: crc32(0, &data),
        };
        let mut decoder = EntryDecoder::new(range.clone(), Limits::default()).unwrap();
        let mut offset = 0;
        let mut output = Vec::new();
        let mut steps = 0;
        loop {
            let step = decoder.step(&compressed[offset..], 4096, true).unwrap();
            offset += step.consumed;
            assert!(step.output.len() <= 4096);
            output.extend_from_slice(&step.output);
            steps += 1;
            if step.done {
                assert!(step.verified);
                break;
            }
        }
        assert!(steps >= 4096);
        assert_eq!(output, data);
        let mut cancelled = EntryDecoder::new(range, Limits::default()).unwrap();
        cancelled.step(&compressed, 4096, true).unwrap();
        cancelled.cancel();
        assert!(matches!(
            cancelled.step(&[], 4096, true),
            Err(Error::Cancelled)
        ));
    }
    #[test]
    fn crc_truncation_and_actual_output_mismatch_fail_terminal_verification() {
        let data = b"decoded payload";
        let mut compressed = Vec::new();
        crate::codec::deflate(data, &mut compressed, false).unwrap();
        let range = EntryRange {
            id: EntryId(0),
            offset: 0,
            compressed_bytes: compressed.len() as u64,
            decoded_bytes: data.len() as u64,
            method: 8,
            crc32: 0,
        };
        assert!(
            EntryDecoder::new(range.clone(), Limits::default())
                .unwrap()
                .step(&compressed, 65536, true)
                .is_err()
        );
        let mut range = range;
        range.crc32 = crc32(0, data);
        range.compressed_bytes -= 1;
        let mut truncated = EntryDecoder::new(range.clone(), Limits::default()).unwrap();
        match truncated.step(&compressed[..compressed.len() - 1], 65536, true) {
            Err(_) => {}
            Ok(step) => {
                assert!(!step.verified);
                assert!(truncated.step(&[], 65536, true).is_err());
            }
        }
        range.compressed_bytes += 1;
        range.decoded_bytes = 1;
        assert!(
            EntryDecoder::new(range, Limits::default())
                .unwrap()
                .step(&compressed, 65536, true)
                .is_err()
        );
    }
    #[test]
    fn sparse_range_index_avoids_whole_payload_and_releases_metadata() {
        let payload = vec![7u8; 4 * 1024 * 1024];
        let mut zip = std::io::Cursor::new(Vec::new());
        crate::create(
            crate::Format::Zip,
            &[crate::CreateEntry {
                name: "large.bin".into(),
                data: payload.clone(),
                kind: crate::EntryKind::File,
            }],
            &mut zip,
            Limits::default(),
        )
        .unwrap();
        let zip = zip.into_inner();
        let mut index = RangeIndex::new(zip.len() as u64, Limits::default()).unwrap();
        let mut reads = 0;
        let mut fetched = 0;
        while let IndexPoll::NeedRange(range) = index.poll().unwrap() {
            reads += 1;
            fetched += range.length;
            assert!(range.length <= STEP_BYTES);
            index
                .supply(
                    range.offset,
                    &zip[range.offset as usize..range.offset as usize + range.length],
                )
                .unwrap();
        }
        assert!(reads > 0);
        assert!(fetched < 256 * 1024);
        assert_eq!(index.cached_bytes(), 0);
        assert_eq!(index.entries().unwrap()[0].size, payload.len() as u64);
        let range = index.entry_range(EntryId(0)).unwrap();
        let mut decoder = index.decoder(EntryId(0)).unwrap();
        let mut position = range.offset as usize;
        let end = position + range.compressed_bytes as usize;
        let mut decoded = 0;
        loop {
            let next = (position + STEP_BYTES).min(end);
            let step = decoder
                .step(&zip[position..next], STEP_BYTES, next == end)
                .unwrap();
            position += step.consumed;
            decoded += step.output.len();
            if step.done {
                break;
            }
        }
        assert_eq!(decoded, payload.len());
    }
}
