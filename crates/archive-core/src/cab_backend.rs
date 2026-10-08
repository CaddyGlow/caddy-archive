//! Reader-based CAB creation with one bounded solid-folder encoder.
use crate::{
    CabCompression, CreateOptions, CreationEntry, EntryKind, Error, Limits, Result, StoredTimestamp,
};
use std::io::{self, Read, Seek, Write};

pub(crate) fn create_readers<'a, E: CreationEntry>(
    entries: &[E],
    open: &mut impl FnMut(usize) -> Result<Box<dyn Read + 'a>>,
    writer: &mut (impl Write + Seek),
    limits: Limits,
    options: &CreateOptions<'_>,
) -> Result<()> {
    if options.password.is_some() {
        return Err(Error::Unsupported("CAB encryption".into()));
    }
    let (dictionary, workspace, compression) = match options.cab_compression {
        CabCompression::Copy => (0, 32768, cabinet::WriteCompression::None),
        CabCompression::MsZip => (32768, 1 << 20, cabinet::WriteCompression::MsZip),
        CabCompression::Lzx => (
            2 << 20,
            64 << 20,
            cabinet::WriteCompression::Lzx { window_order: 21 },
        ),
        CabCompression::Quantum => (
            2 << 20,
            64 << 20,
            cabinet::WriteCompression::Quantum {
                level: 6,
                window_order: 21,
            },
        ),
    };
    if dictionary > limits.max_dictionary_bytes || workspace > limits.max_active_workspace_bytes {
        return Err(Error::ResourceLimit("CAB encoder workspace bytes"));
    }
    let mut builder = cabinet::CabinetBuilder::new(compression);
    for (index, entry) in entries.iter().enumerate() {
        if entry.source_kind() != EntryKind::File {
            return Err(Error::Unsupported("CAB directories and links".into()));
        }
        let metadata = options.entry_metadata.and_then(|values| values.get(index));
        if let Some(StoredTimestamp::DosLocal {
            year,
            month,
            day,
            hour,
            minute,
            second,
        }) = metadata.and_then(|value| value.modified)
        {
            if !(1980..=2107).contains(&year) {
                return Err(Error::Unsupported(
                    "CAB timestamp year must be 1980 through 2107".into(),
                ));
            }
            let date = ((year - 1980) << 9) | (u16::from(month) << 5) | u16::from(day);
            let time = (u16::from(hour) << 11) | (u16::from(minute) << 5) | u16::from(second / 2);
            let attributes = 0x20
                | if metadata
                    .and_then(|value| value.unix_mode)
                    .is_some_and(|mode| mode & 0o222 == 0)
                {
                    1
                } else {
                    0
                };
            builder.add_file_source_with_metadata(
                entry.source_name(),
                entry.source_size(),
                date,
                time,
                attributes,
            )?;
        } else {
            builder.add_file_source(entry.source_name(), entry.source_size())?;
        }
    }
    let mut source_error = None;
    let result = builder.write_from_readers(writer, &mut |index| {
        open(index).map_err(|error| {
            source_error = Some(error);
            io::Error::other("CAB source opening failed")
        })
    });
    if let Some(error) = source_error {
        return Err(error);
    }
    result?;
    Ok(())
}
