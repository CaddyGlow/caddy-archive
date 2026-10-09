#![cfg(feature = "zip")]
use archive_core::{
    Archive, CreateEntry, CreateOptions, EntryId, EntryKind, Error, Format, Limits, ZipCompression,
    create_with_options,
    zip_edit::{self, EditOperation},
};
use std::io::{Cursor, Write};

fn fixture() -> Vec<u8> {
    let entries: Vec<_> = ["dir/", "dir/a", "directory/a", "erase"]
        .into_iter()
        .map(|name| CreateEntry {
            name: name.into(),
            data: if name.ends_with('/') {
                Vec::new()
            } else {
                name.as_bytes().repeat(1024)
            },
            kind: if name.ends_with('/') {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
        })
        .collect();
    let mut output = Cursor::new(Vec::new());
    create_with_options(
        Format::Zip,
        &entries,
        &mut output,
        Limits::default(),
        CreateOptions {
            zip_compression: ZipCompression::Copy,
            ..Default::default()
        },
    )
    .unwrap();
    output.into_inner()
}
fn raw_payload(bytes: &[u8], name: &str) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let index = zip
        .file_names()
        .position(|candidate| candidate == name)
        .unwrap();
    let entry = zip.by_index_raw(index).unwrap();
    bytes[entry.data_start() as usize..(entry.data_start() + entry.compressed_size()) as usize]
        .to_vec()
}
#[test]
fn zip_edit_renames_directory_at_path_boundaries_and_deletes_without_recompressing() {
    let original = fixture();
    let mut input = Cursor::new(original.clone());
    let mut output = Cursor::new(Vec::new());
    let ops = [
        EditOperation::Rename {
            from: "dir/".into(),
            to: "new/".into(),
        },
        EditOperation::Delete {
            name: "erase".into(),
        },
    ];
    let plan = zip_edit::plan(&mut input, &ops, Limits::default()).unwrap();
    assert_eq!(plan.entries()[1].result_name.as_deref(), Some("new/a"));
    let report = zip_edit::execute(&mut input, &mut output, &plan, || false).unwrap();
    assert_eq!(
        (
            report.retained_entries,
            report.removed_entries,
            report.renamed_entries
        ),
        (3, 1, 2)
    );
    assert!(!report.payloads_verified);
    let changed = output.into_inner();
    assert_eq!(
        raw_payload(&original, "dir/a"),
        raw_payload(&changed, "new/a")
    );
    let mut archive = Archive::open(Cursor::new(&changed), Limits::default()).unwrap();
    assert_eq!(
        archive
            .entries()
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["new/", "new/a", "directory/a"]
    );
    assert_eq!(
        archive.read_entry(EntryId(1), 100_000).unwrap(),
        b"dir/a".repeat(1024)
    );
    assert_eq!(input.into_inner(), original);
}
#[test]
fn zip_edit_preserves_archive_comments_dos_metadata_and_attributes() {
    let date = zip::DateTime::from_date_and_time(2024, 2, 3, 4, 5, 6).unwrap();
    let mut source = zip::ZipWriter::new(Cursor::new(Vec::new()));
    source.set_comment("comment PK\u{5}\u{6}");
    source
        .start_file(
            "old",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .last_modified_time(date)
                .unix_permissions(0o640),
        )
        .unwrap();
    source.write_all(b"payload").unwrap();
    let mut input = source.finish().unwrap();
    let mut output = Cursor::new(Vec::new());
    zip_edit::edit(
        &mut input,
        &mut output,
        &[EditOperation::Rename {
            from: "old".into(),
            to: "new-name".into(),
        }],
        Limits::default(),
    )
    .unwrap();
    let mut reader = zip::ZipArchive::new(output).unwrap();
    assert_eq!(reader.comment(), b"comment PK\x05\x06");
    let entry = reader.by_index_raw(0).unwrap();
    assert_eq!(entry.name(), "new-name");
    assert_eq!(entry.last_modified(), Some(date));
    assert_eq!(entry.unix_mode().unwrap() & 0o777, 0o640);
}
#[test]
fn zip_edit_collisions_overlapping_operations_and_missing_sources_fail_before_output() {
    for ops in [
        vec![EditOperation::Rename {
            from: "dir/".into(),
            to: "directory/".into(),
        }],
        vec![
            EditOperation::Delete {
                name: "dir/".into(),
            },
            EditOperation::Rename {
                from: "dir/a".into(),
                to: "other".into(),
            },
        ],
        vec![EditOperation::Delete {
            name: "missing".into(),
        }],
        vec![EditOperation::Rename {
            from: "erase".into(),
            to: "dir".into(),
        }],
    ] {
        let mut output = Cursor::new(Vec::new());
        assert!(
            zip_edit::edit(
                &mut Cursor::new(fixture()),
                &mut output,
                &ops,
                Limits::default()
            )
            .is_err()
        );
        assert!(output.into_inner().is_empty());
    }
}
#[test]
fn zip_edit_deleting_every_entry_creates_an_independently_readable_empty_archive() {
    let mut output = Cursor::new(Vec::new());
    zip_edit::edit(
        &mut Cursor::new(fixture()),
        &mut output,
        &[
            EditOperation::Delete {
                name: "dir/".into(),
            },
            EditOperation::Delete {
                name: "directory/".into(),
            },
            EditOperation::Delete {
                name: "erase".into(),
            },
        ],
        Limits::default(),
    )
    .unwrap();
    assert_eq!(zip::ZipArchive::new(output).unwrap().len(), 0);
}
#[test]
fn zip_edit_unknown_extras_split_trailing_and_prefix_profiles_fail_before_output() {
    let mut source = zip::ZipWriter::new(Cursor::new(Vec::new()));
    source
        .start_file(
            "name",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .large_file(true),
        )
        .unwrap();
    source.write_all(b"payload").unwrap();
    let mut unknown = source.finish().unwrap().into_inner();
    // ZIP64 is the first local extra in this independently authored fixture.
    unknown[34..36].copy_from_slice(&0xbeefu16.to_le_bytes());
    let mut split = fixture();
    let end = split.len() - 22;
    split[end + 4] = 1;
    let mut trailing = fixture();
    trailing.extend_from_slice(b"trailing");
    let mut prefixed = b"prefix".to_vec();
    prefixed.extend_from_slice(&fixture());
    for bytes in [unknown, split, trailing, prefixed] {
        let mut output = Cursor::new(Vec::new());
        assert!(
            zip_edit::edit(&mut Cursor::new(bytes), &mut output, &[], Limits::default()).is_err()
        );
        assert!(output.into_inner().is_empty());
    }
}
#[test]
fn zip_edit_metadata_change_and_cancellation_leave_original_untouched() {
    let original = fixture();
    let mut input = Cursor::new(original.clone());
    let plan = zip_edit::plan(&mut input, &[], Limits::default()).unwrap();
    let mut calls = 0;
    let mut output = Cursor::new(Vec::new());
    let result = zip_edit::execute(&mut input, &mut output, &plan, || {
        calls += 1;
        calls == 4
    });
    assert!(matches!(result, Err(Error::Cancelled)));
    assert_eq!(input.get_ref(), &original);
    let mut changed = original.clone();
    let central = changed
        .windows(4)
        .position(|bytes| bytes == b"PK\x01\x02")
        .unwrap();
    changed[12] ^= 1;
    changed[central + 14] ^= 1;
    let mut output = Cursor::new(Vec::new());
    assert!(zip_edit::execute(&mut Cursor::new(changed), &mut output, &plan, || false).is_err());
    assert!(output.into_inner().is_empty());
}
#[cfg(feature = "crypto")]
#[test]
fn zip_edit_reuses_aes_and_zipcrypto_ciphertext_without_credentials() {
    use archive_core::{RandomSource, ZipEncryption};
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
    for mode in [ZipEncryption::Aes256, ZipEncryption::ZipCrypto] {
        let entry = CreateEntry {
            name: "secret".into(),
            data: b"private payload".repeat(100),
            kind: EntryKind::File,
        };
        let mut source = Cursor::new(Vec::new());
        create_with_options(
            Format::Zip,
            &[entry],
            &mut source,
            Limits::default(),
            CreateOptions {
                password: Some(b"correct"),
                randomness: Some(&mut TestRandom(0)),
                zip_encryption: mode,
                ..Default::default()
            },
        )
        .unwrap();
        let original = source.into_inner();
        let mut output = Cursor::new(Vec::new());
        zip_edit::edit(
            &mut Cursor::new(&original),
            &mut output,
            &[EditOperation::Rename {
                from: "secret".into(),
                to: "renamed".into(),
            }],
            Limits::default(),
        )
        .unwrap();
        let changed = output.into_inner();
        assert_eq!(
            raw_payload(&original, "secret"),
            raw_payload(&changed, "renamed")
        );
        let mut archive =
            Archive::open_with_password(Cursor::new(changed), Limits::default(), b"correct")
                .unwrap();
        assert_eq!(
            archive.read_entry(EntryId(0), 10_000).unwrap(),
            b"private payload".repeat(100)
        );
    }
}

