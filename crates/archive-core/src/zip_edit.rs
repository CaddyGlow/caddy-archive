//! Bounded ZIP rename, deletion, timestamp and per-entry encryption edits.
//!
//! This profile accepts single-disk, prefix-free ZIP archives with Copy/DEFLATE
//! or WinZip AES payloads. Rename/delete and ordinary timestamp edits copy
//! retained packed bytes without credentials or decoded verification. Selected
//! encryption/password changes authenticate/decode old data before any output,
//! then stream compressed bytes through the new encryption without recompression.
//! A ZipCrypto descriptor timestamp check-byte change follows that verified path.
//! UT Unix seconds are authoritative; DOS fallback uses UTC with pre-1980 dates
//! clamped to 1980. Inputs above the 32-bit UT range fail. Existing NTFS access and
//! creation times remain intact. ZIP cannot encrypt filenames; use 7z for that.
//!
//! Unknown/name-dependent extras, signatures, padding and trailing data fail
//! before writing. Reports separate verified changed payloads from opaque packed
//! copying. Callers own provisional output/publication and keep source contents
//! stable throughout execution. Credential buffers remain under caller ownership.
use crate::{Error, Limits, Result};
use serde::Serialize;
use std::collections::BTreeSet;
use std::io::{Read, Seek, SeekFrom, Write};

