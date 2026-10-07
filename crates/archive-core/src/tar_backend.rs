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
        let size = header.size()?;
        if matches!(raw[156], b'x' | b'g' | b'L' | b'K') {
            extensions = extensions
                .checked_add(size)
                .ok_or(Error::ResourceLimit("metadata bytes"))?;
            if extensions > limits.max_metadata_bytes {
                return Err(Error::ResourceLimit("metadata bytes"));
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
    let mut builder = tar::Builder::new(writer);
    for (index, entry) in entries.iter().enumerate() {
        let mut header = tar::Header::new_ustar();
        header.set_mode(if entry.kind == EntryKind::Directory {
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
        match entry.kind {
            EntryKind::File => {
                header.set_entry_type(tar::EntryType::Regular);
                header.set_size(entry.data.len() as u64);
            }
            EntryKind::Directory => {
                header.set_entry_type(tar::EntryType::Directory);
                header.set_size(0);
            }
            _ => return Err(Error::Unsupported("TAR link creation".into())),
        }
        if header.set_path(&entry.name).is_err() {
            builder.append_pax_extensions([("path", entry.name.as_bytes())])?;
            header.set_path("PaxPayload")?;
        }
        header.set_cksum();
        builder.append(
            &header,
            if entry.kind == EntryKind::File {
                entry.data.as_slice()
            } else {
                &[]
            },
        )?;
    }
    builder.finish()?;
    Ok(())
}