#[test]
fn zip_edit_truncated_metadata_and_resource_limits_fail_before_output() {
    let mut source = zip::ZipWriter::new(Cursor::new(Vec::new()));
    source
        .start_file(
            "name",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .large_file(true),
        )
        .unwrap();
    source.write_all(b"payload").unwrap();
    let bytes = source.finish().unwrap().into_inner();
    for length in 0..bytes.len() {
        let mut output = Cursor::new(Vec::new());
        assert!(
            zip_edit::edit(
                &mut Cursor::new(&bytes[..length]),
                &mut output,
                &[],
                Limits::default()
            )
            .is_err()
        );
        assert!(output.into_inner().is_empty());
    }
    for limits in [
        Limits {
            max_input_bytes: bytes.len() as u64 - 1,
            ..Limits::default()
        },
        Limits {
            max_metadata_bytes: 1,
            ..Limits::default()
        },
        Limits {
            max_entries: 0,
            ..Limits::default()
        },
        Limits {
            max_entry_bytes: 1,
            ..Limits::default()
        },
        Limits {
            max_total_bytes: 1,
            ..Limits::default()
        },
    ] {
        let mut output = Cursor::new(Vec::new());
        assert!(matches!(
            zip_edit::edit(&mut Cursor::new(&bytes), &mut output, &[], limits),
            Err(Error::ResourceLimit(_))
        ));
        assert!(output.into_inner().is_empty());
    }
    let mut output = Cursor::new(Vec::new());
    zip_edit::edit(
        &mut Cursor::new(&bytes),
        &mut output,
        &[EditOperation::Rename {
            from: "name".into(),
            to: "renamed".into(),
        }],
        Limits::default(),
    )
    .unwrap();
    assert_eq!(
        raw_payload(&bytes, "name"),
        raw_payload(output.get_ref(), "renamed")
    );
}