/// Simultaneous name-based operations against original archive names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum EditOperation {
    /// Remove an entry, or a directory and its descendants when ending in `/`.
    Delete { name: String },
    /// Rename an entry, or directory descendants using path boundaries.
    Rename { from: String, to: String },
    /// Set authoritative Unix seconds and a UTC-derived DOS fallback.
    SetModified {
        name: String,
        modified_unix_seconds: u64,
    },
    /// Re-encrypt or clear selected payloads; credentials belong to execution.
    SetEncryption {
        name: String,
        encryption: EntryEncryption,
    },
}
/// Target encryption of selected ZIP entries. Filename encryption requires 7z.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum EntryEncryption {
    None,
    Aes256,
    ZipCrypto,
}
/// Borrowed credentials/randomness never enter plans, reports or diagnostics.
/// Callers retain ownership and must erase their credential buffers themselves.
#[derive(Default)]
pub struct ZipEditOptions<'a> {
    pub password: Option<&'a [u8]>,
    pub new_password: Option<&'a [u8]>,
    pub randomness: Option<&'a mut dyn crate::RandomSource>,
}
/// A dry-run decision; names remain archive names, not filesystem destinations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlannedEntry {
    pub original_name: String,
    pub result_name: Option<String>,
    pub packed_bytes: u64,
    pub modified_unix_seconds: Option<u64>,
    pub encryption: Option<EntryEncryption>,
    pub requires_reencryption: bool,
}
/// Immutable validated metadata and decisions. Payloads are never buffered.
#[derive(Debug)]
pub struct EditPlan {
    entries: Vec<PlannedEntry>,
    records: Vec<Record>,
    comment: Vec<u8>,
    source_length: u64,
    source_trailer: Vec<u8>,
    trailer_offset: u64,
    output_records: Vec<Option<Record>>,
    limits: Limits,
    output_offsets: Vec<Option<u64>>,
}
impl EditPlan {
    pub fn entries(&self) -> &[PlannedEntry] {
        &self.entries
    }
}
/// Counts describe packed reuse, without claiming decoded payload verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EditReport {
    pub retained_entries: u64,
    pub removed_entries: u64,
    pub renamed_entries: u64,
    pub packed_bytes_copied: u64,
    pub payloads_verified: bool,
    pub verified_entries: u64,
    pub reencrypted_entries: u64,
    pub metadata_changed_entries: u64,
}
#[derive(Debug, Clone)]
struct Record {
    local: [u8; 30],
    central: [u8; 46],
    name: String,
    local_extra: Vec<u8>,
    central_extra: Vec<u8>,
    comment: Vec<u8>,
    payload_offset: u64,
    compressed: u64,
    descriptor: Vec<u8>,
    size: u64,
    header_offset: u64,
    central_offset: u64,
}
fn malformed(message: &str) -> Error {
    Error::Malformed(message.into())
}
fn unsupported(message: &str) -> Error {
    Error::Unsupported(message.into())
}
fn u16at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}
fn u32at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap_or([0; 4]))
}
fn u64at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap_or([0; 8]))
}
fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn add(left: u64, right: u64) -> Result<u64> {
    left.checked_add(right)
        .ok_or_else(|| malformed("ZIP edit range overflow"))
}
fn charge(total: &mut u64, bytes: u64, limits: Limits) -> Result<()> {
    *total = add(*total, bytes)?;
    if *total > limits.max_metadata_bytes {
        return Err(Error::ResourceLimit("metadata bytes"));
    }
    Ok(())
}
fn read_vec(reader: &mut impl Read, length: usize) -> Result<Vec<u8>> {
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}
fn extras(extra: &[u8]) -> Result<Vec<(u16, &[u8])>> {
    let mut result = Vec::new();
    let mut cursor = 0;
    let mut ids = BTreeSet::new();
    while cursor < extra.len() {
        if extra.len() - cursor < 4 {
            return Err(malformed("truncated ZIP extra field"));
        }
        let id = u16at(extra, cursor);
        let length = usize::from(u16at(extra, cursor + 2));
        cursor += 4;
        if length > extra.len() - cursor {
            return Err(malformed("ZIP extra field length"));
        }
        if !matches!(id, 1 | 0x5455 | 0x000a | 0x9901 | 0x7875) {
            return Err(unsupported(
                "ZIP edit unknown or name-dependent extra field",
            ));
        }
        if !ids.insert(id) {
            return Err(unsupported("ZIP edit duplicate extra field"));
        }
        result.push((id, &extra[cursor..cursor + length]));
        cursor += length;
    }
    Ok(result)
}
fn resolved_fields(header: &[u8], extra: &[u8], central: bool) -> Result<(u64, u64, u64)> {
    let (size_at, packed_at) = if central { (24, 20) } else { (22, 18) };
    let mut fields = [
        u64::from(u32at(header, size_at)),
        u64::from(u32at(header, packed_at)),
        if central {
            u64::from(u32at(header, 42))
        } else {
            0
        },
    ];
    let parsed = extras(extra)?;
    let zip64 = parsed
        .iter()
        .find(|(id, _)| *id == 1)
        .map(|(_, value)| *value)
        .unwrap_or_default();
    let mut cursor = 0;
    for field in &mut fields {
        if *field == u64::from(u32::MAX) {
            if zip64.len() - cursor < 8 {
                return Err(malformed("ZIP64 edit field missing"));
            }
            *field = u64at(zip64, cursor);
            cursor += 8;
        }
    }
    if cursor != zip64.len() {
        return Err(unsupported("ZIP edit unsupported ZIP64 extra layout"));
    }
    Ok((fields[0], fields[1], fields[2]))
}
fn matches_name(name: &str, pattern: &str) -> bool {
    name == pattern || (pattern.ends_with('/') && name.starts_with(pattern))
}
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.as_bytes().contains(&0) || name.len() > usize::from(u16::MAX) {
        return Err(malformed("ZIP edit invalid archive name"));
    }
    Ok(())
}
/// Read and validate metadata, classify simultaneous operations and reject
/// preservation gaps and collisions before creating any output.
pub fn plan(
    reader: &mut (impl Read + Seek),
    operations: &[EditOperation],
    limits: Limits,
) -> Result<EditPlan> {
    if operations.len() as u64 > limits.max_entries {
        return Err(Error::ResourceLimit("edit operations"));
    }
    let length = reader.seek(SeekFrom::End(0))?;
    if length > limits.max_input_bytes {
        return Err(Error::ResourceLimit("input bytes"));
    }
    let index = crate::zip_backend::preflight(reader, limits)?;
    let tail_len = length.min(65557) as usize;
    reader.seek(SeekFrom::Start(length - tail_len as u64))?;
    let tail = read_vec(reader, tail_len)?;
    let end_at = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&at| {
            tail[at..].starts_with(b"PK\x05\x06")
                && at + 22 + usize::from(u16at(&tail, at + 20)) == tail.len()
        })
        .ok_or_else(|| malformed("ZIP edit end record"))?;
    let end = &tail[end_at..end_at + 22];
    if u16at(end, 4) != 0 || u16at(end, 6) != 0 || u16at(end, 8) != u16at(end, 10) {
        return Err(unsupported("ZIP edit split archive"));
    }
    let comment = tail[end_at + 22..].to_vec();
    let mut metadata = 0;
    charge(&mut metadata, comment.len() as u64, limits)?;
    let absolute_end = length - tail_len as u64 + end_at as u64;
    let zip64 =
        u16at(end, 10) == u16::MAX || u32at(end, 12) == u32::MAX || u32at(end, 16) == u32::MAX;
    let directory_end = if zip64 {
        if absolute_end < 20 {
            return Err(malformed("ZIP64 edit locator"));
        }
        reader.seek(SeekFrom::Start(absolute_end - 20))?;
        let mut locator = [0; 20];
        reader.read_exact(&mut locator)?;
        if !locator.starts_with(b"PK\x06\x07")
            || u32at(&locator, 4) != 0
            || u32at(&locator, 16) != 1
        {
            return Err(unsupported("ZIP edit split ZIP64"));
        }
        let offset = u64at(&locator, 8);
        reader.seek(SeekFrom::Start(offset))?;
        let mut record = [0; 56];
        reader.read_exact(&mut record)?;
        if !record.starts_with(b"PK\x06\x06")
            || u64at(&record, 4) != 44
            || u32at(&record, 16) != 0
            || u32at(&record, 20) != 0
            || u64at(&record, 24) != u64at(&record, 32)
            || add(offset, 76)? != absolute_end
        {
            return Err(unsupported("ZIP edit unsupported ZIP64 end records"));
        }
        if u64at(&record, 48) != index.directory_start {
            return Err(unsupported("ZIP edit SFX or archive prefix"));
        }
        offset
    } else {
        if u64::from(u32at(end, 16)) != index.directory_start {
            return Err(unsupported("ZIP edit SFX or archive prefix"));
        }
        absolute_end
    };
    if add(index.directory_start, index.directory_size)? != directory_end {
        return Err(unsupported("ZIP edit signatures or directory padding"));
    }
    let trailer_length = length
        .checked_sub(directory_end)
        .ok_or_else(|| malformed("ZIP edit trailer range"))?;
    charge(&mut metadata, trailer_length, limits)?;
    reader.seek(SeekFrom::Start(directory_end))?;
    let source_trailer = read_vec(reader, trailer_length as usize)?;
    charge(&mut metadata, operations.len() as u64, limits)?;
    reader.seek(SeekFrom::Start(index.directory_start))?;
    let mut records = Vec::new();
    let mut names = BTreeSet::new();
    let mut total_size = 0;
    for _ in 0..index.entries {
        let central_offset = reader.stream_position()?;
        if add(central_offset, 46)? > directory_end {
            return Err(malformed("ZIP edit central header range"));
        }
        let mut central = [0; 46];
        reader.read_exact(&mut central)?;
        if !central.starts_with(b"PK\x01\x02") {
            return Err(malformed("ZIP edit central header"));
        }
        if u16at(&central, 34) != 0 {
            return Err(unsupported("ZIP edit split entry"));
        }
        let name_len = usize::from(u16at(&central, 28));
        let extra_len = usize::from(u16at(&central, 30));
        let comment_len = usize::from(u16at(&central, 32));
        charge(
            &mut metadata,
            (name_len * 3
                + extra_len
                + comment_len
                + std::mem::size_of::<Record>()
                + std::mem::size_of::<PlannedEntry>()) as u64,
            limits,
        )?;
        if add(
            reader.stream_position()?,
            (name_len + extra_len + comment_len) as u64,
        )? > directory_end
        {
            return Err(malformed("ZIP edit central metadata range"));
        }
        let raw_name = read_vec(reader, name_len)?;
        let name = String::from_utf8(raw_name)
            .map_err(|_| unsupported("ZIP edit non-UTF-8 archive name"))?;
        validate_name(&name)?;
        if u16at(&central, 8) & 0x800 == 0 && !name.is_ascii() {
            return Err(unsupported("ZIP edit legacy filename encoding"));
        }
        if !names.insert(name.clone()) {
            return Err(unsupported("ZIP edit duplicate archive names"));
        }
        let extra = read_vec(reader, extra_len)?;
        let entry_comment = read_vec(reader, comment_len)?;
        let (size, compressed, offset) = resolved_fields(&central, &extra, true)?;
        if size > limits.max_entry_bytes {
            return Err(Error::ResourceLimit("entry bytes"));
        }
        total_size = add(total_size, size)?;
        if total_size > limits.max_total_bytes {
            return Err(Error::ResourceLimit("total bytes"));
        }
        records.push(Record {
            local: [0; 30],
            central,
            name,
            local_extra: Vec::new(),
            central_extra: extra,
            comment: entry_comment,
            payload_offset: 0,
            compressed,
            descriptor: Vec::new(),
            size,
            header_offset: offset,
            central_offset,
        });
    }
    if reader.stream_position()? != directory_end {
        return Err(unsupported(
            "ZIP edit digital signature or unindexed directory data",
        ));
    }
    let mut ranges = Vec::new();
    for record in &mut records {
        if add(record.header_offset, 30)? > index.directory_start {
            return Err(malformed("ZIP edit local header range"));
        }
        reader.seek(SeekFrom::Start(record.header_offset))?;
        reader.read_exact(&mut record.local)?;
        if !record.local.starts_with(b"PK\x03\x04") {
            return Err(malformed("ZIP edit local header"));
        }
        let flags = u16at(&record.local, 6);
        let method = u16at(&record.local, 8);
        if flags & !0x80f != 0 || !matches!(method, 0 | 8 | 99) {
            return Err(unsupported("ZIP edit unsupported flags or codec"));
        }
        if flags != u16at(&record.central, 8)
            || method != u16at(&record.central, 10)
            || record.local[10..14] != record.central[12..16]
        {
            return Err(malformed("ZIP edit local/central properties mismatch"));
        }
        let name_len = usize::from(u16at(&record.local, 26));
        let extra_len = usize::from(u16at(&record.local, 28));
        if add(reader.stream_position()?, (name_len + extra_len) as u64)? > index.directory_start {
            return Err(malformed("ZIP edit local metadata range"));
        }
        charge(
            &mut metadata,
            (name_len + extra_len + 24 + 16) as u64,
            limits,
        )?;
        if read_vec(reader, name_len)? != record.name.as_bytes() {
            return Err(malformed("ZIP edit local/central names mismatch"));
        }
        record.local_extra = read_vec(reader, extra_len)?;
        let local_fields = resolved_fields(&record.local, &record.local_extra, false)?;
        let local_aes = extras(&record.local_extra)?
            .into_iter()
            .find(|(id, _)| *id == 0x9901)
            .map(|(_, value)| value);
        let central_aes = extras(&record.central_extra)?
            .into_iter()
            .find(|(id, _)| *id == 0x9901)
            .map(|(_, value)| value);
        if local_aes != central_aes {
            return Err(malformed("ZIP edit AES extra mismatch"));
        }
        if method == 99 {
            let aes = local_aes.ok_or_else(|| malformed("ZIP edit missing AES extra"))?;
            if aes.len() != 7
                || &aes[2..4] != b"AE"
                || !matches!(u16at(aes, 0), 1 | 2)
                || !(1..=3).contains(&aes[4])
                || !matches!(u16at(aes, 5), 0 | 8)
                || flags & 1 == 0
            {
                return Err(unsupported("ZIP edit unsupported AES profile"));
            }
        } else if local_aes.is_some() {
            return Err(malformed("ZIP edit AES extra without AES method"));
        }
        if flags & 8 == 0
            && (local_fields.0 != record.size
                || local_fields.1 != record.compressed
                || u32at(&record.local, 14) != u32at(&record.central, 16))
        {
            return Err(malformed("ZIP edit local/central sizes or CRC mismatch"));
        }
        record.payload_offset = reader.stream_position()?;
        let mut end = add(record.payload_offset, record.compressed)?;
        if end > index.directory_start {
            return Err(malformed("ZIP edit payload overlaps directory"));
        }
        if flags & 8 != 0 {
            if add(end, 12)? > index.directory_start {
                return Err(malformed("ZIP edit descriptor range"));
            }
            reader.seek(SeekFrom::Start(end))?;
            let mut first = [0; 4];
            reader.read_exact(&mut first)?;
            let signed = first == *b"PK\x07\x08";
            let wide = u32at(&record.local, 18) == u32::MAX || u32at(&record.local, 22) == u32::MAX;
            let rest = if wide { 16 } else { 8 };
            record.descriptor.extend_from_slice(&first);
            if signed {
                record.descriptor.extend_from_slice(&read_vec(reader, 4)?);
            }
            record
                .descriptor
                .extend_from_slice(&read_vec(reader, rest)?);
            let at = if signed { 4 } else { 0 };
            let decoded = if wide {
                u64at(&record.descriptor, at + 12)
            } else {
                u64::from(u32at(&record.descriptor, at + 8))
            };
            let packed = if wide {
                u64at(&record.descriptor, at + 4)
            } else {
                u64::from(u32at(&record.descriptor, at + 4))
            };
            if u32at(&record.descriptor, at) != u32at(&record.central, 16)
                || decoded != record.size
                || packed != record.compressed
            {
                return Err(malformed("ZIP edit descriptor mismatch"));
            }
            end = add(end, record.descriptor.len() as u64)?;
            if end > index.directory_start {
                return Err(malformed("ZIP edit descriptor overlaps directory"));
            }
        }
        ranges.push((record.header_offset, end));
    }
    ranges.sort_unstable();
    let mut next = 0;
    for (start, end) in ranges {
        if start != next {
            return Err(unsupported("ZIP edit overlapping members, SFX or padding"));
        }
        next = end;
    }
    if next != index.directory_start {
        return Err(unsupported("ZIP edit unindexed local data or padding"));
    }
    let mut entries = Vec::new();
    let mut used = vec![false; operations.len()];
    let mut result_names = BTreeSet::new();
    for record in &records {
        let mut result = Some(record.name.clone());
        let mut name_matched = false;
        let mut modified = None;
        let mut encryption = None;
        for (i, operation) in operations.iter().enumerate() {
            let pattern = match operation {
                EditOperation::Delete { name }
                | EditOperation::SetModified { name, .. }
                | EditOperation::SetEncryption { name, .. } => name,
                EditOperation::Rename { from, .. } => from,
            };
            validate_name(pattern)?;
            if !matches_name(&record.name, pattern) {
                continue;
            }
            used[i] = true;
            match operation {
                EditOperation::Delete { .. } => {
                    if name_matched {
                        return Err(malformed("ZIP edit overlapping name operations"));
                    }
                    name_matched = true;
                    result = None;
                }
                EditOperation::Rename { from, to } => {
                    if name_matched {
                        return Err(malformed("ZIP edit overlapping name operations"));
                    }
                    name_matched = true;
                    validate_name(to)?;
                    if from.ends_with('/') != to.ends_with('/') {
                        return Err(malformed(
                            "ZIP edit directory rename requires trailing slash",
                        ));
                    }
                    let name = format!("{}{}", to, &record.name[from.len()..]);
                    validate_name(&name)?;
                    if u16at(&record.central, 8) & 0x800 == 0 && !name.is_ascii() {
                        return Err(unsupported("ZIP edit Unicode rename of legacy name"));
                    }
                    result = Some(name);
                }
                EditOperation::SetModified {
                    modified_unix_seconds,
                    ..
                } => {
                    if modified.is_some() {
                        return Err(malformed("ZIP edit overlapping timestamp operations"));
                    }
                    if *modified_unix_seconds > u64::from(u32::MAX) {
                        return Err(unsupported("ZIP edit Unix timestamp exceeds 32 bits"));
                    }
                    modified = Some(*modified_unix_seconds);
                }
                EditOperation::SetEncryption {
                    encryption: target, ..
                } => {
                    if encryption.is_some() {
                        return Err(malformed("ZIP edit overlapping encryption operations"));
                    }
                    encryption = Some(*target);
                }
            }
        }
        if result.is_none() && (modified.is_some() || encryption.is_some()) {
            return Err(malformed("ZIP edit removal conflicts with property edits"));
        }
        let requires_reencryption = encryption.is_some()
            || modified.is_some_and(|seconds| {
                source_encryption(record) == EntryEncryption::ZipCrypto
                    && u16at(&record.local, 6) & 8 != 0
                    && utc_dos_time(seconds).0.to_le_bytes()[1] != record.local[11]
            });
        if requires_reencryption
            && !cfg!(feature = "crypto")
            && (source_encryption(record) != EntryEncryption::None
                || encryption.is_some_and(|mode| mode != EntryEncryption::None))
        {
            return Err(unsupported("ZIP edit crypto feature unavailable"));
        }
        if let Some(name) = &result {
            if !result_names.insert(name.clone()) {
                return Err(malformed("ZIP edit resulting name collision"));
            }
            charge(&mut metadata, name.len() as u64 * 2, limits)?;
        }
        entries.push(PlannedEntry {
            original_name: record.name.clone(),
            result_name: result,
            packed_bytes: record.compressed,
            modified_unix_seconds: modified,
            encryption,
            requires_reencryption,
        });
    }
    if used.iter().any(|used| !used) {
        return Err(malformed("ZIP edit operation matched no entries"));
    }
    for name in &result_names {
        if !name.ends_with('/')
            && result_names
                .range(format!("{name}/")..)
                .next()
                .is_some_and(|other| other.starts_with(&format!("{name}/")))
        {
            return Err(malformed("ZIP edit resulting file/directory conflict"));
        }
    }
    // Resolve all metadata, wrapper sizes and offsets before provisional output.
    let mut output_records = Vec::new();
    let mut output_offsets = Vec::new();
    let mut output_offset = 0;
    for (record, decision) in records.iter().zip(&entries) {
        charge(
            &mut metadata,
            (std::mem::size_of::<Option<Record>>() + std::mem::size_of::<Option<u64>>()) as u64,
            limits,
        )?;
        if let Some(name) = &decision.result_name {
            charge(
                &mut metadata,
                (record.name.len()
                    + record.local_extra.len()
                    + record.central_extra.len()
                    + record.comment.len()
                    + record.descriptor.len()
                    + 128) as u64,
                limits,
            )?;
            let mut output = record.clone();
            if let Some(seconds) = decision.modified_unix_seconds {
                set_modified(&mut output, seconds)?;
            }
            if decision.requires_reencryption {
                let target = decision.encryption.unwrap_or(source_encryption(record));
                prepare_encryption(&mut output, record, target)?;
            }
            let (extra, offset_field) = relocated_extra(&output, output_offset)?;
            output.central_extra = extra;
            put32(&mut output.central, 42, offset_field);
            if offset_field == u32::MAX {
                let version = u16at(&output.central, 6).max(45);
                put16(&mut output.central, 6, version);
            }
            output_offsets.push(Some(output_offset));
            output_offset = add(
                output_offset,
                30 + name.len() as u64 + output.local_extra.len() as u64,
            )?;
            output_offset = add(output_offset, output.compressed)?;
            output_offset = add(output_offset, output.descriptor.len() as u64)?;
            output_records.push(Some(output));
        } else {
            output_records.push(None);
            output_offsets.push(None);
        }
    }
    Ok(EditPlan {
        entries,
        records,
        comment,
        source_length: length,
        source_trailer,
        trailer_offset: directory_end,
        output_records,
        output_offsets,
        limits,
    })
}

