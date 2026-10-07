#![cfg(all(feature = "bzip2", feature = "brotli"))]
use archive_core::{Archive, CreateEntry, EntryId, EntryKind, Format, Limits, create_stream};
use std::io::Cursor;

#[test]
fn bzip2_brotli_and_tar_profiles_roundtrip_and_budget() {
    for format in [
        Format::Bzip2,
        Format::Brotli,
        Format::TarBzip2,
        Format::TarBrotli,
    ] {
        let entry = CreateEntry {
            name: "payload.txt".into(),
            kind: EntryKind::File,
            data: b"portable compression".repeat(1024),
        };
        let mut bytes = Vec::new();
        create_stream(
            format,
            std::slice::from_ref(&entry),
            &mut bytes,
            Limits::default(),
        )
        .unwrap();
        let mut archive = Archive::open_as(Cursor::new(&bytes), format, Limits::default()).unwrap();
        assert_eq!(
            archive
                .read_entry(EntryId(0), entry.data.len() as u64)
                .unwrap(),
            entry.data
        );
        let limits = Limits {
            max_active_workspace_bytes: 1024,
            ..Limits::default()
        };
        assert!(Archive::open_as(Cursor::new(&bytes), format, limits).is_err());
        assert!(
            Archive::open_as(
                Cursor::new(&bytes[..bytes.len() / 2]),
                format,
                Limits::default()
            )
            .is_err()
        );
    }
}

#[test]
fn independent_bzip2_decodes_created_stream() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let entry = CreateEntry {
        name: "data".into(),
        kind: EntryKind::File,
        data: b"independent bzip2".repeat(1000),
    };
    let mut bytes = Vec::new();
    create_stream(
        Format::Bzip2,
        std::slice::from_ref(&entry),
        &mut bytes,
        Limits::default(),
    )
    .unwrap();
    let mut child = Command::new("bzip2")
        .args(["-dc"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("bzip2 required");
    child.stdin.take().unwrap().write_all(&bytes).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, entry.data);
}

#[test]
fn forward_raw_deflate_gzip_and_zlib_roundtrip_and_limits() {
    for format in [Format::Deflate, Format::Gzip, Format::Zlib] {
        let data = b"forward only deflate".repeat(1000);
        let mut encoded = Vec::new();
        archive_core::deflate_stream(
            &mut data.as_slice(),
            &mut encoded,
            format,
            Limits::default(),
        )
        .unwrap();
        let mut decoded = Vec::new();
        archive_core::inflate_stream(
            &mut encoded.as_slice(),
            &mut decoded,
            format,
            Limits::default(),
        )
        .unwrap();
        assert_eq!(data, decoded);
        let limits = Limits {
            max_total_bytes: 1,
            ..Limits::default()
        };
        assert!(
            archive_core::inflate_stream(&mut encoded.as_slice(), &mut Vec::new(), format, limits)
                .is_err()
        );
        assert!(
            archive_core::deflate_stream(&mut data.as_slice(), &mut Vec::new(), format, limits)
                .is_err()
        );
    }
}
