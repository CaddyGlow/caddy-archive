//! UDF archive adapter with separate main namespace and associated streams.
use crate::{Entry, EntryId, EntryKind, Error, ExtractReport, Limits, Result};
use libmkiso::udf;
use std::{
    cell::RefCell,
    io::{Read, Seek, SeekFrom, Write},
};

struct SeekSource<R> {
    reader: RefCell<R>,
    length: u64,
}
impl<R: Read + Seek> libmkiso::source::ReadAt for SeekSource<R> {
    fn len(&self) -> u64 {
        self.length
    }
    fn read_at(&self, offset: u64, bytes: &mut [u8]) -> std::io::Result<usize> {
        if offset >= self.length {
            return Ok(0);
        }
        let count = (self.length - offset).min(bytes.len() as u64) as usize;
        let mut reader = self.reader.borrow_mut();
        reader.seek(SeekFrom::Start(offset))?;
        reader.read(&mut bytes[..count])
    }
}

fn map_error(error: udf::Error) -> Error {
    match error {
        udf::Error::Io(error) => Error::Io(error),
        udf::Error::Malformed(message) => Error::Malformed(format!("UDF: {message}")),
        udf::Error::Unsupported(message) => Error::Unsupported(message),
        udf::Error::Integrity(message) => Error::Integrity(message),
        udf::Error::ResourceLimit(resource) => Error::ResourceLimit(resource),
    }
}

/// Bounded UDF baseline reader over caller-owned bytes.
pub struct UdfArchive<'a> {
    reader: udf::UdfReader<'a>,
    entries: Vec<Entry>,
    reader_indices: Vec<usize>,
    stream_indices: Vec<usize>,
}
impl<'a> UdfArchive<'a> {
    /// Parse supported UDF 1.02–2.60 profiles, including associated streams.
    pub fn open(bytes: &'a [u8], limits: Limits) -> Result<Self> {
        Self::open_reader(std::io::Cursor::new(bytes), limits)
    }
    /// Open immutable seekable input without retaining the complete image.
    /// The caller must keep source bytes stable until the archive is dropped.
    pub fn open_reader<R: Read + Seek + 'a>(mut reader: R, limits: Limits) -> Result<Self> {
        let length = reader.seek(SeekFrom::End(0))?;
        let reader = udf::UdfReader::open_source(
            SeekSource {
                reader: RefCell::new(reader),
                length,
            },
            udf::Limits {
                max_entries: limits.max_entries,
                max_metadata_bytes: limits.max_metadata_bytes,
                max_entry_bytes: limits.max_entry_bytes,
                max_total_bytes: limits.max_total_bytes,
                max_input_bytes: limits.max_input_bytes,
                max_nesting_depth: limits.max_nesting_depth,
            },
        )
        .map_err(map_error)?;
        let reader_indices: Vec<_> = reader
            .entries()
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.stream.is_none())
            .map(|(index, _)| index)
            .collect();
        let stream_indices = reader
            .entries()
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.stream.is_some())
            .map(|(index, _)| index)
            .collect();
        let entries = reader
            .entries()
            .iter()
            .filter(|entry| entry.stream.is_none())
            .enumerate()
            .map(|(index, entry)| Entry {
                id: EntryId(index),
                raw_name: entry.raw_name.clone(),
                name: entry.name.clone(),
                kind: if entry.kind == udf::EntryKind::SymbolicLink {
                    EntryKind::Link
                } else if entry.directory {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
                size: entry.size,
                compressed_size: Some(entry.stored_size),
                compression: "stored".into(),
                encrypted: false,
            })
            .collect();
        Ok(Self {
            reader,
            entries,
            reader_indices,
            stream_indices,
        })
    }
    /// Metadata in breadth-first stored directory order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    fn reader_index(&self, id: EntryId) -> Result<usize> {
        self.reader_indices
            .get(id.0)
            .copied()
            .ok_or_else(|| Error::Malformed("invalid UDF entry index".into()))
    }
    /// Associated stream metadata. Stream IDs are the returned reader indices.
    pub fn streams(&self) -> impl Iterator<Item = (usize, &udf::Entry)> {
        self.stream_indices
            .iter()
            .map(|&index| (index, &self.reader.entries()[index]))
    }
    /// Extract an associated stream by its stream ID.
    pub fn extract_stream(&self, id: usize, output: &mut impl Write) -> Result<ExtractReport> {
        if !self.stream_indices.contains(&id) {
            return Err(Error::Malformed("invalid UDF stream index".into()));
        }
        let bytes = self.reader.extract(id, output).map_err(map_error)?;
        Ok(ExtractReport {
            bytes,
            entries: 1,
            verified: true,
        })
    }
    /// Main namespace owner of a stream, when its owner is published there.
    pub fn stream_owner(&self, id: usize) -> Option<EntryId> {
        if !self.stream_indices.contains(&id) {
            return None;
        }
        let owner = self.reader.entries()[id].stream.as_ref()?.owner?;
        self.reader_indices
            .iter()
            .position(|&index| index == owner)
            .map(EntryId)
    }
    /// Decoded symbolic link target, without materializing a host link.
    pub fn link_target(&self, id: EntryId) -> Option<&str> {
        self.reader
            .entries()
            .get(*self.reader_indices.get(id.0)?)?
            .link_target
            .as_deref()
    }
    /// Stream extents; descriptor checksums do not authenticate payload bytes.
    pub fn extract(&self, id: EntryId, output: &mut impl Write) -> Result<ExtractReport> {
        let bytes = self
            .reader
            .extract(self.reader_index(id)?, output)
            .map_err(map_error)?;
        Ok(ExtractReport {
            bytes,
            entries: 1,
            verified: true,
        })
    }
    /// Read one payload with an explicit allocation maximum.
    pub fn read_entry(&self, id: EntryId, maximum: u64) -> Result<Vec<u8>> {
        self.reader
            .read_entry(self.reader_index(id)?, maximum)
            .map_err(map_error)
    }
    /// Validate extents; the profile does not provide payload authentication.
    pub fn test(&self) -> Result<ExtractReport> {
        let mut report = ExtractReport {
            verified: true,
            ..Default::default()
        };
        for entry in &self.entries {
            report.bytes += self.extract(entry.id, &mut std::io::sink())?.bytes;
            report.entries += 1;
        }
        for &id in &self.stream_indices {
            report.bytes += self.extract_stream(id, &mut std::io::sink())?.bytes;
            report.entries += 1;
        }
        Ok(report)
    }
}
