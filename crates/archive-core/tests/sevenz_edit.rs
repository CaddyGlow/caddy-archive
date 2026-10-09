#![cfg(all(feature = "sevenz", feature = "crypto"))]
use archive_core::sevenz_edit::{EditOperation, EditOptions, EncryptionMode, edit};
use archive_core::{
    Archive, CreateEntry, CreateOptions, EntryId, EntryKind, Error, Format, Limits, RandomSource,
};
use std::io::Cursor;

struct TestRandom(u8);
impl RandomSource for TestRandom {
    fn fill(&mut self, bytes: &mut [u8]) -> archive_core::Result<()> {
        for byte in bytes {
            self.0 = self.0.wrapping_add(1);
            *byte = self.0;
        }
        Ok(())
    }
}
fn fixture(password: Option<&[u8]>, headers: bool) -> Vec<u8> {
    let entries = [
        CreateEntry {
            name: "first.txt".into(),
            data: b"first payload".repeat(100),
            kind: EntryKind::File,
        },
        CreateEntry {
            name: "second.txt".into(),
            data: b"second payload".repeat(100),
            kind: EntryKind::File,
        },
    ];
    let mut output = Cursor::new(Vec::new());
    archive_core::create_with_options(
        Format::SevenZip,
        &entries,
        &mut output,
        Limits::default(),
        CreateOptions {
            password,
            encrypt_headers: headers,
            randomness: Some(&mut TestRandom(1)),
            ..Default::default()
        },
    )
    .unwrap();
    output.into_inner()
}

#[test]
fn whole_archive_header_encryption_password_change_and_decryption_round_trip() {
    let source = fixture(Some(b"old-secret"), true);
    let mut output = Cursor::new(Vec::new());
    let report = edit(
        &mut Cursor::new(&source),
        &mut output,
        &[
            EditOperation::SetEncryption {
                name: None,
                mode: EncryptionMode::Encrypt,
            },
            EditOperation::SetModified {
                name: None,
                modified_unix_seconds: 1_700_000_000,
            },
        ],
        EditOptions {
            old_password: Some(b"old-secret"),
            new_password: Some(b"new-secret"),
            randomness: Some(&mut TestRandom(50)),
            encrypt_headers: Some(true),
        },
        Limits::default(),
    )
    .unwrap();
    let encrypted = output.into_inner();
    assert!(report.headers_encrypted);
    assert!(report.payloads_verified);
    assert_eq!(report.encryption_entries, 2);
    assert!(matches!(
        Archive::open(Cursor::new(&encrypted), Limits::default()),
        Err(Error::PasswordRequired)
    ));
    assert!(
        Archive::open_with_password(Cursor::new(&encrypted), Limits::default(), b"old-secret")
            .is_err()
    );
    let mut archive =
        Archive::open_with_password(Cursor::new(&encrypted), Limits::default(), b"new-secret")
            .unwrap();
    archive.test().unwrap();
    assert_eq!(
        archive.entry_metadata(EntryId(0)).unwrap().modified,
        Some(archive_core::StoredTimestamp::UnixSeconds(1_700_000_000))
    );
    assert_eq!(
        archive.read_entry(EntryId(1), 2000).unwrap(),
        b"second payload".repeat(100)
    );
    let mut output = Cursor::new(Vec::new());
    let report = edit(
        &mut Cursor::new(&encrypted),
        &mut output,
        &[EditOperation::SetEncryption {
            name: None,
            mode: EncryptionMode::Decrypt,
        }],
        EditOptions {
            old_password: Some(b"new-secret"),
            encrypt_headers: Some(false),
            ..Default::default()
        },
        Limits::default(),
    )
    .unwrap();
    assert!(!report.headers_encrypted);
    let mut archive = Archive::open(Cursor::new(output.into_inner()), Limits::default()).unwrap();
    assert!(archive.entries().iter().all(|entry| !entry.encrypted));
    archive.test().unwrap();
}

