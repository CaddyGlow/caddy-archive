#![cfg(feature = "streams")]
use archive_core::{Archive, CreateEntry, EntryId, EntryKind, Format, Limits, create};
use std::io::Cursor;
fn entry() -> CreateEntry {
    CreateEntry {
        name: "payload.bin".into(),
        data: b"portable stream payload".repeat(4096),
        kind: EntryKind::File,
    }
}
fn encoded(format: Format) -> Vec<u8> {
    let mut writer = Cursor::new(Vec::new());
    create(format, &[entry()], &mut writer, Limits::default()).unwrap();
    writer.into_inner()
}
#[test]
fn gzip_zlib_lzma_roundtrip_with_explicit_interpretation() {
    for format in [Format::Gzip, Format::Zlib, Format::Lzma] {
        let mut archive =
            Archive::open_as(Cursor::new(encoded(format)), format, Limits::default()).unwrap();
        assert_eq!(archive.format(), format);
        assert_eq!(
            archive.read_entry(EntryId(0), 100000).unwrap(),
            entry().data
        );
        assert!(archive.test().unwrap().verified);
    }
}
#[test]
fn gzip_members_are_checked_and_concatenated() {
    let mut data = encoded(Format::Gzip);
    data.extend_from_slice(&encoded(Format::Gzip));
    let mut archive = Archive::open(Cursor::new(data), Limits::default()).unwrap();
    assert_eq!(archive.format(), Format::Gzip);
    assert_eq!(
        archive.read_entry(EntryId(0), 200000).unwrap(),
        entry().data.repeat(2)
    );
    let mut corrupted = encoded(Format::Gzip);
    let tail = corrupted.len() - 8;
    corrupted[tail] ^= 1;
    assert!(Archive::open(Cursor::new(corrupted), Limits::default()).is_err());
}
#[test]
fn explicit_gzip_view_returns_decoded_tar_bytes() {
    let mut writer = Cursor::new(Vec::new());
    create(Format::TarGzip, &[entry()], &mut writer, Limits::default()).unwrap();
    let bytes = writer.into_inner();
    let auto = Archive::open(Cursor::new(&bytes), Limits::default()).unwrap();
    assert_eq!(auto.format(), Format::TarGzip);
    let mut raw = Archive::open_as(Cursor::new(bytes), Format::Gzip, Limits::default()).unwrap();
    let tar = raw.read_entry(EntryId(0), 200000).unwrap();
    assert_eq!(&tar[257..262], b"ustar");
}
#[test]
fn independent_gzip_and_lzma_read_and_write() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("payload.bin");
    std::fs::write(&source, entry().data).unwrap();
    for (format, tool, arguments) in [
        (Format::Gzip, "gzip", vec!["-dc"]),
        (Format::Lzma, "xz", vec!["--format=lzma", "-dc"]),
    ] {
        if std::process::Command::new(tool)
            .arg("--version")
            .output()
            .is_err()
        {
            continue;
        }
        let path = temp.path().join(if format == Format::Gzip {
            "archive.gz"
        } else {
            "archive.lzma"
        });
        std::fs::write(&path, encoded(format)).unwrap();
        let decoded = std::process::Command::new(tool)
            .args(&arguments)
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            decoded.status.success(),
            "{}",
            String::from_utf8_lossy(&decoded.stderr)
        );
        assert_eq!(decoded.stdout, entry().data);
        let mut command = std::process::Command::new(tool);
        if format == Format::Lzma {
            command.arg("--format=lzma");
        }
        let encoded = command.arg("-c").arg(&source).output().unwrap();
        assert!(encoded.status.success());
        let mut archive =
            Archive::open_as(Cursor::new(encoded.stdout), format, Limits::default()).unwrap();
        assert_eq!(
            archive.read_entry(EntryId(0), 100000).unwrap(),
            entry().data
        );
    }
}
#[test]
fn gzip_header_metadata_is_preserved_and_bounded() {
    let mut bytes = encoded(Format::Gzip);
    bytes[3] |= 8 | 16;
    let mut metadata = Vec::new();
    metadata.extend_from_slice(b"original.bin\0sample comment\0");
    let end = 10 + bytes[10..].iter().position(|byte| *byte == 0).unwrap() + 1;
    bytes.splice(10..end, metadata);
    let archive = Archive::open(Cursor::new(&bytes), Limits::default()).unwrap();
    let header = archive.gzip_header().unwrap();
    assert_eq!(header.original_name, b"original.bin");
    assert_eq!(header.comment, b"sample comment");
    let limits = Limits {
        max_metadata_bytes: 12,
        ..Default::default()
    };
    assert!(Archive::open(Cursor::new(bytes), limits).is_err());
}