fn inject_single_entry_extra(mut bytes: Vec<u8>, extra: &[u8]) -> Vec<u8> {
    fn short(bytes: &[u8], at: usize) -> usize {
        u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap()) as usize
    }
    let end = bytes.len() - 22;
    let mut central = u32::from_le_bytes(bytes[end + 16..end + 20].try_into().unwrap()) as usize;
    let directory_size = u32::from_le_bytes(bytes[end + 12..end + 16].try_into().unwrap());
    let local_extra = short(&bytes, 28);
    let insertion = 30 + short(&bytes, 26) + local_extra;
    bytes.splice(insertion..insertion, extra.iter().copied());
    bytes[28..30].copy_from_slice(&((local_extra + extra.len()) as u16).to_le_bytes());
    central += extra.len();
    let central_extra = short(&bytes, central + 30);
    let insertion = central + 46 + short(&bytes, central + 28) + central_extra;
    bytes.splice(insertion..insertion, extra.iter().copied());
    bytes[central + 30..central + 32]
        .copy_from_slice(&((central_extra + extra.len()) as u16).to_le_bytes());
    let end = bytes.len() - 22;
    bytes[end + 12..end + 16].copy_from_slice(&(directory_size + extra.len() as u32).to_le_bytes());
    bytes[end + 16..end + 20].copy_from_slice(&(central as u32).to_le_bytes());
    bytes
}
fn single_plain() -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    create_with_options(
        Format::Zip,
        &[CreateEntry {
            name: "name".into(),
            data: b"payload".repeat(100),
            kind: EntryKind::File,
        }],
        &mut output,
        Limits::default(),
        CreateOptions {
            zip_compression: ZipCompression::Copy,
            ..Default::default()
        },
    )
    .unwrap();
    output.into_inner()
}
#[test]
fn zip_timestamp_updates_ut_utc_dos_and_ntfs_preserving_other_times_and_packed_data() {
    let seconds = 1_706_933_107u64; // 2024-02-03 04:05:07 UTC.
    let mut ntfs = vec![10, 0, 32, 0, 0, 0, 0, 0, 1, 0, 24, 0];
    for value in [42u64, 123, 456] {
        ntfs.extend_from_slice(&value.to_le_bytes());
    }
    let original = inject_single_entry_extra(single_plain(), &ntfs);
    let mut input = Cursor::new(&original);
    let mut output = Cursor::new(Vec::new());
    let plan = zip_edit::plan(
        &mut input,
        &[
            EditOperation::Rename {
                from: "name".into(),
                to: "new".into(),
            },
            EditOperation::SetModified {
                name: "name".into(),
                modified_unix_seconds: seconds,
            },
        ],
        Limits::default(),
    )
    .unwrap();
    let report = zip_edit::execute(&mut input, &mut output, &plan, || false).unwrap();
    assert_eq!(report.metadata_changed_entries, 1);
    assert_eq!(report.verified_entries, 0);
    assert_eq!(
        raw_payload(&original, "name"),
        raw_payload(output.get_ref(), "new")
    );
    let mut archive = Archive::open(Cursor::new(output.get_ref()), Limits::default()).unwrap();
    assert_eq!(
        archive.entry_metadata(EntryId(0)).unwrap().modified,
        Some(archive_core::StoredTimestamp::UnixSeconds(seconds))
    );
    let mut independent = zip::ZipArchive::new(Cursor::new(output.get_ref())).unwrap();
    let entry = independent.by_index_raw(0).unwrap();
    let date = entry.last_modified().unwrap();
    assert_eq!(
        (
            date.year(),
            date.month(),
            date.day(),
            date.hour(),
            date.minute(),
            date.second()
        ),
        (2024, 2, 3, 4, 5, 6)
    );
    let extra = entry.extra_data().unwrap();
    assert_eq!(
        u64::from_le_bytes(extra[12..20].try_into().unwrap()),
        (seconds + 11_644_473_600) * 10_000_000
    );
    assert_eq!(u64::from_le_bytes(extra[20..28].try_into().unwrap()), 123);
    assert_eq!(u64::from_le_bytes(extra[28..36].try_into().unwrap()), 456);
}
#[test]
fn zip_timestamp_range_and_duplicate_properties_are_validated_before_output() {
    for operations in [
        vec![EditOperation::SetModified {
            name: "name".into(),
            modified_unix_seconds: u64::from(u32::MAX) + 1,
        }],
        vec![
            EditOperation::SetModified {
                name: "name".into(),
                modified_unix_seconds: 1,
            },
            EditOperation::SetModified {
                name: "name".into(),
                modified_unix_seconds: 2,
            },
        ],
        vec![
            EditOperation::Delete {
                name: "name".into(),
            },
            EditOperation::SetModified {
                name: "name".into(),
                modified_unix_seconds: 1,
            },
        ],
    ] {
        let mut output = Cursor::new(Vec::new());
        assert!(
            zip_edit::edit(
                &mut Cursor::new(single_plain()),
                &mut output,
                &operations,
                Limits::default()
            )
            .is_err()
        );
        assert!(output.into_inner().is_empty());
    }
    let mut output = Cursor::new(Vec::new());
    zip_edit::edit(
        &mut Cursor::new(single_plain()),
        &mut output,
        &[EditOperation::SetModified {
            name: "name".into(),
            modified_unix_seconds: 0,
        }],
        Limits::default(),
    )
    .unwrap();
    let mut archive = Archive::open(Cursor::new(output.get_ref()), Limits::default()).unwrap();
    assert_eq!(
        archive.entry_metadata(EntryId(0)).unwrap().modified,
        Some(archive_core::StoredTimestamp::UnixSeconds(0))
    );
}