fn source_encryption(record: &Record) -> EntryEncryption {
    if u16at(&record.local, 6) & 1 == 0 {
        EntryEncryption::None
    } else if u16at(&record.local, 8) == 99 {
        EntryEncryption::Aes256
    } else {
        EntryEncryption::ZipCrypto
    }
}
fn aes_properties(record: &Record) -> Result<Option<(u16, u8, u16)>> {
    Ok(extras(&record.local_extra)?
        .into_iter()
        .find(|(id, _)| *id == 0x9901)
        .map(|(_, bytes)| (u16at(bytes, 0), bytes[4], u16at(bytes, 5))))
}
// Authoritative UT seconds are UTC. DOS fallback is also derived from UTC and
// clamps pre-1980 input; it does not claim to reproduce caller-local wall time.
fn utc_dos_time(seconds: u64) -> (u16, u16) {
    let seconds = seconds.max(315_532_800);
    let z = seconds / 86400 + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    year += u64::from(month <= 2);
    let time =
        (((seconds / 3600 % 24) << 11) | ((seconds / 60 % 60) << 5) | (seconds % 60 / 2)) as u16;
    let date = (((year - 1980) << 9) | (month << 5) | day) as u16;
    (time, date)
}
fn append_extra(output: &mut Vec<u8>, id: u16, data: &[u8]) -> Result<()> {
    let length = u16::try_from(data.len()).map_err(|_| Error::ResourceLimit("ZIP extra bytes"))?;
    output.extend_from_slice(&id.to_le_bytes());
    output.extend_from_slice(&length.to_le_bytes());
    output.extend_from_slice(data);
    if output.len() > usize::from(u16::MAX) {
        return Err(Error::ResourceLimit("ZIP extra bytes"));
    }
    Ok(())
}
fn timestamp_extra(extra: &[u8], seconds: u64, central: bool) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut ut = false;
    for (id, value) in extras(extra)? {
        let mut data = value.to_vec();
        match id {
            0x5455 => {
                if data.is_empty() || data[0] & !7 != 0 {
                    return Err(malformed("ZIP edit invalid Unix timestamp extra"));
                }
                if data[0] & 1 != 0 {
                    if data.len() < 5 {
                        return Err(malformed("ZIP edit truncated Unix modification time"));
                    }
                    data[1..5].copy_from_slice(&(seconds as u32).to_le_bytes());
                } else {
                    data[0] |= 1;
                    data.splice(1..1, (seconds as u32).to_le_bytes());
                }
                if !central && data.len() != 1 + (data[0].count_ones() as usize) * 4 {
                    return Err(malformed("ZIP edit invalid Unix timestamp layout"));
                }
                ut = true;
            }
            0x000a => {
                if data.len() < 4 {
                    return Err(malformed("ZIP edit truncated NTFS timestamp extra"));
                }
                let mut at = 4;
                while at < data.len() {
                    if data.len() - at < 4 {
                        return Err(malformed("ZIP edit truncated NTFS tag"));
                    }
                    let tag = u16at(&data, at);
                    let length = usize::from(u16at(&data, at + 2));
                    at += 4;
                    if length > data.len() - at {
                        return Err(malformed("ZIP edit NTFS tag length"));
                    }
                    if tag == 1 {
                        if length != 24 {
                            return Err(malformed("ZIP edit NTFS time layout"));
                        }
                        let filetime = (seconds + 11_644_473_600) * 10_000_000;
                        data[at..at + 8].copy_from_slice(&filetime.to_le_bytes());
                    }
                    at += length;
                }
            }
            _ => {}
        }
        append_extra(&mut output, id, &data)?;
    }
    if !ut {
        let mut data = vec![1];
        data.extend_from_slice(&(seconds as u32).to_le_bytes());
        append_extra(&mut output, 0x5455, &data)?;
    }
    Ok(output)
}
fn set_modified(record: &mut Record, seconds: u64) -> Result<()> {
    let (time, date) = utc_dos_time(seconds);
    put16(&mut record.local, 10, time);
    put16(&mut record.local, 12, date);
    put16(&mut record.central, 12, time);
    put16(&mut record.central, 14, date);
    record.local_extra = timestamp_extra(&record.local_extra, seconds, false)?;
    record.central_extra = timestamp_extra(&record.central_extra, seconds, true)?;
    Ok(())
}
fn raw_compressed_size(record: &Record) -> Result<u64> {
    let overhead = match source_encryption(record) {
        EntryEncryption::None => 0,
        EntryEncryption::ZipCrypto => 12,
        EntryEncryption::Aes256 => match aes_properties(record)?.map(|(_, strength, _)| strength) {
            Some(1) => 20,
            Some(2) => 24,
            Some(3) => 28,
            _ => return Err(unsupported("ZIP edit AES strength")),
        },
    };
    record
        .compressed
        .checked_sub(overhead)
        .ok_or_else(|| malformed("ZIP edit encryption payload length"))
}
fn encryption_extra(
    extra: &[u8],
    target: EntryEncryption,
    method: u16,
    zip64: &[u8],
) -> Result<Vec<u8>> {
    let aes = [2, 0, b'A', b'E', 3, method as u8, (method >> 8) as u8];
    let mut output = Vec::new();
    let mut saw_aes = false;
    let mut saw_zip64 = false;
    for (id, value) in extras(extra)? {
        match id {
            1 => {
                saw_zip64 = true;
                if !zip64.is_empty() {
                    append_extra(&mut output, 1, zip64)?;
                }
            }
            0x9901 => {
                saw_aes = true;
                if target == EntryEncryption::Aes256 {
                    append_extra(&mut output, id, &aes)?;
                }
            }
            _ => append_extra(&mut output, id, value)?,
        }
    }
    if !saw_zip64 && !zip64.is_empty() {
        append_extra(&mut output, 1, zip64)?;
    }
    if !saw_aes && target == EntryEncryption::Aes256 {
        append_extra(&mut output, 0x9901, &aes)?;
    }
    Ok(output)
}
fn prepare_encryption(
    output: &mut Record,
    original: &Record,
    target: EntryEncryption,
) -> Result<()> {
    let actual_method = aes_properties(original)?
        .map(|(_, _, method)| method)
        .unwrap_or(u16at(&original.local, 8));
    let overhead = match target {
        EntryEncryption::None => 0,
        EntryEncryption::ZipCrypto => 12,
        EntryEncryption::Aes256 => 28,
    };
    output.compressed = add(raw_compressed_size(original)?, overhead)?;
    output.descriptor.clear();
    let flags = (u16at(&output.local, 6) & !(1 | 8)) | u16::from(target != EntryEncryption::None);
    put16(&mut output.local, 6, flags);
    put16(&mut output.central, 8, flags);
    let method = if target == EntryEncryption::Aes256 {
        99
    } else {
        actual_method
    };
    put16(&mut output.local, 8, method);
    put16(&mut output.central, 10, method);
    let wide = output.size >= u64::from(u32::MAX) || output.compressed >= u64::from(u32::MAX);
    let version = if target == EntryEncryption::Aes256 {
        51
    } else if wide {
        45
    } else {
        20
    };
    put16(&mut output.local, 4, version);
    put16(&mut output.central, 6, version);
    let mut local_zip64 = Vec::new();
    if wide {
        local_zip64.extend_from_slice(&output.size.to_le_bytes());
        local_zip64.extend_from_slice(&output.compressed.to_le_bytes());
    }
    let mut central_zip64 = Vec::new();
    if output.size >= u64::from(u32::MAX) {
        central_zip64.extend_from_slice(&output.size.to_le_bytes());
    }
    if output.compressed >= u64::from(u32::MAX) {
        central_zip64.extend_from_slice(&output.compressed.to_le_bytes());
    }
    if u32at(&output.central, 42) == u32::MAX {
        central_zip64.extend_from_slice(&output.header_offset.to_le_bytes());
    }
    output.local_extra =
        encryption_extra(&output.local_extra, target, actual_method, &local_zip64)?;
    output.central_extra =
        encryption_extra(&output.central_extra, target, actual_method, &central_zip64)?;
    put32(
        &mut output.local,
        18,
        if wide {
            u32::MAX
        } else {
            output.compressed as u32
        },
    );
    put32(
        &mut output.local,
        22,
        if wide { u32::MAX } else { output.size as u32 },
    );
    put32(
        &mut output.central,
        20,
        output.compressed.min(u64::from(u32::MAX)) as u32,
    );
    put32(
        &mut output.central,
        24,
        output.size.min(u64::from(u32::MAX)) as u32,
    );
    if target == EntryEncryption::Aes256 {
        put32(&mut output.local, 14, 0);
        put32(&mut output.central, 16, 0);
    }
    Ok(())
}
fn open_compressed<'a, R: Read + Seek>(
    reader: &'a mut R,
    record: &Record,
    password: Option<&[u8]>,
) -> Result<Box<dyn Read + 'a>> {
    reader.seek(SeekFrom::Start(record.payload_offset))?;
    match source_encryption(record) {
        EntryEncryption::None => Ok(Box::new(reader.take(record.compressed))),
        #[cfg(feature = "crypto")]
        EntryEncryption::Aes256 => {
            let (_, strength, _) =
                aes_properties(record)?.ok_or_else(|| malformed("ZIP edit missing AES profile"))?;
            Ok(Box::new(crate::crypto::decrypt(
                reader,
                record.payload_offset,
                record.compressed,
                strength,
                password.ok_or(Error::PasswordRequired)?,
            )?))
        }
        #[cfg(feature = "crypto")]
        EntryEncryption::ZipCrypto => {
            let check = if u16at(&record.local, 6) & 8 != 0 {
                record.local[11]
            } else {
                (u32at(&record.central, 16) >> 24) as u8
            };
            Ok(Box::new(crate::crypto::legacy_decrypt(
                reader.take(record.compressed),
                password.ok_or(Error::PasswordRequired)?,
                check,
            )?))
        }
        #[cfg(not(feature = "crypto"))]
        _ => {
            let _ = password;
            Err(unsupported("ZIP edit crypto feature unavailable"))
        }
    }
}
struct CancelInput<'a, R, F> {
    source: &'a mut R,
    cancelled: &'a mut F,
    was_cancelled: bool,
}
impl<R: Read, F: FnMut() -> bool> Read for CancelInput<'_, R, F> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if (self.cancelled)() {
            self.was_cancelled = true;
            return Err(std::io::Error::other("operation cancelled"));
        }
        self.source.read(bytes)
    }
}
impl<R: Seek, F> Seek for CancelInput<'_, R, F> {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        self.source.seek(from)
    }
}
#[derive(Default)]
struct CrcSink {
    crc: u32,
}
impl Write for CrcSink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.crc = ms_compress::zlib::crc32::crc32(self.crc, bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn verify_record<R: Read + Seek>(
    reader: &mut R,
    record: &Record,
    password: Option<&[u8]>,
    cancelled: &mut impl FnMut() -> bool,
    limits: Limits,
) -> Result<u32> {
    let mut source = CancelInput {
        source: reader,
        cancelled,
        was_cancelled: false,
    };
    let result = (|| {
        let mut input = open_compressed(&mut source, record, password)?;
        let method = aes_properties(record)?
            .map(|(_, _, method)| method)
            .unwrap_or(u16at(&record.local, 8));
        if method == 8
            && (limits.max_dictionary_bytes < 32768 || limits.max_active_workspace_bytes < 1 << 20)
        {
            return Err(Error::ResourceLimit("ZIP edit decoder workspace"));
        }
        let mut sink = CrcSink::default();
        let size = match method {
            0 => crate::copy_bounded(&mut input, &mut sink, limits.max_entry_bytes)?,
            8 => crate::codec::inflate(&mut input, &mut sink, false, limits.max_entry_bytes)?,
            _ => return Err(unsupported("ZIP edit codec")),
        };
        if size != record.size {
            return Err(Error::Integrity("ZIP edit decoded size mismatch".into()));
        }
        let has_crc = aes_properties(record)?.is_none_or(|(version, _, _)| version == 1);
        if has_crc && sink.crc != u32at(&record.central, 16) {
            return Err(Error::Integrity("ZIP edit CRC32 mismatch".into()));
        }
        Ok(sink.crc)
    })();
    if source.was_cancelled {
        Err(Error::Cancelled)
    } else {
        result
    }
}
fn check_options(plan: &EditPlan, options: &ZipEditOptions<'_>) -> Result<()> {
    for password in [options.password, options.new_password]
        .into_iter()
        .flatten()
    {
        if password.len() > 1 << 20 {
            return Err(Error::ResourceLimit("password bytes"));
        }
    }
    for (record, decision) in plan.records.iter().zip(&plan.entries) {
        if !decision.requires_reencryption {
            continue;
        }
        if source_encryption(record) != EntryEncryption::None && options.password.is_none() {
            return Err(Error::PasswordRequired);
        }
        let target = decision.encryption.unwrap_or(source_encryption(record));
        if target != EntryEncryption::None {
            let password = if decision.encryption.is_some() {
                options.new_password.or(options.password)
            } else {
                options.password
            };
            if password.is_none() {
                return Err(Error::PasswordRequired);
            }
            if options.randomness.is_none() {
                return Err(unsupported("ZIP edit fresh randomness required"));
            }
            if target == EntryEncryption::Aes256 && plan.limits.max_password_iterations < 1000 {
                return Err(Error::ResourceLimit("password derivation iterations"));
            }
        }
        if source_encryption(record) == EntryEncryption::Aes256
            && plan.limits.max_password_iterations < 1000
        {
            return Err(Error::ResourceLimit("password derivation iterations"));
        }
    }
    Ok(())
}
/// Validate transform credentials and decoded payload integrity before opening
/// native provisional output. Execution repeats this check against stable input.
pub fn validate_credentials(
    reader: &mut (impl Read + Seek),
    plan: &EditPlan,
    options: &ZipEditOptions<'_>,
    mut cancelled: impl FnMut() -> bool,
) -> Result<()> {
    check_options(plan, options)?;
    for (record, decision) in plan.records.iter().zip(&plan.entries) {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        if decision.requires_reencryption {
            verify_record(
                reader,
                record,
                options.password,
                &mut cancelled,
                plan.limits,
            )?;
        }
    }
    Ok(())
}

fn copy_raw_packed(input: &mut impl Read, sink: &mut impl Write, expected: u64) -> Result<()> {
    let mut remaining = expected;
    let mut buffer = [0; 65536];
    while remaining > 0 {
        let count = remaining.min(buffer.len() as u64) as usize;
        input.read_exact(&mut buffer[..count])?;
        sink.write_all(&buffer[..count])?;
        remaining -= count as u64;
    }
    if input.read(&mut [0; 1])? != 0 {
        return Err(Error::Integrity("ZIP edit compressed size changed".into()));
    }
    Ok(())
}
struct PayloadRewrite<'a> {
    target: EntryEncryption,
    old_password: Option<&'a [u8]>,
    new_password: Option<&'a [u8]>,
    crc: u32,
}
fn rewrite_payload(
    reader: &mut (impl Read + Seek),
    writer: &mut impl Write,
    record: &Record,
    rewrite: PayloadRewrite<'_>,
    random: &mut Option<&mut dyn crate::RandomSource>,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<()> {
    let mut source = CancelInput {
        source: reader,
        cancelled,
        was_cancelled: false,
    };
    let result = (|| {
        let expected = raw_compressed_size(record)?;
        let mut input = open_compressed(&mut source, record, rewrite.old_password)?;
        if rewrite.target == EntryEncryption::None {
            copy_raw_packed(&mut input, writer, expected)
        } else {
            #[cfg(feature = "crypto")]
            {
                let mode = match rewrite.target {
                    EntryEncryption::Aes256 => crate::ZipEncryption::Aes256,
                    EntryEncryption::ZipCrypto => crate::ZipEncryption::ZipCrypto,
                    EntryEncryption::None => return Err(malformed("ZIP edit encryption target")),
                };
                let mut encrypted = crate::crypto::EncryptWriter::new(
                    writer,
                    rewrite.new_password.ok_or(Error::PasswordRequired)?,
                    random
                        .as_deref_mut()
                        .ok_or_else(|| unsupported("ZIP edit fresh randomness required"))?,
                    mode,
                    (rewrite.crc >> 24) as u8,
                )?;
                copy_raw_packed(&mut input, &mut encrypted, expected)?;
                encrypted.finish()
            }
            #[cfg(not(feature = "crypto"))]
            {
                let _ = (random, rewrite.new_password, rewrite.crc);
                Err(unsupported("ZIP edit crypto feature unavailable"))
            }
        }
    })();
    if source.was_cancelled {
        Err(Error::Cancelled)
    } else {
        result
    }
}

fn relocated_extra(record: &Record, offset: u64) -> Result<(Vec<u8>, u32)> {
    let had_offset = u32at(&record.central, 42) == u32::MAX;
    let needs_offset = offset >= u64::from(u32::MAX);
    let mut output = Vec::new();
    let mut found_zip64 = false;
    for (id, value) in extras(&record.central_extra)? {
        let mut data = value.to_vec();
        if id == 1 {
            found_zip64 = true;
            if had_offset {
                data.truncate(
                    data.len()
                        .checked_sub(8)
                        .ok_or_else(|| malformed("ZIP edit missing ZIP64 offset"))?,
                );
            }
            if needs_offset {
                data.extend_from_slice(&offset.to_le_bytes());
            }
            if data.is_empty() {
                continue;
            }
        }
        output.extend_from_slice(&id.to_le_bytes());
        output.extend_from_slice(&(data.len() as u16).to_le_bytes());
        output.extend_from_slice(&data);
    }
    if needs_offset && !found_zip64 {
        output.extend_from_slice(&[1, 0, 8, 0]);
        output.extend_from_slice(&offset.to_le_bytes());
    }
    if output.len() > usize::from(u16::MAX) {
        return Err(Error::ResourceLimit("ZIP extra bytes"));
    }
    Ok((
        output,
        if needs_offset {
            u32::MAX
        } else {
            offset as u32
        },
    ))
}
/// Execute a validated plan with bounded packed copying and cancellation.
/// Rechecks source metadata before writing; caller-owned source immutability is
/// required because same-length concurrent payload changes are not detected.
/// Output must be an empty provisional artifact positioned at zero.
pub fn execute(
    reader: &mut (impl Read + Seek),
    writer: &mut (impl Write + Seek),
    edit_plan: &EditPlan,
    cancelled: impl FnMut() -> bool,
) -> Result<EditReport> {
    execute_with_options(
        reader,
        writer,
        edit_plan,
        ZipEditOptions::default(),
        cancelled,
    )
}
/// Execute with explicit credential/randomness inputs. Selected encryption
/// transforms verify old decoded payloads before writing any provisional bytes.
pub fn execute_with_options(
    reader: &mut (impl Read + Seek),
    writer: &mut (impl Write + Seek),
    edit_plan: &EditPlan,
    mut options: ZipEditOptions<'_>,
    mut cancelled: impl FnMut() -> bool,
) -> Result<EditReport> {
    if cancelled() {
        return Err(Error::Cancelled);
    }
    // Exact metadata comparison avoids hash-collision ambiguity and a second
    // in-memory metadata snapshot. Packed payloads intentionally stay opaque.
    if reader.seek(SeekFrom::End(0))? != edit_plan.source_length {
        return Err(malformed("ZIP edit source length changed"));
    }
    let mut check_buffer = [0; 65536];
    let mut compare = |reader: &mut dyn Read, expected: &[u8]| -> Result<()> {
        for bytes in expected.chunks(check_buffer.len()) {
            if cancelled() {
                return Err(Error::Cancelled);
            }
            reader.read_exact(&mut check_buffer[..bytes.len()])?;
            if &check_buffer[..bytes.len()] != bytes {
                return Err(malformed("ZIP edit source metadata changed"));
            }
        }
        Ok(())
    };
    reader.seek(SeekFrom::Start(edit_plan.trailer_offset))?;
    compare(reader, &edit_plan.source_trailer)?;
    for record in &edit_plan.records {
        reader.seek(SeekFrom::Start(record.header_offset))?;
        for bytes in [
            record.local.as_slice(),
            record.name.as_bytes(),
            &record.local_extra,
        ] {
            compare(reader, bytes)?;
        }
        reader.seek(SeekFrom::Start(record.central_offset))?;
        for bytes in [
            record.central.as_slice(),
            record.name.as_bytes(),
            &record.central_extra,
            &record.comment,
        ] {
            compare(reader, bytes)?;
        }
        reader.seek(SeekFrom::Start(add(
            record.payload_offset,
            record.compressed,
        )?))?;
        compare(reader, &record.descriptor)?;
    }
    check_options(edit_plan, &options)?;
    let mut verified_crcs = Vec::new();
    for (record, decision) in edit_plan.records.iter().zip(&edit_plan.entries) {
        verified_crcs.push(if decision.requires_reencryption {
            Some(verify_record(
                reader,
                record,
                options.password,
                &mut cancelled,
                edit_plan.limits,
            )?)
        } else {
            None
        });
    }
    if writer.seek(SeekFrom::End(0))? != 0 {
        return Err(malformed("ZIP edit output must be empty"));
    }
    writer.rewind()?;
    let mut report = EditReport {
        retained_entries: 0,
        removed_entries: 0,
        renamed_entries: 0,
        packed_bytes_copied: 0,
        payloads_verified: false,
        verified_entries: 0,
        reencrypted_entries: 0,
        metadata_changed_entries: 0,
    };
    let mut buffer = [0; 65536];
    for (entry_index, (record, decision)) in
        edit_plan.records.iter().zip(&edit_plan.entries).enumerate()
    {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let Some(name) = &decision.result_name else {
            report.removed_entries += 1;
            continue;
        };
        let output = edit_plan.output_records[entry_index]
            .as_ref()
            .ok_or_else(|| malformed("ZIP edit planned output missing"))?;
        let mut local = output.local;
        put16(&mut local, 26, name.len() as u16);
        put16(&mut local, 28, output.local_extra.len() as u16);
        if let Some(crc) = verified_crcs[entry_index]
            && decision.encryption.unwrap_or(source_encryption(record)) != EntryEncryption::Aes256
        {
            put32(&mut local, 14, crc);
        }
        writer.write_all(&local)?;
        writer.write_all(name.as_bytes())?;
        writer.write_all(&output.local_extra)?;
        if decision.requires_reencryption {
            let target = decision.encryption.unwrap_or(source_encryption(record));
            let password = if decision.encryption.is_some() {
                options.new_password.or(options.password)
            } else {
                options.password
            };
            rewrite_payload(
                reader,
                writer,
                record,
                PayloadRewrite {
                    target,
                    old_password: options.password,
                    new_password: password,
                    crc: verified_crcs[entry_index]
                        .ok_or_else(|| malformed("ZIP edit verification missing"))?,
                },
                &mut options.randomness,
                &mut cancelled,
            )?;
            report.verified_entries += 1;
            report.reencrypted_entries += 1;
        } else {
            reader.seek(SeekFrom::Start(record.payload_offset))?;
            let mut remaining = record.compressed;
            while remaining > 0 {
                if cancelled() {
                    return Err(Error::Cancelled);
                }
                let count = remaining.min(buffer.len() as u64) as usize;
                reader.read_exact(&mut buffer[..count])?;
                writer.write_all(&buffer[..count])?;
                remaining -= count as u64;
            }
            report.packed_bytes_copied = add(report.packed_bytes_copied, record.compressed)?;
        }
        writer.write_all(&output.descriptor)?;
        report.retained_entries += 1;
        report.renamed_entries += u64::from(name != &record.name);
        report.metadata_changed_entries += u64::from(decision.modified_unix_seconds.is_some());
    }
    let directory = writer.stream_position()?;
    for (entry_index, (record, decision)) in
        edit_plan.records.iter().zip(&edit_plan.entries).enumerate()
    {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let Some(offset) = edit_plan.output_offsets[entry_index] else {
            continue;
        };
        let name = decision
            .result_name
            .as_ref()
            .ok_or_else(|| malformed("ZIP edit plan decision"))?;
        let output = edit_plan.output_records[entry_index]
            .as_ref()
            .ok_or_else(|| malformed("ZIP edit planned output missing"))?;
        let extra = &output.central_extra;
        let mut central = output.central;
        put16(&mut central, 28, name.len() as u16);
        put16(&mut central, 30, extra.len() as u16);
        if let Some(crc) = verified_crcs[entry_index]
            && decision.encryption.unwrap_or(source_encryption(record)) != EntryEncryption::Aes256
        {
            put32(&mut central, 16, crc);
        }
        let _ = offset;
        writer.write_all(&central)?;
        writer.write_all(name.as_bytes())?;
        writer.write_all(extra)?;
        writer.write_all(&record.comment)?;
    }
    let size = writer
        .stream_position()?
        .checked_sub(directory)
        .ok_or_else(|| malformed("ZIP edit output offsets"))?;
    crate::zip_backend::write_end_records(writer, report.retained_entries, size, directory)?;
    writer.seek(SeekFrom::Current(-2))?;
    writer.write_all(&(edit_plan.comment.len() as u16).to_le_bytes())?;
    writer.write_all(&edit_plan.comment)?;
    report.payloads_verified =
        report.retained_entries > 0 && report.verified_entries == report.retained_entries;
    Ok(report)
}
/// Plan and reconstruct into a new caller-owned artifact. The original input is
/// never written. Cancelled/failed provisional output must be discarded by caller.
pub fn edit(
    reader: &mut (impl Read + Seek),
    writer: &mut (impl Write + Seek),
    operations: &[EditOperation],
    limits: Limits,
) -> Result<EditReport> {
    let edit_plan = plan(reader, operations, limits)?;
    execute(reader, writer, &edit_plan, || false)
}

#[cfg(all(test, feature = "crypto"))]
mod crypto_edit_tests {
    use super::*;
    use std::io::Cursor;
    struct TestRandom;
    impl crate::RandomSource for TestRandom {
        fn fill(&mut self, bytes: &mut [u8]) -> Result<()> {
            bytes.fill(41);
            Ok(())
        }
    }
    #[test]
    fn zipcrypto_verifier_collision_still_requires_decoded_crc_before_output() {
        let mut source = Cursor::new(Vec::new());
        crate::create_with_options(
            crate::Format::Zip,
            &[crate::CreateEntry {
                name: "secret".into(),
                data: b"payload".repeat(100),
                kind: crate::EntryKind::File,
            }],
            &mut source,
            Limits::default(),
            crate::CreateOptions {
                password: Some(b"correct"),
                randomness: Some(&mut TestRandom),
                zip_encryption: crate::ZipEncryption::ZipCrypto,
                zip_compression: crate::ZipCompression::Copy,
                ..Default::default()
            },
        )
        .unwrap();
        let edit_plan = plan(
            &mut source,
            &[EditOperation::SetEncryption {
                name: "secret".into(),
                encryption: EntryEncryption::None,
            }],
            Limits::default(),
        )
        .unwrap();
        let record = &edit_plan.records[0];
        let bytes = source.get_ref();
        let packed = &bytes
            [record.payload_offset as usize..(record.payload_offset + record.compressed) as usize];
        let wrong = (0u32..65536)
            .map(|value| format!("candidate-{value}").into_bytes())
            .find(|password| {
                crate::crypto::legacy_decrypt(Cursor::new(packed), password, record.local[11])
                    .is_ok()
            })
            .expect("fixture must have a wrong password with the one-byte verifier collision");
        let mut output = Cursor::new(Vec::new());
        let error = execute_with_options(
            &mut source,
            &mut output,
            &edit_plan,
            ZipEditOptions {
                password: Some(&wrong),
                ..Default::default()
            },
            || false,
        )
        .unwrap_err();
        assert!(matches!(error, Error::Integrity(message) if message == "ZIP edit CRC32 mismatch"));
        assert!(output.get_ref().is_empty());
    }
}
