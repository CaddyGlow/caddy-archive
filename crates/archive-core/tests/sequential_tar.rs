#![cfg(feature = "tar")]
use archive_core::{CreateEntry, EntryKind, Format, Limits, create, sequential_tar::SequentialTar};
use std::io::{Cursor, Read};
struct ForwardOnly(Cursor<Vec<u8>>);
impl Read for ForwardOnly {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(bytes)
    }
}
#[test]
fn forward_only_reader_preserves_pax_names_and_payloads() {
    let entries = [
        CreateEntry {
            name: format!("{}/file", "long".repeat(100)),
            data: b"first".to_vec(),
            kind: EntryKind::File,
        },
        CreateEntry {
            name: "second.txt".into(),
            data: b"second".to_vec(),
            kind: EntryKind::File,
        },
    ];
    let mut output = Cursor::new(Vec::new());
    create(Format::Tar, &entries, &mut output, Limits::default()).unwrap();
    let mut archive = SequentialTar::new(
        ForwardOnly(Cursor::new(output.into_inner())),
        Limits::default(),
    );
    for expected in entries {
        let entry = archive.next_entry().unwrap().unwrap();
        assert_eq!(entry.name, expected.name);
        assert!(archive.next_entry().is_err());
        let mut bytes = Vec::new();
        assert!(archive.copy_current(&mut bytes).unwrap().verified);
        assert_eq!(bytes, expected.data);
    }
    assert!(archive.next_entry().unwrap().is_none());
}
#[test]
fn metadata_limit_applies_before_extension_allocation() {
    let entries = [CreateEntry {
        name: "x".repeat(400),
        data: Vec::new(),
        kind: EntryKind::File,
    }];
    let mut output = Cursor::new(Vec::new());
    create(Format::Tar, &entries, &mut output, Limits::default()).unwrap();
    let mut archive = SequentialTar::new(
        ForwardOnly(Cursor::new(output.into_inner())),
        Limits {
            max_metadata_bytes: 100,
            ..Default::default()
        },
    );
    assert!(archive.next_entry().is_err());
}
#[test]
fn missing_payload_or_end_marker_fails() {
    let entries = [CreateEntry {
        name: "data".into(),
        data: vec![0; 1024],
        kind: EntryKind::File,
    }];
    let mut output = Cursor::new(Vec::new());
    create(Format::Tar, &entries, &mut output, Limits::default()).unwrap();
    let data = output.into_inner();
    let mut archive = SequentialTar::new(
        ForwardOnly(Cursor::new(data[..600].to_vec())),
        Limits::default(),
    );
    assert!(archive.next_entry().unwrap().is_some());
    assert!(archive.skip_current().is_err());
    let mut archive = SequentialTar::new(
        ForwardOnly(Cursor::new(data[..1536].to_vec())),
        Limits::default(),
    );
    archive.next_entry().unwrap();
    archive.skip_current().unwrap();
    assert!(archive.next_entry().is_err());
}