#[cfg(feature = "crypto")]
struct EditRandom(u8);
#[cfg(feature = "crypto")]
impl archive_core::RandomSource for EditRandom {
    fn fill(&mut self, bytes: &mut [u8]) -> archive_core::Result<()> {
        for byte in bytes {
            self.0 = self.0.wrapping_add(1);
            *byte = self.0;
        }
        Ok(())
    }
}
#[cfg(feature = "crypto")]
fn encrypted_single(mode: archive_core::ZipEncryption, password: &[u8]) -> Vec<u8> {
    encrypted_single_using(mode, password, ZipCompression::Copy)
}
#[cfg(feature = "crypto")]
fn encrypted_single_using(
    mode: archive_core::ZipEncryption,
    password: &[u8],
    compression: ZipCompression,
) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    create_with_options(
        Format::Zip,
        &[CreateEntry {
            name: "name".into(),
            data: b"payload".repeat(100),
            kind: EntryKind::File,
        }],
        &mut output,
        Limits::default(),
        CreateOptions {
            password: Some(password),
            randomness: Some(&mut EditRandom(1)),
            zip_encryption: mode,
            zip_compression: compression,
            ..Default::default()
        },
    )
    .unwrap();
    output.into_inner()
}
#[cfg(feature = "crypto")]
#[test]
fn zipcrypto_descriptor_timestamp_change_requires_credentials_and_reencrypts_with_fresh_header() {
    use zip_edit::ZipEditOptions;
    let original = encrypted_single(archive_core::ZipEncryption::ZipCrypto, b"old");
    let operations = [EditOperation::SetModified {
        name: "name".into(),
        modified_unix_seconds: 1_706_933_107,
    }];
    let mut input = Cursor::new(&original);
    let plan = zip_edit::plan(&mut input, &operations, Limits::default()).unwrap();
    assert!(plan.entries()[0].requires_reencryption);
    let mut output = Cursor::new(Vec::new());
    assert!(matches!(
        zip_edit::execute(&mut input, &mut output, &plan, || false),
        Err(Error::PasswordRequired)
    ));
    assert!(output.get_ref().is_empty());
    let report = zip_edit::execute_with_options(
        &mut input,
        &mut output,
        &plan,
        ZipEditOptions {
            password: Some(b"old"),
            randomness: Some(&mut EditRandom(20)),
            ..Default::default()
        },
        || false,
    )
    .unwrap();
    assert_eq!(report.verified_entries, 1);
    assert!(report.payloads_verified);
    assert_ne!(
        raw_payload(&original, "name"),
        raw_payload(output.get_ref(), "name")
    );
    let mut decoded =
        Archive::open_with_password(Cursor::new(output.get_ref()), Limits::default(), b"old")
            .unwrap();
    assert_eq!(
        decoded.read_entry(EntryId(0), 1000).unwrap(),
        b"payload".repeat(100)
    );
    assert_eq!(
        decoded.entry_metadata(EntryId(0)).unwrap().modified,
        Some(archive_core::StoredTimestamp::UnixSeconds(1_706_933_107))
    );
}
#[cfg(feature = "crypto")]
#[test]
fn per_entry_aes_encryption_keeps_unselected_payloads_exact_and_composes_with_timestamp() {
    use zip_edit::{EntryEncryption, ZipEditOptions};
    let original = fixture();
    let mut input = Cursor::new(&original);
    let operations = [
        EditOperation::SetEncryption {
            name: "dir/a".into(),
            encryption: EntryEncryption::Aes256,
        },
        EditOperation::SetModified {
            name: "dir/a".into(),
            modified_unix_seconds: 1_706_933_107,
        },
    ];
    let plan = zip_edit::plan(&mut input, &operations, Limits::default()).unwrap();
    let mut output = Cursor::new(Vec::new());
    let report = zip_edit::execute_with_options(
        &mut input,
        &mut output,
        &plan,
        ZipEditOptions {
            new_password: Some(b"new"),
            randomness: Some(&mut EditRandom(3)),
            ..Default::default()
        },
        || false,
    )
    .unwrap();
    assert_eq!(report.verified_entries, 1);
    assert!(!report.payloads_verified);
    assert_eq!(
        raw_payload(&original, "directory/a"),
        raw_payload(output.get_ref(), "directory/a")
    );
    let mut archive =
        Archive::open_with_password(Cursor::new(output.get_ref()), Limits::default(), b"new")
            .unwrap();
    assert!(archive.entries()[1].encrypted);
    assert!(!archive.entries()[2].encrypted);
    assert_eq!(
        archive.read_entry(EntryId(1), 10000).unwrap(),
        b"dir/a".repeat(1024)
    );
    assert_eq!(
        archive.read_entry(EntryId(2), 20000).unwrap(),
        b"directory/a".repeat(1024)
    );
}
#[cfg(feature = "crypto")]
#[test]
fn zip_password_switch_and_removal_verify_old_credentials_before_any_output() {
    use zip_edit::{EntryEncryption, ZipEditOptions};
    for old_mode in [
        archive_core::ZipEncryption::Aes256,
        archive_core::ZipEncryption::ZipCrypto,
    ] {
        for target in [
            EntryEncryption::None,
            EntryEncryption::Aes256,
            EntryEncryption::ZipCrypto,
        ] {
            let original = encrypted_single(old_mode, b"old");
            let mut input = Cursor::new(&original);
            let plan = zip_edit::plan(
                &mut input,
                &[EditOperation::SetEncryption {
                    name: "name".into(),
                    encryption: target,
                }],
                Limits::default(),
            )
            .unwrap();
            let mut random = EditRandom(55);
            let mut output = Cursor::new(Vec::new());
            let options = ZipEditOptions {
                password: Some(b"wrong"),
                new_password: Some(b"new"),
                randomness: Some(&mut random),
            };
            assert!(zip_edit::validate_credentials(&mut input, &plan, &options, || false).is_err());
            assert!(
                zip_edit::execute_with_options(&mut input, &mut output, &plan, options, || false)
                    .is_err()
            );
            assert!(output.get_ref().is_empty());
            assert_eq!(input.get_ref(), &&original);
            let report = zip_edit::execute_with_options(
                &mut input,
                &mut output,
                &plan,
                ZipEditOptions {
                    password: Some(b"old"),
                    new_password: Some(b"new"),
                    randomness: Some(&mut EditRandom(55)),
                },
                || false,
            )
            .unwrap();
            assert!(report.payloads_verified);
            assert_eq!(report.reencrypted_entries, 1);
            let mut archive = if target == EntryEncryption::None {
                Archive::open(Cursor::new(output.get_ref()), Limits::default()).unwrap()
            } else {
                Archive::open_with_password(
                    Cursor::new(output.get_ref()),
                    Limits::default(),
                    b"new",
                )
                .unwrap()
            };
            assert_eq!(
                archive.read_entry(EntryId(0), 1000).unwrap(),
                b"payload".repeat(100)
            );
            assert_eq!(
                archive.entries()[0].encrypted,
                target != EntryEncryption::None
            );
        }
    }
}
#[cfg(feature = "crypto")]
#[test]
#[ignore = "requires pinned 7-Zip 26.04 executable in ARCHIVE_REFERENCE_7ZIP"]
fn pinned_7zip_decodes_changed_password_modes_and_timestamp_rewrites() {
    use zip_edit::{EntryEncryption, ZipEditOptions};
    let executable = std::env::var_os("ARCHIVE_REFERENCE_7ZIP")
        .expect("set ARCHIVE_REFERENCE_7ZIP to pinned 7-Zip 26.04 executable");
    let banner = std::process::Command::new(&executable)
        .arg("i")
        .output()
        .unwrap();
    assert!(banner.status.success());
    assert!(
        String::from_utf8_lossy(&banner.stdout).contains("26.04"),
        "reference executable must be 7-Zip 26.04"
    );
    let temp = tempfile::tempdir().unwrap();
    for (target, compression) in [
        (EntryEncryption::None, ZipCompression::Copy),
        (EntryEncryption::None, ZipCompression::Deflate),
        (EntryEncryption::Aes256, ZipCompression::Copy),
        (EntryEncryption::Aes256, ZipCompression::Deflate),
        (EntryEncryption::ZipCrypto, ZipCompression::Copy),
        (EntryEncryption::ZipCrypto, ZipCompression::Deflate),
    ] {
        let mut input = Cursor::new(encrypted_single_using(
            archive_core::ZipEncryption::ZipCrypto,
            b"old",
            compression,
        ));
        let plan = zip_edit::plan(
            &mut input,
            &[
                EditOperation::SetEncryption {
                    name: "name".into(),
                    encryption: target,
                },
                EditOperation::SetModified {
                    name: "name".into(),
                    modified_unix_seconds: 1_706_933_107,
                },
            ],
            Limits::default(),
        )
        .unwrap();
        let mut output = Cursor::new(Vec::new());
        zip_edit::execute_with_options(
            &mut input,
            &mut output,
            &plan,
            ZipEditOptions {
                password: Some(b"old"),
                new_password: Some(b"new"),
                randomness: Some(&mut EditRandom(15)),
            },
            || false,
        )
        .unwrap();
        let archive = temp.path().join("edited.zip");
        std::fs::write(&archive, output.into_inner()).unwrap();
        let decoded = std::process::Command::new(&executable)
            .args(["x", "-so", "-pnew"])
            .arg(&archive)
            .arg("name")
            .output()
            .unwrap();
        assert!(
            decoded.status.success(),
            "{}",
            String::from_utf8_lossy(&decoded.stderr)
        );
        assert_eq!(decoded.stdout, b"payload".repeat(100));
        let listed = std::process::Command::new(&executable)
            .env("TZ", "UTC")
            .args(["l", "-slt"])
            .arg(&archive)
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&listed.stdout).contains("Modified = 2024-02-03 04:05:07"));
    }
}

