#![cfg(all(feature = "zip", feature = "tar", feature = "gzip", feature = "cab"))]
use archive_core::{Archive, CreateEntry, EntryId, EntryKind, Error, Format, Limits, create};
use std::io::Cursor;
fn entries() -> Vec<CreateEntry> {
    vec![
        CreateEntry {
            name: "hello.txt".into(),
            data: b"archive payload".to_vec(),
            kind: EntryKind::File,
        },
        CreateEntry {
            name: "second.bin".into(),
            data: (0..=255).cycle().take(130000).collect(),
            kind: EntryKind::File,
        },
    ]
}
#[test]
fn zip_duplicate_names_never_silently_omit_members() {
    let entries = vec![
        CreateEntry {
            name: "first.txt".into(),
            data: b"first payload".to_vec(),
            kind: EntryKind::File,
        },
        CreateEntry {
            name: "other.txt".into(),
            data: b"other payload".to_vec(),
            kind: EntryKind::File,
        },
    ];
    let mut output = Cursor::new(Vec::new());
    create(Format::Zip, &entries, &mut output, Limits::default()).unwrap();
    let mut bytes = output.into_inner();
    // Equal-length names keep the fixture's local and central header sizes valid.
    let positions: Vec<_> = bytes
        .windows(9)
        .enumerate()
        .filter_map(|(position, bytes)| (bytes == b"other.txt").then_some(position))
        .collect();
    assert_eq!(positions.len(), 2);
    for position in positions {
        bytes[position..position + 9].copy_from_slice(b"first.txt");
    }
    assert!(matches!(
        Archive::open(Cursor::new(bytes), Limits::default()),
        Err(Error::Unsupported(_))
    ));
}
#[test]
fn declared_writable_profiles_roundtrip_and_test() {
    for format in [Format::Zip, Format::Tar, Format::TarGzip, Format::Cab] {
        let mut output = Cursor::new(Vec::new());
        create(format, &entries(), &mut output, Limits::default()).unwrap();
        let mut archive =
            Archive::open(Cursor::new(output.into_inner()), Limits::default()).unwrap();
        assert_eq!(archive.entries().len(), 2);
        assert_eq!(
            archive.read_entry(EntryId(0), 1024).unwrap(),
            b"archive payload"
        );
        assert_eq!(
            archive.read_entry(EntryId(1), 140000).unwrap(),
            entries()[1].data
        );
        assert!(archive.test().unwrap().verified);
    }
}
#[test]
fn declared_and_buffered_limits_are_enforced() {
    let mut output = Cursor::new(Vec::new());
    create(Format::Zip, &entries(), &mut output, Limits::default()).unwrap();
    let data = output.into_inner();
    let limits = Limits {
        max_entry_bytes: 100,
        ..Limits::default()
    };
    assert!(matches!(
        Archive::open(Cursor::new(&data), limits),
        Err(Error::ResourceLimit(_))
    ));
    let mut archive = Archive::open(Cursor::new(data), Limits::default()).unwrap();
    assert!(matches!(
        archive.read_entry(EntryId(1), 10),
        Err(Error::ResourceLimit(_))
    ));
}
#[test]
fn crc_corruption_is_never_verified() {
    let mut output = Cursor::new(Vec::new());
    create(Format::Zip, &entries()[..1], &mut output, Limits::default()).unwrap();
    let mut data = output.into_inner();
    let position = data.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    data[position + 16] ^= 1;
    let mut archive = Archive::open(Cursor::new(data), Limits::default()).unwrap();
    assert!(matches!(archive.test(), Err(Error::Integrity(_))));
}
#[test]
fn gzip_truncation_is_rejected() {
    let mut output = Cursor::new(Vec::new());
    create(Format::TarGzip, &entries(), &mut output, Limits::default()).unwrap();
    let mut data = output.into_inner();
    data.truncate(data.len() - 3);
    assert!(matches!(
        Archive::open(Cursor::new(data), Limits::default()),
        Err(Error::Integrity(_))
    ));
}
#[test]
fn zip64_writer_is_readable_by_independent_container_parser() {
    let mut output = Cursor::new(Vec::new());
    create(Format::Zip, &entries(), &mut output, Limits::default()).unwrap();
    let zip = zip::ZipArchive::new(Cursor::new(output.into_inner())).unwrap();
    assert_eq!(zip.len(), 2);
}
#[test]
fn pax_long_path_preserves_name_and_payload() {
    let name = format!("{}/file.txt", "directory".repeat(50));
    let entry = CreateEntry {
        name: name.clone(),
        data: b"PAX payload".to_vec(),
        kind: EntryKind::File,
    };
    let mut output = Cursor::new(Vec::new());
    create(Format::Tar, &[entry], &mut output, Limits::default()).unwrap();
    let data = output.into_inner();
    assert_eq!(data[156], b'x');
    let mut archive = Archive::open(Cursor::new(data), Limits::default()).unwrap();
    assert_eq!(archive.entries()[0].name, name);
    assert_eq!(archive.read_entry(EntryId(0), 100).unwrap(), b"PAX payload");
}