#[test]
fn hidden_header_partial_rekey_requires_explicit_plain_header_policy() {
    let source = fixture(Some(b"old-secret"), true);
    let operation = [EditOperation::SetEncryption {
        name: Some("first.txt".into()),
        mode: EncryptionMode::Encrypt,
    }];
    let mut output = Cursor::new(Vec::new());
    let error = edit(
        &mut Cursor::new(&source),
        &mut output,
        &operation,
        EditOptions {
            old_password: Some(b"old-secret"),
            new_password: Some(b"new-secret"),
            randomness: Some(&mut TestRandom(50)),
            ..Default::default()
        },
        Limits::default(),
    )
    .unwrap_err();
    assert!(matches!(error, Error::Unsupported(_)));
    assert!(output.into_inner().is_empty());
    assert!(!error.to_string().contains("old-secret"));
    assert!(!error.to_string().contains("new-secret"));
    let mut output = Cursor::new(Vec::new());
    edit(
        &mut Cursor::new(&source),
        &mut output,
        &operation,
        EditOptions {
            old_password: Some(b"old-secret"),
            new_password: Some(b"new-secret"),
            randomness: Some(&mut TestRandom(50)),
            encrypt_headers: Some(false),
        },
        Limits::default(),
    )
    .unwrap();
    let bytes = output.into_inner();
    let mut archive =
        Archive::open_with_password(Cursor::new(&bytes), Limits::default(), b"new-secret").unwrap();
    assert_eq!(
        archive.read_entry(EntryId(0), 2000).unwrap(),
        b"first payload".repeat(100)
    );
    let mut archive =
        Archive::open_with_password(Cursor::new(&bytes), Limits::default(), b"old-secret").unwrap();
    assert_eq!(
        archive.read_entry(EntryId(1), 2000).unwrap(),
        b"second payload".repeat(100)
    );
}

#[test]
fn source_solid_folder_subset_rejects_and_metadata_reuses_packed_stream() {
    let bytes = include_bytes!("fixtures/sevenz-independent/solid-lzma2.7z");
    let original = Archive::open(Cursor::new(bytes), Limits::default()).unwrap();
    let name = original
        .entries()
        .iter()
        .find(|entry| entry.kind == EntryKind::File && entry.size > 0)
        .unwrap()
        .name
        .clone();
    let mut output = Cursor::new(Vec::new());
    assert!(matches!(
        edit(
            &mut Cursor::new(bytes),
            &mut output,
            &[EditOperation::SetEncryption {
                name: Some(name.clone()),
                mode: EncryptionMode::Encrypt
            }],
            EditOptions {
                new_password: Some(b"new-secret"),
                randomness: Some(&mut TestRandom(50)),
                ..Default::default()
            },
            Limits::default()
        ),
        Err(Error::Unsupported(_))
    ));
    assert!(output.into_inner().is_empty());
    let mut output = Cursor::new(Vec::new());
    let report = edit(
        &mut Cursor::new(bytes),
        &mut output,
        &[EditOperation::SetModified {
            name: Some(name),
            modified_unix_seconds: 1_700_000_000,
        }],
        EditOptions::default(),
        Limits::default(),
    )
    .unwrap();
    assert!(report.packed_bytes_copied > 0);
    assert_eq!(report.packed_bytes_transformed, 0);
    Archive::open(Cursor::new(output.into_inner()), Limits::default())
        .unwrap()
        .test()
        .unwrap();
}

#[test]
fn global_empty_entries_report_no_payload_and_header_encryption_still_hides_names() {
    let mut source = Cursor::new(Vec::new());
    archive_core::create(
        Format::SevenZip,
        &[CreateEntry {
            name: "empty.txt".into(),
            data: Vec::new(),
            kind: EntryKind::File,
        }],
        &mut source,
        Limits::default(),
    )
    .unwrap();
    let mut output = Cursor::new(Vec::new());
    let report = edit(
        &mut Cursor::new(source.into_inner()),
        &mut output,
        &[EditOperation::SetEncryption {
            name: None,
            mode: EncryptionMode::Encrypt,
        }],
        EditOptions {
            new_password: Some(b"header-secret"),
            randomness: Some(&mut TestRandom(50)),
            encrypt_headers: Some(true),
            ..Default::default()
        },
        Limits::default(),
    )
    .unwrap();
    assert_eq!(report.encryption_entries, 0);
    assert_eq!(report.entries_without_payload, 1);
    assert!(report.headers_encrypted);
    let mut archive = Archive::open_with_password(
        Cursor::new(output.into_inner()),
        Limits::default(),
        b"header-secret",
    )
    .unwrap();
    assert_eq!(archive.entries()[0].size, 0);
    archive.test().unwrap();
}