#[cfg(feature = "crypto")]
#[test]
fn zip_credential_verification_cancellation_and_corruption_fail_before_output() {
    use zip_edit::{EntryEncryption, ZipEditOptions};
    let original = encrypted_single(archive_core::ZipEncryption::Aes256, b"old");
    let mut input = Cursor::new(&original);
    let plan = zip_edit::plan(
        &mut input,
        &[EditOperation::SetEncryption {
            name: "name".into(),
            encryption: EntryEncryption::None,
        }],
        Limits::default(),
    )
    .unwrap();
    let mut calls = 0;
    assert!(matches!(
        zip_edit::validate_credentials(
            &mut input,
            &plan,
            &ZipEditOptions {
                password: Some(b"old"),
                ..Default::default()
            },
            || {
                calls += 1;
                calls >= 3
            }
        ),
        Err(Error::Cancelled)
    ));
    let mut corrupt = original.clone();
    let central = corrupt
        .windows(4)
        .position(|bytes| bytes == b"PK\x01\x02")
        .unwrap();
    corrupt[central - 1] ^= 1;
    let mut output = Cursor::new(Vec::new());
    assert!(
        zip_edit::execute_with_options(
            &mut Cursor::new(corrupt),
            &mut output,
            &plan,
            ZipEditOptions {
                password: Some(b"old"),
                ..Default::default()
            },
            || false
        )
        .is_err()
    );
    assert!(output.get_ref().is_empty());
    assert_eq!(input.get_ref(), &&original);
}

