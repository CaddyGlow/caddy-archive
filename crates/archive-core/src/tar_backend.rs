use crate::{CreateEntry, Entry, EntryId, EntryKind, Error, Limits, Result};
use std::io::{Read, Seek, SeekFrom, Write};
pub(crate) fn metadata(
    reader: &mut (impl Read + Seek),
    payload_offset: u64,
) -> Result<crate::EntryMetadata> {
    let header_offset = payload_offset
        .checked_sub(512)
        .ok_or_else(|| Error::Malformed("TAR metadata offset".into()))?;
    reader.seek(SeekFrom::Start(header_offset))?;
    let mut bytes = [0; 512];
    reader.read_exact(&mut bytes)?;
    let header = tar::Header::from_byte_slice(&bytes);
    Ok(crate::EntryMetadata {
        modified: Some(crate::StoredTimestamp::UnixSeconds(header.mtime()?)),
        unix_mode: Some(header.mode()?),
        user_id: Some(header.uid()?),
        group_id: Some(header.gid()?),
        link_target: header.link_name_bytes().map(|value| value.into_owned()),
        format: Some(crate::EntryFormatMetadata::Tar {
            stored_type: bytes[156],
        }),
    })
}
pub(crate) fn index(
    reader: &mut (impl Read + Seek),
    limits: Limits,
) -> Result<(Vec<Entry>, Vec<u64>)> {
    preflight(reader, limits)?;
    let mut archive = tar::Archive::new(reader);
    let mut entries = Vec::new();
    let mut positions = Vec::new();
    let mut metadata = 0u64;
    for member in archive.entries()? {
        let member = member?;
        if entries.len() as u64 >= limits.max_entries {
            return Err(Error::ResourceLimit("entries"));
        }
        let raw = member.path_bytes().into_owned();
        metadata = metadata
            .checked_add(raw.len() as u64)
            .ok_or(Error::ResourceLimit("metadata bytes"))?;
        if metadata > limits.max_metadata_bytes {
            return Err(Error::ResourceLimit("metadata bytes"));
        }
        let kind = member.header().entry_type();
        let kind = if kind.is_file() {
            EntryKind::File
        } else if kind.is_dir() {
            EntryKind::Directory
        } else if kind.is_symlink() || kind.is_hard_link() {
            EntryKind::Link
        } else {
            EntryKind::Other
        };
        positions.push(member.raw_file_position());
        entries.push(Entry {
            id: EntryId(entries.len()),
            name: String::from_utf8_lossy(&raw).into_owned(),
            raw_name: raw,
            kind,
            size: member.size(),
            compressed_size: Some(member.size()),
            compression: "stored".into(),
            encrypted: false,
        });
    }
    Ok((entries, positions))
}
fn preflight(reader: &mut (impl Read + Seek), limits: Limits) -> Result<()> {
    reader.rewind()?;
    let length = reader.seek(SeekFrom::End(0))?;
    reader.rewind()?;
    let mut headers = 0u64;
    let mut extensions = 0u64;
    let mut position = 0u64;
    let mut pax_size = None;
    loop {
        if position == length {
            break;
        }
        let mut raw = [0u8; 512];
        reader.read_exact(&mut raw)?;
        if raw.iter().all(|b| *b == 0) {
            break;
        }
        headers = headers
            .checked_add(1)
            .ok_or(Error::ResourceLimit("entries"))?;
        if headers > limits.max_entries.saturating_mul(2) {
            return Err(Error::ResourceLimit("TAR headers"));
        }
        let header = tar::Header::from_byte_slice(&raw);
        let mut size = header.size()?;
        if matches!(raw[156], b'x' | b'g' | b'L' | b'K') {
            extensions = extensions
                .checked_add(size)
                .ok_or(Error::ResourceLimit("metadata bytes"))?;
            if extensions > limits.max_metadata_bytes {
                return Err(Error::ResourceLimit("metadata bytes"));
            }
        }
        // Match the indexer's local PAX size override, including intervening
        // GNU long-name/link headers. Bound extension allocations before reading.
        if raw[156] == b'x' && (header.as_ustar().is_some() || header.as_gnu().is_some()) {
            let length =
                usize::try_from(size).map_err(|_| Error::ResourceLimit("metadata bytes"))?;
            let mut body = vec![0; length];
            reader.read_exact(&mut body)?;
            pax_size = None;
            for extension in tar::PaxExtensions::new(&body) {
                let extension = extension?;
                if extension.key_bytes() == b"size" {
                    pax_size = Some(
                        extension
                            .value()
                            .ok()
                            .and_then(|value| value.parse::<u64>().ok())
                            .ok_or_else(|| Error::Malformed("PAX size".into()))?,
                    );
                    break;
                }
            }
        } else if !matches!(raw[156], b'L' | b'K') {
            if raw[156] != b'g' {
                size = pax_size.take().unwrap_or(size);
            } else {
                // Global headers are exposed as entries by the indexed TAR reader.
                pax_size = None;
            }
        }
        position = position
            .checked_add(512)
            .and_then(|v| {
                size.checked_add(511)
                    .and_then(|s| v.checked_add(s / 512 * 512))
            })
            .ok_or(Error::Malformed("TAR offset overflow".into()))?;
        if position > length {
            return Err(Error::Malformed("TAR member exceeds input".into()));
        }
        reader.seek(SeekFrom::Start(position))?;
    }
    reader.rewind()?;
    Ok(())
}
pub(crate) fn create(entries: &[CreateEntry], writer: &mut impl Write) -> Result<()> {
    create_with_metadata(entries, writer, None)
}
pub(crate) fn create_with_metadata(
    entries: &[CreateEntry],
    writer: &mut impl Write,
    metadata: Option<&[crate::EntryMetadata]>,
) -> Result<()> {
    create_readers(
        entries,
        &mut |index| {
            Ok(Box::new(std::io::Cursor::new(
                entries[index].data.as_slice(),
            )))
        },
        writer,
        metadata,
    )
}
pub(crate) fn create_readers<'a, E: crate::CreationEntry>(
    entries: &[E],
    open: &mut impl FnMut(usize) -> Result<Box<dyn Read + 'a>>,
    writer: &mut impl Write,
    metadata: Option<&[crate::EntryMetadata]>,
) -> Result<()> {
    let mut builder = tar::Builder::new(writer);
    for (index, entry) in entries.iter().enumerate() {
        let mut header = tar::Header::new_ustar();
        header.set_mode(if entry.source_kind() == EntryKind::Directory {
            0o755
        } else {
            0o644
        });
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        if let Some(metadata) = metadata.and_then(|values| values.get(index)) {
            if let Some(mode) = metadata.unix_mode {
                header.set_mode(mode & 0o7777);
            }
            if let Some(crate::StoredTimestamp::UnixSeconds(seconds)) = metadata.modified {
                header.set_mtime(seconds);
            }
            if let Some(uid) = metadata.user_id {
                header.set_uid(uid);
            }
            if let Some(gid) = metadata.group_id {
                header.set_gid(gid);
            }
        }
        match entry.source_kind() {
            EntryKind::File => {
                header.set_entry_type(tar::EntryType::Regular);
                header.set_size(entry.source_size());
            }
            EntryKind::Directory => {
                header.set_entry_type(tar::EntryType::Directory);
                header.set_size(0);
            }
            _ => return Err(Error::Unsupported("TAR link creation".into())),
        }
        if header.set_path(entry.source_name()).is_err() {
            builder.append_pax_extensions([("path", entry.source_name().as_bytes())])?;
            header.set_path("PaxPayload")?;
        }
        header.set_cksum();
        if entry.source_kind() == EntryKind::File {
            let mut source = open(index)?;
            let mut payload = source.by_ref().take(entry.source_size());
            builder.append(&header, &mut payload)?;
            if payload.limit() != 0 || source.read(&mut [0u8; 1])? != 0 {
                return Err(Error::Malformed("creation source size changed".into()));
            }
        } else {
            builder.append(&header, std::io::empty())?;
        }
    }
    builder.finish()?;
    Ok(())
}
