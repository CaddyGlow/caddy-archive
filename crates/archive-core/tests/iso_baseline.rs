#![cfg(feature = "iso")]
use archive_core::{Archive, EntryId, Format, Limits};
use std::io::Cursor;

fn both32(bytes: &mut [u8], value: u32) {
    bytes[..4].copy_from_slice(&value.to_le_bytes());
    bytes[4..8].copy_from_slice(&value.to_be_bytes());
}
fn both16(bytes: &mut [u8], value: u16) {
    bytes[..2].copy_from_slice(&value.to_le_bytes());
    bytes[2..4].copy_from_slice(&value.to_be_bytes());
}
fn record(name: &[u8], extent: u32, size: u32, directory: bool) -> Vec<u8> {
    let len = (33 + name.len() + 1) & !1;
    let mut bytes = vec![0; len];
    bytes[0] = len as u8;
    both32(&mut bytes[2..10], extent);
    both32(&mut bytes[10..18], size);
    bytes[18..25].copy_from_slice(&[126, 10, 5, 12, 0, 0, 0]);
    bytes[25] = if directory { 2 } else { 0 };
    both16(&mut bytes[28..32], 1);
    bytes[32] = name.len() as u8;
    bytes[33..33 + name.len()].copy_from_slice(name);
    bytes
}
fn fixture() -> Vec<u8> {
    let mut image = vec![0; 22 * 2048];
    let primary = &mut image[16 * 2048..17 * 2048];
    primary[0] = 1;
    primary[1..6].copy_from_slice(b"CD001");
    primary[6] = 1;
    both32(&mut primary[80..88], 22);
    both16(&mut primary[120..124], 1);
    both16(&mut primary[124..128], 1);
    both16(&mut primary[128..132], 2048);
    let root = record(&[0], 20, 2048, true);
    primary[156..156 + root.len()].copy_from_slice(&root);
    image[17 * 2048] = 255;
    image[17 * 2048 + 1..17 * 2048 + 6].copy_from_slice(b"CD001");
    image[17 * 2048 + 6] = 1;
    let mut offset = 20 * 2048;
    for bytes in [
        root,
        record(&[1], 20, 2048, true),
        record(b"HELLO.TXT;1", 21, 5, false),
    ] {
        image[offset..offset + bytes.len()].copy_from_slice(&bytes);
        offset += bytes.len();
    }
    image[21 * 2048..21 * 2048 + 5].copy_from_slice(b"hello");
    image
}

#[test]
fn iso_system_area_is_not_misidentified_as_empty_tar() {
    let mut archive = Archive::open(Cursor::new(fixture()), Limits::default()).unwrap();
    assert_eq!(archive.format(), Format::Iso);
    assert_eq!(archive.entries()[0].name, "HELLO.TXT");
    assert_eq!(archive.read_entry(EntryId(0), 5).unwrap(), b"hello");
}

#[test]
fn malformed_root_cycle_is_rejected() {
    let mut bytes = fixture();
    let record = record(b"LOOP", 20, 2048, true);
    bytes[20 * 2048 + 68..20 * 2048 + 68 + record.len()].copy_from_slice(&record);
    assert!(Archive::open(Cursor::new(bytes), Limits::default()).is_err());
}

#[test]
fn independent_7z_reads_baseline_fixture_when_available() {
    if std::process::Command::new("7z").arg("i").output().is_err() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("baseline.iso");
    std::fs::write(&path, fixture()).unwrap();
    let output = std::process::Command::new("7z")
        .args(["e", "-so"])
        .arg(path)
        .arg("HELLO.TXT")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"hello");
}

#[test]
fn multi_extent_adapter_reads_noncontiguous_sections_in_file_order() {
    let mut image = fixture();
    image.resize(24 * 2048, 0);
    both32(&mut image[16 * 2048 + 80..16 * 2048 + 88], 24);
    let offset = 20 * 2048 + 68;
    let mut first = record(b"DATA.BIN;1", 21, 2048, false);
    first[25] = 0x80;
    let second = record(b"DATA.BIN;1", 23, 7, false);
    image[offset..21 * 2048].fill(0);
    image[offset..offset + first.len()].copy_from_slice(&first);
    image[offset + first.len()..offset + first.len() + second.len()].copy_from_slice(&second);
    image[21 * 2048..22 * 2048].fill(0x31);
    image[22 * 2048..23 * 2048].fill(0x99);
    image[23 * 2048..23 * 2048 + 7].copy_from_slice(b"trailer");
    let mut archive = Archive::open(Cursor::new(image), Limits::default()).unwrap();
    assert_eq!(archive.entries().len(), 1);
    assert_eq!(archive.entries()[0].size, 2055);
    let mut expected = vec![0x31; 2048];
    expected.extend(b"trailer");
    assert_eq!(archive.read_entry(EntryId(0), 2055).unwrap(), expected);
}