#[cfg(feature = "crypto")]
#[test]
fn selected_password_change_keeps_other_ciphertext_and_old_password_usable() {
    use zip_edit::{EntryEncryption, ZipEditOptions};
    let mut input = Cursor::new(fixture());
    let mut encrypted = Cursor::new(Vec::new());
    let plan = zip_edit::plan(
        &mut input,
        &[
            EditOperation::SetEncryption {
                name: "dir/a".into(),
                encryption: EntryEncryption::Aes256,
            },
            EditOperation::SetEncryption {
                name: "directory/a".into(),
                encryption: EntryEncryption::Aes256,
            },
        ],
        Limits::default(),
    )
    .unwrap();
    zip_edit::execute_with_options(
        &mut input,
        &mut encrypted,
        &plan,
        ZipEditOptions {
            new_password: Some(b"old"),
            randomness: Some(&mut EditRandom(1)),
            ..Default::default()
        },
        || false,
    )
    .unwrap();
    let original = encrypted.into_inner();
    let mut input = Cursor::new(&original);
    let mut changed = Cursor::new(Vec::new());
    let plan = zip_edit::plan(
        &mut input,
        &[EditOperation::SetEncryption {
            name: "dir/a".into(),
            encryption: EntryEncryption::Aes256,
        }],
        Limits::default(),
    )
    .unwrap();
    let report = zip_edit::execute_with_options(
        &mut input,
        &mut changed,
        &plan,
        ZipEditOptions {
            password: Some(b"old"),
            new_password: Some(b"new"),
            randomness: Some(&mut EditRandom(55)),
        },
        || false,
    )
    .unwrap();
    assert_eq!(report.verified_entries, 1);
    assert!(!report.payloads_verified);
    assert_eq!(
        raw_payload(&original, "directory/a"),
        raw_payload(changed.get_ref(), "directory/a")
    );
    let mut new =
        Archive::open_with_password(Cursor::new(changed.get_ref()), Limits::default(), b"new")
            .unwrap();
    assert_eq!(
        new.read_entry(EntryId(1), 10000).unwrap(),
        b"dir/a".repeat(1024)
    );
    let mut old =
        Archive::open_with_password(Cursor::new(changed.get_ref()), Limits::default(), b"old")
            .unwrap();
    assert_eq!(
        old.read_entry(EntryId(2), 20000).unwrap(),
        b"directory/a".repeat(1024)
    );
}
#[cfg(feature = "crypto")]
#[test]
fn aes_timestamp_and_unchanged_zipcrypto_checkbyte_reuse_ciphertext_without_credentials() {
    for (mode, seconds) in [
        (archive_core::ZipEncryption::Aes256, 1_706_933_107),
        (archive_core::ZipEncryption::ZipCrypto, 315_619_200),
    ] {
        let original = encrypted_single(mode, b"old");
        let mut input = Cursor::new(&original);
        let mut output = Cursor::new(Vec::new());
        let plan = zip_edit::plan(
            &mut input,
            &[EditOperation::SetModified {
                name: "name".into(),
                modified_unix_seconds: seconds,
            }],
            Limits::default(),
        )
        .unwrap();
        assert!(!plan.entries()[0].requires_reencryption);
        let report = zip_edit::execute(&mut input, &mut output, &plan, || false).unwrap();
        assert_eq!(report.verified_entries, 0);
        assert_eq!(
            raw_payload(&original, "name"),
            raw_payload(output.get_ref(), "name")
        );
        let mut archive =
            Archive::open_with_password(Cursor::new(output.get_ref()), Limits::default(), b"old")
                .unwrap();
        assert_eq!(
            archive.read_entry(EntryId(0), 1000).unwrap(),
            b"payload".repeat(100)
        );
    }
}
