#![cfg(all(
    feature = "tar",
    feature = "zip",
    feature = "sevenz",
    feature = "crypto"
))]
use archive_core::{
    Archive, CreateEntry, CreateOptions, CreateSource, EntryId, EntryKind, Error, Format, Limits,
    RandomSource, SevenZipCompression, ZipCompression, ZipEncryption, create, create_from_readers,
    create_with_options,
};
use std::{
    cell::Cell,
    io::{Cursor, Read, Seek, SeekFrom, Write},
    rc::Rc,
};

struct Random;
impl RandomSource for Random {
    fn fill(&mut self, bytes: &mut [u8]) -> archive_core::Result<()> {
        bytes.fill(42);
        Ok(())
    }
}
#[test]
fn all_compressed_creation_dispatches_enforce_zero_budgets_before_output() {
    let input = [CreateEntry {
        name: "payload".into(),
        kind: EntryKind::File,
        data: b"hello".to_vec(),
    }];
    for limits in [
        Limits {
            max_dictionary_bytes: 0,
            ..Limits::default()
        },
        Limits {
            max_active_workspace_bytes: 0,
            ..Limits::default()
        },
    ] {
        let mut output = Cursor::new(Vec::new());
        assert!(matches!(
            create(Format::SevenZip, &input, &mut output, limits),
            Err(Error::ResourceLimit(_))
        ));
        assert!(output.get_ref().is_empty());
        for method in [
            SevenZipCompression::Deflate,
            SevenZipCompression::Lzma,
            SevenZipCompression::Lzma2,
            SevenZipCompression::Bzip2,
            SevenZipCompression::Brotli,
        ] {
            assert!(matches!(
                create_with_options(
                    Format::SevenZip,
                    &input,
                    &mut output,
                    limits,
                    CreateOptions {
                        sevenz_compression: method,
                        ..Default::default()
                    }
                ),
                Err(Error::ResourceLimit(_))
            ));
        }
        let mut random = Random;
        assert!(matches!(
            create_with_options(
                Format::Zip,
                &input,
                &mut output,
                limits,
                CreateOptions {
                    password: Some(b"fixture"),
                    randomness: Some(&mut random),
                    ..Default::default()
                }
            ),
            Err(Error::ResourceLimit(_))
        ));
        assert!(output.get_ref().is_empty());
    }
}
#[test]
fn sevenz_deflate_uses_dictionary_budget_independently_and_streams_large_output() {
    let data = vec![b'a'; 4 << 20];
    let input = [CreateEntry {
        name: "large".into(),
        kind: EntryKind::File,
        data: data.clone(),
    }];
    let mut encoded = Cursor::new(Vec::new());
    create_with_options(
        Format::SevenZip,
        &input,
        &mut encoded,
        Limits::default(),
        CreateOptions {
            sevenz_compression: SevenZipCompression::Deflate,
            ..Default::default()
        },
    )
    .unwrap();
    let limits = Limits {
        max_dictionary_bytes: 32768,
        max_active_workspace_bytes: 2 << 20,
        ..Limits::default()
    };
    let mut archive = Archive::open(encoded, limits).unwrap();
    let mut decoded = Vec::new();
    archive.extract(EntryId(0), &mut decoded).unwrap();
    assert_eq!(decoded, data);
}
struct Generated {
    remaining: usize,
    active: Rc<Cell<usize>>,
}
impl Read for Generated {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        assert!(output.len() <= 65536);
        let count = output.len().min(self.remaining).min(7919);
        output[..count].fill(b'z');
        self.remaining -= count;
        Ok(count)
    }
}
impl Drop for Generated {
    fn drop(&mut self) {
        self.active.set(self.active.get() - 1);
    }
}
#[test]
fn reader_creation_opens_one_source_at_a_time_and_roundtrips_encrypted_zip() {
    for (format, password, mode) in [
        (Format::Tar, None, ZipEncryption::Aes256),
        (Format::Zip, None, ZipEncryption::Aes256),
        (
            Format::Zip,
            Some(b"fixture".as_slice()),
            ZipEncryption::Aes256,
        ),
        (
            Format::Zip,
            Some(b"fixture".as_slice()),
            ZipEncryption::ZipCrypto,
        ),
    ] {
        for compression in [ZipCompression::Copy, ZipCompression::Deflate] {
            let entries: Vec<_> = (0..3)
                .map(|i| CreateSource {
                    name: format!("{i}.bin"),
                    kind: EntryKind::File,
                    size: 300_000,
                })
                .collect();
            let active = Rc::new(Cell::new(0));
            let mut open = |_| -> archive_core::Result<Box<dyn Read>> {
                assert_eq!(active.get(), 0);
                active.set(1);
                Ok(Box::new(Generated {
                    remaining: 300_000,
                    active: active.clone(),
                }))
            };
            let mut random = Random;
            let mut encoded = Cursor::new(Vec::new());
            create_from_readers(
                format,
                &entries,
                &mut open,
                &mut encoded,
                Limits::default(),
                CreateOptions {
                    password,
                    randomness: Some(&mut random),
                    zip_encryption: mode,
                    zip_compression: compression,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(active.get(), 0);
            let mut archive = if let Some(password) = password {
                Archive::open_with_password(encoded, Limits::default(), password).unwrap()
            } else {
                Archive::open(encoded, Limits::default()).unwrap()
            };
            for i in 0..3 {
                let mut output = Vec::new();
                archive.extract(EntryId(i), &mut output).unwrap();
                assert_eq!(output, vec![b'z'; 300_000]);
            }
        }
    }
}
#[test]
fn advertised_sizes_are_enforced_for_short_and_growing_sources() {
    for format in [Format::Tar, Format::Zip, Format::SevenZip] {
        for data in [vec![1; 9], vec![1; 11]] {
            let entries = [CreateSource {
                name: "file".into(),
                kind: EntryKind::File,
                size: 10,
            }];
            let mut open = |_| -> archive_core::Result<Box<dyn Read>> {
                Ok(Box::new(Cursor::new(data.clone())))
            };
            assert!(
                create_from_readers(
                    format,
                    &entries,
                    &mut open,
                    &mut Cursor::new(Vec::new()),
                    Limits::default(),
                    CreateOptions::default()
                )
                .is_err()
            );
        }
    }
}

#[test]
fn encrypted_creation_rejects_kdf_work_and_unsupported_headers_before_opening_sources() {
    let entries = [CreateSource {
        name: "file".into(),
        kind: EntryKind::File,
        size: 0,
    }];
    let mut never_open = |_| -> archive_core::Result<Box<dyn Read>> {
        panic!("invalid options must fail before opening input")
    };
    let mut random = Random;
    let mut output = Cursor::new(Vec::new());
    assert!(matches!(
        create_from_readers(
            Format::Zip,
            &entries,
            &mut never_open,
            &mut output,
            Limits {
                max_password_iterations: 999,
                ..Limits::default()
            },
            CreateOptions {
                password: Some(b"fixture"),
                randomness: Some(&mut random),
                ..Default::default()
            }
        ),
        Err(Error::ResourceLimit(_))
    ));
    assert!(matches!(
        create_from_readers(
            Format::Tar,
            &entries,
            &mut never_open,
            &mut output,
            Limits::default(),
            CreateOptions {
                encrypt_headers: true,
                ..Default::default()
            }
        ),
        Err(Error::Unsupported(_))
    ));
    assert!(output.get_ref().is_empty());
}

struct DiscardOutput {
    position: u64,
    written: Rc<Cell<u64>>,
    fail_at: Option<u64>,
}
impl Write for DiscardOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self
            .fail_at
            .is_some_and(|end| self.position + bytes.len() as u64 > end)
        {
            return Err(std::io::Error::other("injected output failure"));
        }
        self.position += bytes.len() as u64;
        self.written.set(self.written.get().max(self.position));
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Seek for DiscardOutput {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        self.position = match from {
            SeekFrom::Start(offset) => offset,
            SeekFrom::Current(offset) => self.position.checked_add_signed(offset).unwrap(),
            SeekFrom::End(offset) => self.written.get().checked_add_signed(offset).unwrap(),
        };
        Ok(self.position)
    }
}
struct StreamingProbe {
    position: u64,
    size: u64,
    written: Rc<Cell<u64>>,
}
impl Read for StreamingProbe {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        // Copy output must reach the sink as reads progress, before input EOF.
        assert!(self.written.get() >= self.position);
        let n = bytes.len().min((self.size - self.position) as usize);
        bytes[..n].fill(42);
        self.position += n as u64;
        Ok(n)
    }
}
#[test]
fn sevenz_copy_streams_payload_larger_than_workspace_directly_to_output() {
    let size = 16 << 20;
    let written = Rc::new(Cell::new(0));
    let entries = [CreateSource {
        name: "large".into(),
        kind: EntryKind::File,
        size,
    }];
    let mut open = |_| -> archive_core::Result<Box<dyn Read>> {
        Ok(Box::new(StreamingProbe {
            position: 0,
            size,
            written: written.clone(),
        }))
    };
    let mut output = DiscardOutput {
        position: 0,
        written: written.clone(),
        fail_at: None,
    };
    create_from_readers(
        Format::SevenZip,
        &entries,
        &mut open,
        &mut output,
        Limits {
            max_active_workspace_bytes: 1 << 20,
            ..Limits::default()
        },
        CreateOptions {
            sevenz_compression: SevenZipCompression::Copy,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(written.get() > size);
}
#[test]
fn sevenz_reader_codecs_and_aes_padding_roundtrip() {
    for method in [
        SevenZipCompression::Copy,
        SevenZipCompression::Deflate,
        SevenZipCompression::Lzma,
        SevenZipCompression::Lzma2,
        SevenZipCompression::Bzip2,
        SevenZipCompression::Brotli,
    ] {
        if method == SevenZipCompression::Bzip2 && !cfg!(feature = "bzip2")
            || method == SevenZipCompression::Brotli && !cfg!(feature = "brotli")
        {
            continue;
        }
        for encrypted in [false, true] {
            let data: Vec<_> = (0..65537).map(|i| (i % 251) as u8).collect();
            let entries: Vec<_> = [0, 1, 15, 16, 17, 65537]
                .into_iter()
                .enumerate()
                .map(|(i, size)| CreateSource {
                    name: format!("{i}.bin"),
                    kind: EntryKind::File,
                    size,
                })
                .collect();
            let mut open = |i: usize| -> archive_core::Result<Box<dyn Read>> {
                Ok(Box::new(Cursor::new(
                    data[..entries[i].size as usize].to_vec(),
                )))
            };
            let mut random = Random;
            let mut output = Cursor::new(Vec::new());
            create_from_readers(
                Format::SevenZip,
                &entries,
                &mut open,
                &mut output,
                Limits::default(),
                CreateOptions {
                    sevenz_compression: method,
                    password: encrypted.then_some(b"fixture"),
                    encrypt_headers: encrypted,
                    randomness: Some(&mut random),
                    ..Default::default()
                },
            )
            .unwrap();
            let mut archive = if encrypted {
                Archive::open_with_password(output, Limits::default(), b"fixture").unwrap()
            } else {
                Archive::open(output, Limits::default()).unwrap()
            };
            for (i, entry) in entries.iter().enumerate() {
                let mut actual = Vec::new();
                archive
                    .extract(EntryId(i), &mut actual)
                    .unwrap_or_else(|error| {
                        panic!(
                            "{method:?} encrypted={encrypted} size={} {error}",
                            entry.size
                        )
                    });
                assert_eq!(
                    actual,
                    data[..entry.size as usize],
                    "{method:?}, encrypted={encrypted}"
                );
            }
        }
    }
}
#[test]
fn sevenz_encrypted_final_padding_failure_is_returned() {
    let entries = [CreateSource {
        name: "file".into(),
        kind: EntryKind::File,
        size: 17,
    }];
    let mut open =
        |_| -> archive_core::Result<Box<dyn Read>> { Ok(Box::new(Cursor::new(vec![1; 17]))) };
    // First block fits; final zero-padded block must surface the sink's error.
    let mut output = DiscardOutput {
        position: 0,
        written: Rc::new(Cell::new(0)),
        fail_at: Some(48),
    };
    let mut random = Random;
    assert!(matches!(
        create_from_readers(
            Format::SevenZip,
            &entries,
            &mut open,
            &mut output,
            Limits::default(),
            CreateOptions {
                sevenz_compression: SevenZipCompression::Copy,
                password: Some(b"fixture"),
                randomness: Some(&mut random),
                ..Default::default()
            }
        ),
        Err(Error::Io(_))
    ));
    assert_eq!(output.position, 48);
}

#[test]
fn sevenz_staging_workspace_is_checked_before_opening_or_writing() {
    let entries = [CreateSource {
        name: "file".into(),
        kind: EntryKind::File,
        size: 1,
    }];
    for encrypted in [false, true] {
        let mut never_open = |_| -> archive_core::Result<Box<dyn Read>> {
            panic!("workspace must be checked before opening input")
        };
        let mut output = Cursor::new(Vec::new());
        let mut random = Random;
        let result = create_from_readers(
            Format::SevenZip,
            &entries,
            &mut never_open,
            &mut output,
            Limits {
                max_active_workspace_bytes: if encrypted { 131071 } else { 65535 },
                ..Limits::default()
            },
            CreateOptions {
                sevenz_compression: SevenZipCompression::Copy,
                password: encrypted.then_some(b"fixture"),
                randomness: Some(&mut random),
                ..Default::default()
            },
        );
        assert!(matches!(result, Err(Error::ResourceLimit(_))));
        assert!(output.get_ref().is_empty());
    }
}
