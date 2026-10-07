#![cfg(any(feature = "tar", feature = "zip"))]
use archive_core::{Archive, EntryFormatMetadata, EntryId, Limits, StoredTimestamp};
use std::io::Cursor;
#[cfg(feature = "zip")]
use std::io::Write;

#[cfg(feature = "tar")]
#[test]
fn tar_metadata_retains_stored_times_ownership_modes_and_link_bytes() {
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_ustar();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    header.set_mode(0o750);
    header.set_uid(123);
    header.set_gid(456);
    header.set_mtime(1_700_000_001);
    header.set_link_name("target.txt").unwrap();
    builder
        .append_data(&mut header, "link.txt", std::io::empty())
        .unwrap();
    let mut archive = Archive::open(
        Cursor::new(builder.into_inner().unwrap()),
        Limits::default(),
    )
    .unwrap();
    let metadata = archive.entry_metadata(EntryId(0)).unwrap();
    assert_eq!(
        metadata.modified,
        Some(StoredTimestamp::UnixSeconds(1_700_000_001))
    );
    assert_eq!(
        (metadata.unix_mode, metadata.user_id, metadata.group_id),
        (Some(0o750), Some(123), Some(456))
    );
    assert_eq!(
        metadata.link_target.as_deref(),
        Some(b"target.txt".as_slice())
    );
    assert_eq!(
        metadata.format,
        Some(EntryFormatMetadata::Tar { stored_type: b'2' })
    );
    assert!(archive.extract(EntryId(0), &mut Vec::new()).is_err());
    assert!(archive.entry_metadata(EntryId(1)).is_err());
}

#[cfg(feature = "zip")]
#[test]
fn zip_metadata_keeps_local_dos_time_mode_and_container_fields() {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let date = zip::DateTime::from_date_and_time(2024, 2, 3, 4, 5, 6).unwrap();
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(date)
        .unix_permissions(0o640);
    writer.start_file("payload", options).unwrap();
    writer.write_all(b"payload").unwrap();
    let mut archive = Archive::open(writer.finish().unwrap(), Limits::default()).unwrap();
    let metadata = archive.entry_metadata(EntryId(0)).unwrap();
    assert_eq!(
        metadata.modified,
        Some(StoredTimestamp::DosLocal {
            year: 2024,
            month: 2,
            day: 3,
            hour: 4,
            minute: 5,
            second: 6
        })
    );
    assert_eq!(metadata.unix_mode.unwrap() & 0o777, 0o640);
    assert_eq!(
        metadata.format,
        Some(EntryFormatMetadata::Zip {
            crc32: ms_compress::zlib::crc32::crc32(0, b"payload"),
            compression_method: 0,
            aes_version: None,
            aes_strength: None
        })
    );
    assert_eq!(archive.read_entry(EntryId(0), 7).unwrap(), b"payload");
}
