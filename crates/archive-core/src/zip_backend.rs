use crate::{CreateEntry, Entry, EntryId, EntryKind, Error, Limits, Result, codec, copy_bounded};
use std::io::{Read, Seek, SeekFrom, Write};
pub(crate) struct Location {
    pub(crate) offset: u64,
    pub(crate) compressed: u64,
    pub(crate) method: u16,
    pub(crate) crc: u32,
    #[cfg_attr(not(feature = "crypto"), allow(dead_code))]
    aes: Option<(u16, u8)>,
    pub(crate) encrypted: bool,
    header: u64,
    pub(crate) metadata: crate::EntryMetadata,
}
#[allow(deprecated)]
pub(crate) fn index<R: Read + Seek>(
    mut reader: R,
    limits: Limits,
) -> Result<(R, Vec<Entry>, Vec<Location>)> {
    let declared_entries = preflight(&mut reader, limits)?;
    let mut zip = zip::ZipArchive::new(reader).map_err(|e| Error::Malformed(e.to_string()))?;
    // The dependency indexes by name and can silently discard duplicate entries.
    if zip.len() as u64 != declared_entries {
        return Err(Error::Unsupported(
            "ZIP index omits entries (possibly duplicate filenames)".into(),
        ));
    }
    if zip.len() as u64 > limits.max_entries {
        return Err(Error::ResourceLimit("entries"));
    }
    let mut entries = Vec::new();
    let mut locations = Vec::new();
    let mut metadata = 0u64;
    for i in 0..zip.len() {
        let file = zip
            .by_index_raw(i)
            .map_err(|e| Error::Malformed(e.to_string()))?;
        let raw = file.name_raw().to_vec();
        metadata = metadata
            .checked_add(raw.len() as u64)
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<crate::EntryMetadata>() as u64))
            .ok_or(Error::ResourceLimit("metadata bytes"))?;
        if metadata > limits.max_metadata_bytes {
            return Err(Error::ResourceLimit("metadata bytes"));
        }
        let method = file.compression().to_u16();
        if method == 8 && limits.max_dictionary_bytes < 32768 {
            return Err(Error::ResourceLimit("dictionary bytes"));
        }
        if method == 8 && limits.max_active_workspace_bytes < 1 << 20 {
            return Err(Error::ResourceLimit("active decoder workspace bytes"));
        }
        let aes = aes_extra(file.extra_data().unwrap_or_default())?;
        if aes.is_some() && limits.max_password_iterations < 1000 {
            return Err(Error::ResourceLimit("password derivation iterations"));
        }
        locations.push(Location {
            offset: file.data_start(),
            compressed: file.compressed_size(),
            method,
            crc: file.crc32(),
            aes,
            encrypted: file.encrypted(),
            header: file.header_start(),
            metadata: crate::EntryMetadata {
                modified: unix_mtime(file.extra_data().unwrap_or_default())?.or_else(|| {
                    file.last_modified()
                        .map(|date| crate::StoredTimestamp::DosLocal {
                            year: date.year(),
                            month: date.month(),
                            day: date.day(),
                            hour: date.hour(),
                            minute: date.minute(),
                            second: date.second(),
                        })
                }),
                unix_mode: file.unix_mode(),
                format: Some(crate::EntryFormatMetadata::Zip {
                    crc32: file.crc32(),
                    compression_method: method,
                    aes_version: aes.map(|value| value.0),
                    aes_strength: aes.map(|value| value.1),
                }),
                ..Default::default()
            },
        });
        let kind = if file.is_dir() {
            EntryKind::Directory
        } else if file.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            EntryKind::Link
        } else {
            EntryKind::File
        };
        entries.push(Entry {
            id: EntryId(i),
            raw_name: raw,
            name: file.name().into(),
            kind,
            size: file.size(),
            compressed_size: Some(file.compressed_size()),
            compression: format!("{:?}", file.compression()),
            encrypted: file.encrypted(),
        });
    }
    let mut reader = zip.into_inner();
    // Include local headers in the ranges: a member hidden inside another
    // member's payload is also an overlapping ZIP bomb.
    let mut ranges: Vec<_> = locations
        .iter()
        .map(|location| {
            let end = location
                .offset
                .checked_add(location.compressed)
                .ok_or_else(|| Error::Malformed("ZIP payload range overflow".into()))?;
            Ok((location.header, end))
        })
        .collect::<Result<_>>()?;
    ranges.sort_unstable();
    if ranges.windows(2).any(|pair| pair[1].0 < pair[0].1) {
        return Err(Error::Malformed("overlapping ZIP members".into()));
    }
    let length = reader.seek(SeekFrom::End(0))?;
    for (location, entry) in locations.iter().zip(&entries) {
        if location
            .offset
            .checked_add(location.compressed)
            .is_none_or(|end| end > length)
        {
            return Err(Error::Malformed("ZIP payload exceeds input".into()));
        }
        reader.seek(SeekFrom::Start(location.header))?;
        let mut local = [0u8; 30];
        reader.read_exact(&mut local)?;
        if !local.starts_with(b"PK\x03\x04") {
            return Err(Error::Malformed("ZIP local header signature".into()));
        }
        let flags = u16::from_le_bytes([local[6], local[7]]);
        let method = u16::from_le_bytes([local[8], local[9]]);
        if flags & 0x2040 != 0 {
            return Err(Error::Unsupported("ZIP strong/header encryption".into()));
        }
        if (flags & 1 != 0) != location.encrypted
            || method
                != if location.aes.is_some() {
                    99
                } else {
                    location.method
                }
        {
            return Err(Error::Malformed(
                "ZIP local and central flags/method mismatch".into(),
            ));
        }
        let name_len = usize::from(u16::from_le_bytes([local[26], local[27]]));
        if name_len as u64 > limits.max_metadata_bytes {
            return Err(Error::ResourceLimit("metadata bytes"));
        }
        let mut name = vec![0u8; name_len];
        reader.read_exact(&mut name)?;
        if name != entry.raw_name {
            return Err(Error::Malformed(
                "ZIP local and central filename mismatch".into(),
            ));
        }
    }
    Ok((reader, entries, locations))
}
fn unix_mtime(mut extra: &[u8]) -> Result<Option<crate::StoredTimestamp>> {
    while extra.len() >= 4 {
        let id = u16::from_le_bytes([extra[0], extra[1]]);
        let len = usize::from(u16::from_le_bytes([extra[2], extra[3]]));
        extra = &extra[4..];
        if len > extra.len() {
            return Err(Error::Malformed("ZIP extra field length".into()));
        }
        if id == 0x5455 && len >= 5 && extra[0] & 1 != 0 {
            return Ok(Some(crate::StoredTimestamp::UnixSeconds(u64::from(
                u32::from_le_bytes(
                    extra[1..5]
                        .try_into()
                        .map_err(|_| Error::Malformed("ZIP timestamp".into()))?,
                ),
            ))));
        }
        extra = &extra[len..];
    }
    Ok(None)
}
fn aes_extra(mut extra: &[u8]) -> Result<Option<(u16, u8)>> {
    while extra.len() >= 4 {
        let id = u16::from_le_bytes([extra[0], extra[1]]);
        let len = usize::from(u16::from_le_bytes([extra[2], extra[3]]));
        extra = &extra[4..];
        if len > extra.len() {
            return Err(Error::Malformed("ZIP extra field length".into()));
        }
        if id == 0x9901 {
            if len != 7 || extra[2..4] != *b"AE" {
                return Err(Error::Malformed("WinZip AES extra field".into()));
            }
            return Ok(Some((u16::from_le_bytes([extra[0], extra[1]]), extra[4])));
        }
        extra = &extra[len..];
    }
    Ok(None)
}
fn preflight(reader: &mut (impl Read + Seek), limits: Limits) -> Result<u64> {
    let length = reader.seek(SeekFrom::End(0))?;
    let tail_len = length.min(65557);
    reader.seek(SeekFrom::Start(length - tail_len))?;
    let mut tail = vec![0; tail_len as usize];
    reader.read_exact(&mut tail)?;
    let eocd = tail
        .windows(4)
        .enumerate()
        .rfind(|(offset, signature)| {
            if *signature != b"PK\x05\x06" || tail.len() - offset < 22 {
                return false;
            }
            let comment = u16::from_le_bytes([tail[offset + 20], tail[offset + 21]]);
            offset + 22 + usize::from(comment) == tail.len()
        })
        .map(|(offset, _)| offset)
        .ok_or_else(|| Error::Malformed("ZIP end record missing".into()))?;
    if tail.len() - eocd < 22 {
        return Err(Error::Malformed("truncated ZIP end record".into()));
    }
    let record = &tail[eocd..];
    let mut count = u64::from(u16::from_le_bytes([record[10], record[11]]));
    let mut size = u64::from(u32::from_le_bytes(
        record[12..16]
            .try_into()
            .map_err(|_| Error::Malformed("ZIP end record".into()))?,
    ));
    if count == u64::from(u16::MAX) || size == u64::from(u32::MAX) {
        let absolute = length - tail_len + eocd as u64;
        if absolute < 20 {
            return Err(Error::Malformed("ZIP64 locator missing".into()));
        }
        reader.seek(SeekFrom::Start(absolute - 20))?;
        let mut locator = [0u8; 20];
        reader.read_exact(&mut locator)?;
        if !locator.starts_with(b"PK\x06\x07") {
            return Err(Error::Malformed("ZIP64 locator missing".into()));
        }
        let offset = u64::from_le_bytes(
            locator[8..16]
                .try_into()
                .map_err(|_| Error::Malformed("ZIP64 locator".into()))?,
        );
        reader.seek(SeekFrom::Start(offset))?;
        let mut zip64 = [0u8; 56];
        reader.read_exact(&mut zip64)?;
        if !zip64.starts_with(b"PK\x06\x06") {
            return Err(Error::Malformed("ZIP64 end record missing".into()));
        }
        count = u64::from_le_bytes(
            zip64[32..40]
                .try_into()
                .map_err(|_| Error::Malformed("ZIP64 count".into()))?,
        );
        size = u64::from_le_bytes(
            zip64[40..48]
                .try_into()
                .map_err(|_| Error::Malformed("ZIP64 size".into()))?,
        );
    }
    if count > limits.max_entries {
        return Err(Error::ResourceLimit("entries"));
    }
    if size
        > limits
            .max_metadata_bytes
            .checked_add(count.saturating_mul(46))
            .ok_or(Error::ResourceLimit("metadata bytes"))?
    {
        return Err(Error::ResourceLimit("metadata bytes"));
    }
    reader.rewind()?;
    Ok(count)
}
struct CheckedWriter<'a, W> {
    writer: &'a mut W,
    crc: u32,
}
impl<W: Write> Write for CheckedWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let n = self.writer.write(bytes)?;
        self.crc = ms_compress::zlib::crc32::crc32(self.crc, &bytes[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}
pub(crate) fn extract(
    reader: &mut (impl Read + Seek),
    location: &Location,
    writer: &mut impl Write,
    limit: u64,
    password: Option<&[u8]>,
) -> Result<u64> {
    if location.encrypted {
        let password = password.ok_or(Error::PasswordRequired)?;
        #[cfg(feature = "crypto")]
        {
            if let Some((version, strength)) = location.aes {
                let mut input = crate::crypto::decrypt(
                    reader,
                    location.offset,
                    location.compressed,
                    strength,
                    password,
                )?;
                let mut checked = CheckedWriter { writer, crc: 0 };
                let size = decode(&mut input, location.method, &mut checked, limit)?;
                if version == 1 && checked.crc != location.crc {
                    return Err(Error::Integrity("ZIP CRC32 mismatch".into()));
                }
                return Ok(size);
            }
        }
        #[cfg(feature = "crypto")]
        {
            reader.seek(SeekFrom::Start(location.header))?;
            let mut local = [0u8; 30];
            reader.read_exact(&mut local)?;
            let flags = u16::from_le_bytes([local[6], local[7]]);
            let check = if flags & 8 != 0 {
                local[11]
            } else {
                (location.crc >> 24) as u8
            };
            reader.seek(SeekFrom::Start(location.offset))?;
            let mut input =
                crate::crypto::legacy_decrypt(reader.take(location.compressed), password, check)?;
            let mut checked = CheckedWriter { writer, crc: 0 };
            let size = decode(&mut input, location.method, &mut checked, limit)?;
            if checked.crc != location.crc {
                return Err(Error::Integrity("ZIP CRC32 mismatch".into()));
            }
            return Ok(size);
        }
        #[cfg(not(feature = "crypto"))]
        {
            let _ = password;
            return Err(Error::Unsupported(
                "legacy ZIP encryption feature unavailable".into(),
            ));
        }
    }
    reader.seek(SeekFrom::Start(location.offset))?;
    let mut input = reader.take(location.compressed);
    let mut checked = CheckedWriter { writer, crc: 0 };
    let size = match location.method {
        0 => copy_bounded(&mut input, &mut checked, limit)?,
        8 => codec::inflate(&mut input, &mut checked, false, limit)?,
        m => return Err(Error::Unsupported(format!("ZIP compression method {m}"))),
    };
    if input.limit() != 0 {
        return Err(Error::Integrity("truncated ZIP compressed data".into()));
    }
    if checked.crc != location.crc {
        return Err(Error::Integrity("ZIP CRC32 mismatch".into()));
    }
    Ok(size)
}
#[cfg(feature = "crypto")]
fn decode(reader: &mut impl Read, method: u16, writer: &mut impl Write, limit: u64) -> Result<u64> {
    match method {
        0 => copy_bounded(reader, writer, limit),
        8 => codec::inflate(reader, writer, false, limit),
        m => Err(Error::Unsupported(format!("ZIP compression method {m}"))),
    }
}
fn u16le(w: &mut impl Write, v: u16) -> Result<()> {
    w.write_all(&v.to_le_bytes())?;
    Ok(())
}
fn u32le(w: &mut impl Write, v: u32) -> Result<()> {
    w.write_all(&v.to_le_bytes())?;
    Ok(())
}
fn u64le(w: &mut impl Write, v: u64) -> Result<()> {
    w.write_all(&v.to_le_bytes())?;
    Ok(())
}
/// Always writes ZIP64 sizes and end records, allowing one uniform checked profile.
pub(crate) fn create(entries: &[CreateEntry], writer: &mut (impl Write + Seek)) -> Result<()> {
    create_impl(
        entries,
        writer,
        None,
        crate::ZipEncryption::Aes256,
        None,
        crate::ZipCompression::Deflate,
    )
}
pub(crate) fn create_with_compression(
    entries: &[CreateEntry],
    writer: &mut (impl Write + Seek),
    metadata: Option<&[crate::EntryMetadata]>,
    compression: crate::ZipCompression,
) -> Result<()> {
    create_impl(
        entries,
        writer,
        None,
        crate::ZipEncryption::Aes256,
        metadata,
        compression,
    )
}
#[cfg(feature = "crypto")]
pub(crate) fn create_encrypted(
    entries: &[CreateEntry],
    writer: &mut (impl Write + Seek),
    password: &[u8],
    random: &mut dyn crate::RandomSource,
    mode: crate::ZipEncryption,
    metadata: Option<&[crate::EntryMetadata]>,
    compression: crate::ZipCompression,
) -> Result<()> {
    create_impl(
        entries,
        writer,
        Some((password, random)),
        mode,
        metadata,
        compression,
    )
}
fn create_impl(
    entries: &[CreateEntry],
    writer: &mut (impl Write + Seek),
    mut encryption: Option<(&[u8], &mut dyn crate::RandomSource)>,
    mode: crate::ZipEncryption,
    metadata: Option<&[crate::EntryMetadata]>,
    compression: crate::ZipCompression,
) -> Result<()> {
    let mut central = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let metadata = metadata.and_then(|values| values.get(index));
        let (dos_time, dos_date) = match metadata.and_then(|value| value.modified) {
            Some(crate::StoredTimestamp::DosLocal {
                year,
                month,
                day,
                hour,
                minute,
                second,
            }) => {
                zip::DateTime::from_date_and_time(year, month, day, hour, minute, second)
                    .map_err(|_| Error::Malformed("invalid ZIP DOS timestamp".into()))?;
                (
                    (u16::from(hour) << 11) | (u16::from(minute) << 5) | u16::from(second / 2),
                    ((year - 1980) << 9) | (u16::from(month) << 5) | u16::from(day),
                )
            }
            _ => (0, 0x21),
        };
        let mut timestamp = Vec::new();
        if let Some(crate::StoredTimestamp::UnixSeconds(seconds)) =
            metadata.and_then(|value| value.modified)
        {
            let seconds = u32::try_from(seconds)
                .map_err(|_| Error::Unsupported("ZIP Unix timestamp exceeds 32 bits".into()))?;
            timestamp.extend_from_slice(&[0x55, 0x54, 5, 0, 1]);
            timestamp.extend_from_slice(&seconds.to_le_bytes());
        }
        let timestamp_len = timestamp.len() as u16;
        if !matches!(entry.kind, EntryKind::File | EntryKind::Directory) {
            return Err(Error::Unsupported("ZIP link creation".into()));
        }
        let name = if entry.kind == EntryKind::Directory && !entry.name.ends_with('/') {
            format!("{}/", entry.name)
        } else {
            entry.name.clone()
        };
        let name = name.as_bytes();
        let name_len =
            u16::try_from(name.len()).map_err(|_| Error::ResourceLimit("ZIP filename bytes"))?;
        let data = if entry.kind == EntryKind::Directory {
            &[][..]
        } else {
            entry.data.as_slice()
        };
        let mut compressed = Vec::new();
        let actual_method = if compression == crate::ZipCompression::Copy {
            compressed.extend_from_slice(data);
            0
        } else {
            codec::deflate(data, &mut compressed, false)?;
            8
        };
        let original_crc = ms_compress::zlib::crc32::crc32(0, data);
        #[cfg(feature = "crypto")]
        if let Some((password, random)) = &mut encryption {
            match mode {
                crate::ZipEncryption::Aes256 => {
                    crate::crypto::encrypt(&mut compressed, password, *random)?
                }
                crate::ZipEncryption::ZipCrypto => crate::crypto::legacy_encrypt(
                    &mut compressed,
                    password,
                    (original_crc >> 24) as u8,
                    *random,
                )?,
            }
        }
        let _ = &mut encryption;
        let encrypted = encryption.is_some();
        let aes = encrypted && mode == crate::ZipEncryption::Aes256;
        let method = if aes { 99 } else { actual_method };
        let flags = if encrypted { 0x801 } else { 0x800 };
        let crc = if aes { 0 } else { original_crc };
        let offset = writer.stream_position()?;
        u32le(writer, 0x04034b50)?;
        u16le(writer, 45)?;
        u16le(writer, flags)?;
        u16le(writer, method)?;
        u16le(writer, dos_time)?;
        u16le(writer, dos_date)?;
        u32le(writer, crc)?;
        u32le(writer, u32::MAX)?;
        u32le(writer, u32::MAX)?;
        u16le(writer, name_len)?;
        u16le(writer, (if aes { 31 } else { 20 }) + timestamp_len)?;
        writer.write_all(name)?;
        u16le(writer, 1)?;
        u16le(writer, 16)?;
        u64le(writer, data.len() as u64)?;
        u64le(writer, compressed.len() as u64)?;
        if aes {
            writer.write_all(&[1, 0x99, 7, 0, 2, 0, b'A', b'E', 3, actual_method as u8, 0])?;
        }
        writer.write_all(&timestamp)?;
        writer.write_all(&compressed)?;
        u32le(&mut central, 0x02014b50)?;
        u16le(&mut central, if metadata.is_some() { 0x032d } else { 45 })?;
        u16le(&mut central, 45)?;
        u16le(&mut central, flags)?;
        u16le(&mut central, method)?;
        u16le(&mut central, dos_time)?;
        u16le(&mut central, dos_date)?;
        u32le(&mut central, crc)?;
        u32le(&mut central, u32::MAX)?;
        u32le(&mut central, u32::MAX)?;
        u16le(&mut central, name_len)?;
        u16le(&mut central, (if aes { 39 } else { 28 }) + timestamp_len)?;
        u16le(&mut central, 0)?;
        u16le(&mut central, 0)?;
        u16le(&mut central, 0)?;
        u32le(
            &mut central,
            (metadata.and_then(|value| value.unix_mode).unwrap_or(0) << 16)
                | if entry.kind == EntryKind::Directory {
                    0x10
                } else {
                    0
                },
        )?;
        u32le(&mut central, u32::MAX)?;
        central.write_all(name)?;
        u16le(&mut central, 1)?;
        u16le(&mut central, 24)?;
        u64le(&mut central, data.len() as u64)?;
        u64le(&mut central, compressed.len() as u64)?;
        u64le(&mut central, offset)?;
        if aes {
            central.write_all(&[1, 0x99, 7, 0, 2, 0, b'A', b'E', 3, actual_method as u8, 0])?;
        }
        central.write_all(&timestamp)?;
    }
    let directory = writer.stream_position()?;
    writer.write_all(&central)?;
    let zip64 = writer.stream_position()?;
    u32le(writer, 0x06064b50)?;
    u64le(writer, 44)?;
    u16le(writer, 45)?;
    u16le(writer, 45)?;
    u32le(writer, 0)?;
    u32le(writer, 0)?;
    u64le(writer, entries.len() as u64)?;
    u64le(writer, entries.len() as u64)?;
    u64le(writer, central.len() as u64)?;
    u64le(writer, directory)?;
    u32le(writer, 0x07064b50)?;
    u32le(writer, 0)?;
    u64le(writer, zip64)?;
    u32le(writer, 1)?;
    u32le(writer, 0x06054b50)?;
    u16le(writer, 0)?;
    u16le(writer, 0)?;
    u16le(writer, u16::MAX)?;
    u16le(writer, u16::MAX)?;
    u32le(writer, u32::MAX)?;
    u32le(writer, u32::MAX)?;
    u16le(writer, 0)?;
    Ok(())
}