#[test]
#[ignore = "requires pinned 7-Zip 26.04 executable via ARCHIVE_7Z_REFERENCE"]
fn pinned_reference_reads_timestamp_solid_and_encryption_transforms() {
    use std::process::Command;
    let reference =
        std::env::var_os("ARCHIVE_7Z_REFERENCE").expect("set ARCHIVE_7Z_REFERENCE to pinned 7zz");
    let version = Command::new(&reference).arg("i").output().unwrap();
    assert!(String::from_utf8_lossy(&version.stdout).contains("7-Zip (z) 26.04"));
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("first.txt"), b"first payload".repeat(100)).unwrap();
    std::fs::write(
        root.path().join("second.txt"),
        b"second payload".repeat(100),
    )
    .unwrap();
    let status = Command::new(&reference)
        .current_dir(root.path())
        .args([
            "a",
            "-t7z",
            "-ms=on",
            "-mx=1",
            "solid.7z",
            "first.txt",
            "second.txt",
        ])
        .output()
        .unwrap();
    assert!(status.status.success());
    let source = std::fs::read(root.path().join("solid.7z")).unwrap();
    let mut output = Cursor::new(Vec::new());
    edit(
        &mut Cursor::new(&source),
        &mut output,
        &[EditOperation::SetModified {
            name: Some("first.txt".into()),
            modified_unix_seconds: 1_700_000_000,
        }],
        EditOptions::default(),
        Limits::default(),
    )
    .unwrap();
    std::fs::write(root.path().join("timestamp.7z"), output.into_inner()).unwrap();
    let check = Command::new(&reference)
        .current_dir(root.path())
        .env("TZ", "UTC")
        .args(["l", "-slt", "timestamp.7z"])
        .output()
        .unwrap();
    assert!(check.status.success());
    assert!(String::from_utf8_lossy(&check.stdout).contains("Modified = 2023-11-14 22:13:20"));
    let mut encrypted = Cursor::new(Vec::new());
    edit(
        &mut Cursor::new(&source),
        &mut encrypted,
        &[EditOperation::SetEncryption {
            name: None,
            mode: EncryptionMode::Encrypt,
        }],
        EditOptions {
            new_password: Some(b"reference-secret"),
            randomness: Some(&mut TestRandom(50)),
            encrypt_headers: Some(true),
            ..Default::default()
        },
        Limits::default(),
    )
    .unwrap();
    let encrypted = encrypted.into_inner();
    std::fs::write(root.path().join("encrypted.7z"), &encrypted).unwrap();
    let check = Command::new(&reference)
        .current_dir(root.path())
        .args(["t", "-preference-secret", "encrypted.7z"])
        .output()
        .unwrap();
    assert!(check.status.success(), "reference rejected edited archive");
    let check = Command::new(&reference)
        .current_dir(root.path())
        .args(["x", "-preference-secret", "-oextracted", "encrypted.7z"])
        .output()
        .unwrap();
    assert!(check.status.success());
    assert_eq!(
        std::fs::read(root.path().join("extracted/first.txt")).unwrap(),
        b"first payload".repeat(100)
    );
    assert_eq!(
        std::fs::read(root.path().join("extracted/second.txt")).unwrap(),
        b"second payload".repeat(100)
    );
    let mut decrypted = Cursor::new(Vec::new());
    edit(
        &mut Cursor::new(&encrypted),
        &mut decrypted,
        &[EditOperation::SetEncryption {
            name: None,
            mode: EncryptionMode::Decrypt,
        }],
        EditOptions {
            old_password: Some(b"reference-secret"),
            encrypt_headers: Some(false),
            ..Default::default()
        },
        Limits::default(),
    )
    .unwrap();
    std::fs::write(root.path().join("decrypted.7z"), decrypted.into_inner()).unwrap();
    let check = Command::new(&reference)
        .current_dir(root.path())
        .args(["t", "decrypted.7z"])
        .output()
        .unwrap();
    assert!(check.status.success());
}

#[test]
fn reserved_source_data_and_header_budget_fail_before_output() {
    use ms_compress::zlib::crc32::crc32;
    let mut source = fixture(None, false);
    let offset = u64::from_le_bytes(source[12..20].try_into().unwrap());
    source.insert((32 + offset) as usize, 0xa5);
    source[12..20].copy_from_slice(&(offset + 1).to_le_bytes());
    let crc = crc32(0, &source[12..32]);
    source[8..12].copy_from_slice(&crc.to_le_bytes());
    let mut output = Cursor::new(Vec::new());
    assert!(matches!(
        edit(
            &mut Cursor::new(source),
            &mut output,
            &[EditOperation::SetModified {
                name: None,
                modified_unix_seconds: 1_700_000_000
            }],
            EditOptions::default(),
            Limits::default()
        ),
        Err(Error::Unsupported(_))
    ));
    assert!(output.into_inner().is_empty());
    let mut source = Cursor::new(Vec::new());
    archive_core::create(Format::SevenZip, &[], &mut source, Limits::default()).unwrap();
    let mut output = Cursor::new(Vec::new());
    let limits = Limits {
        max_metadata_bytes: 32,
        ..Limits::default()
    };
    assert!(matches!(
        edit(
            &mut Cursor::new(source.into_inner()),
            &mut output,
            &[],
            EditOptions {
                new_password: Some(b"header-secret"),
                randomness: Some(&mut TestRandom(50)),
                encrypt_headers: Some(true),
                ..Default::default()
            },
            limits
        ),
        Err(Error::ResourceLimit(_))
    ));
    assert!(output.into_inner().is_empty());
}
