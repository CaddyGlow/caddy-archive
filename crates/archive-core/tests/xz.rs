#![cfg(feature = "xz")]
use archive_core::{Archive, CreateEntry, EntryId, EntryKind, Format, Limits, create};
use std::io::Cursor;
fn input() -> CreateEntry {
    CreateEntry {
        name: "data".into(),
        data: (0..=255).cycle().take(100000).collect(),
        kind: EntryKind::File,
    }
}
fn encoded(format: Format) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    create(format, &[input()], &mut output, Limits::default()).unwrap();
    output.into_inner()
}
#[test]
fn xz_and_tar_xz_roundtrip() {
    for format in [Format::Xz, Format::TarXz] {
        let mut archive = Archive::open(Cursor::new(encoded(format)), Limits::default()).unwrap();
        assert_eq!(archive.format(), format);
        assert_eq!(
            archive.read_entry(EntryId(0), 100000).unwrap(),
            input().data
        );
    }
}
#[test]
fn xz_concatenated_streams_decode_in_order() {
    let mut bytes = encoded(Format::Xz);
    bytes.extend_from_slice(&[0; 8]);
    bytes.extend_from_slice(&encoded(Format::Xz));
    let mut archive = Archive::open(Cursor::new(bytes), Limits::default()).unwrap();
    let expected: Vec<_> = input()
        .data
        .iter()
        .chain(input().data.iter())
        .copied()
        .collect();
    assert_eq!(archive.read_entry(EntryId(0), 200000).unwrap(), expected);
}
#[test]
fn independent_xz_decodes_output() {
    if std::process::Command::new("xz")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("archive.xz");
    std::fs::write(&path, encoded(Format::Xz)).unwrap();
    let output = std::process::Command::new("xz")
        .args(["-dc"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, input().data);
}
#[test]
fn independent_multiblock_checks_and_filters_decode() {
    if std::process::Command::new("xz")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("data.bin");
    std::fs::write(&path, input().data).unwrap();
    for (check, filter) in [
        ("crc32", None),
        ("crc64", Some("--x86")),
        ("sha256", Some("--delta=dist=3")),
        ("none", None),
    ] {
        let mut command = std::process::Command::new("xz");
        command
            .args(["-c", "--block-size=32768"])
            .arg(format!("--check={check}"));
        if let Some(filter) = filter {
            command.arg(filter).arg("--lzma2");
        }
        let encoded = command.arg(&path).output().unwrap();
        assert!(
            encoded.status.success(),
            "{}",
            String::from_utf8_lossy(&encoded.stderr)
        );
        let mut archive = Archive::open(Cursor::new(encoded.stdout), Limits::default()).unwrap();
        assert_eq!(
            archive.read_entry(EntryId(0), 100000).unwrap(),
            input().data
        );
    }
}
#[test]
fn xz_truncation_and_footer_corruption_fail() {
    let bytes = encoded(Format::Xz);
    for length in [0, 6, 12, bytes.len() / 2, bytes.len() - 1] {
        assert!(Archive::open(Cursor::new(&bytes[..length]), Limits::default()).is_err());
    }
    let mut bytes = bytes;
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    assert!(Archive::open(Cursor::new(bytes), Limits::default()).is_err());
}
