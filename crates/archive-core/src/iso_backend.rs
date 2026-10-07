//! Archive adapter for the libmkiso ISO9660 parser.
use crate::{Entry, EntryId, EntryKind, Error, Limits, Result};
use std::io::{Read, Seek};

pub(crate) fn index<R: Read + Seek>(
    reader: &mut R,
    limits: Limits,
) -> Result<(Vec<Entry>, Vec<Vec<libmkiso::iso9660::Extent>>)> {
    let index = libmkiso::iso9660::read_index(
        reader,
        libmkiso::iso9660::Limits {
            max_entries: limits.max_entries,
            max_metadata_bytes: limits.max_metadata_bytes,
            max_nesting_depth: limits.max_nesting_depth,
        },
    )
    .map_err(|error| match error {
        libmkiso::iso9660::Error::Io(error) => Error::Io(error),
        libmkiso::iso9660::Error::Malformed(message) => Error::Malformed(message),
        libmkiso::iso9660::Error::Unsupported(message) => Error::Unsupported(message),
        libmkiso::iso9660::Error::ResourceLimit(budget) => Error::ResourceLimit(budget),
    })?;
    let entries = index
        .entries
        .into_iter()
        .enumerate()
        .map(|(id, entry)| Entry {
            id: EntryId(id),
            raw_name: entry.raw_name,
            name: entry.name,
            kind: if entry.directory {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            size: entry.size,
            compressed_size: Some(entry.size),
            compression: "stored".into(),
            encrypted: false,
        })
        .collect();
    Ok((entries, index.extents))
}
