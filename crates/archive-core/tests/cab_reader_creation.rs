#![cfg(feature = "cab")]

use archive_core::{
    Archive, CabCompression, CreateOptions, CreateSource, EntryId, EntryKind, EntryMetadata, Error,
    Format, Limits, StoredTimestamp, create_from_readers,
};
use std::{
    cell::Cell,
    io::{self, Cursor, Read},
    rc::Rc,
};

const METHODS: [CabCompression; 4] = [
    CabCompression::Copy,
    CabCompression::MsZip,
    CabCompression::Lzx,
    CabCompression::Quantum,
];

struct Generated {
    position: usize,
    size: usize,
    seed: usize,
    active: Rc<Cell<usize>>,
}
impl Read for Generated {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        assert!(output.len() <= 65536, "payload reads must remain bounded");
        let count = output.len().min(self.size - self.position).min(113);
        for (offset, byte) in output[..count].iter_mut().enumerate() {
            *byte = ((self.position + offset + self.seed) % 251) as u8;
        }
        self.position += count;
        Ok(count)
    }
}
impl Drop for Generated {
    fn drop(&mut self) {
        self.active.set(self.active.get() - 1);
    }
}

#[test]
fn all_cab_codecs_open_one_reader_at_a_time_across_frame_and_member_boundaries() {
    let sources: Vec<_> = [0, 1, 32767, 32768, 32769, 65537]
        .into_iter()
        .enumerate()
        .map(|(index, size)| CreateSource {
            name: format!("directory/{index}.bin"),
            kind: EntryKind::File,
            size,
        })
        .collect();
    for method in METHODS {
        let active = Rc::new(Cell::new(0));
        let opened = Cell::new(0);
        let mut open = |index: usize| -> archive_core::Result<Box<dyn Read>> {
            assert_eq!(active.get(), 0, "{method:?}: concurrent source readers");
            assert_eq!(index, opened.get(), "source order must follow descriptors");
            opened.set(index + 1);
            active.set(1);
            Ok(Box::new(Generated {
                position: 0,
                size: sources[index].size as usize,
                seed: index,
                active: Rc::clone(&active),
            }))
        };
        let mut output = Cursor::new(Vec::new());
        create_from_readers(
            Format::Cab,
            &sources,
            &mut open,
            &mut output,
            Limits::default(),
            CreateOptions {
                cab_compression: method,
                ..Default::default()
            },
        )
        .unwrap_or_else(|error| panic!("{method:?}: {error}"));
        assert_eq!(active.get(), 0);
        assert_eq!(opened.get(), sources.len());
        let mut archive = Archive::open(output, Limits::default()).unwrap();
        assert_eq!(archive.entries().len(), sources.len());
        for (index, source) in sources.iter().enumerate() {
            let mut actual = Vec::new();
            let report = archive.extract(EntryId(index), &mut actual).unwrap();
            assert!(report.verified);
            assert_eq!(actual.len() as u64, source.size);
            assert!(
                actual
                    .iter()
                    .enumerate()
                    .all(|(offset, &byte)| byte == ((offset + index) % 251) as u8),
                "{method:?}: member {index} differs"
            );
        }
    }
}

#[test]
fn cab_reader_sizes_reject_truncation_growth_and_nonempty_zero_length_sources() {
    for method in METHODS {
        for (advertised, actual) in [(0, 1), (1, 0), (32768, 32767), (32768, 32769)] {
            let sources = [CreateSource {
                name: "file".into(),
                kind: EntryKind::File,
                size: advertised,
            }];
            let mut open = |_| -> archive_core::Result<Box<dyn Read>> {
                Ok(Box::new(Cursor::new(vec![42; actual])))
            };
            let result = create_from_readers(
                Format::Cab,
                &sources,
                &mut open,
                &mut Cursor::new(Vec::new()),
                Limits::default(),
                CreateOptions {
                    cab_compression: method,
                    ..Default::default()
                },
            );
            assert!(
                result.is_err(),
                "{method:?}: accepted {actual} bytes as {advertised}"
            );
        }
    }
}

struct FailingReader {
    remaining: usize,
}
impl Read for FailingReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::Error::other("injected CAB source failure"));
        }
        let count = output.len().min(self.remaining);
        output[..count].fill(3);
        self.remaining -= count;
        Ok(count)
    }
}

#[test]
fn cab_input_failure_stops_before_opening_the_next_member() {
    for method in METHODS {
        let sources: Vec<_> = ["first", "second"]
            .into_iter()
            .map(|name| CreateSource {
                name: name.into(),
                kind: EntryKind::File,
                size: 65536,
            })
            .collect();
        let mut open = |index| -> archive_core::Result<Box<dyn Read>> {
            assert_eq!(index, 0, "later source opened after an input failure");
            Ok(Box::new(FailingReader { remaining: 32769 }))
        };
        let result = create_from_readers(
            Format::Cab,
            &sources,
            &mut open,
            &mut Cursor::new(Vec::new()),
            Limits::default(),
            CreateOptions {
                cab_compression: method,
                ..Default::default()
            },
        );
        assert!(
            matches!(result, Err(Error::Io(error)) if error.to_string().contains("injected CAB source failure")),
            "{method:?}: source error must propagate"
        );
    }
}

#[test]
fn cab_reader_creation_preserves_dos_time_and_readonly_attribute() {
    let sources = [CreateSource {
        name: "readonly.txt".into(),
        kind: EntryKind::File,
        size: 7,
    }];
    let metadata = [EntryMetadata {
        modified: Some(StoredTimestamp::DosLocal {
            year: 2024,
            month: 2,
            day: 29,
            hour: 13,
            minute: 17,
            second: 26,
        }),
        unix_mode: Some(0o444),
        ..Default::default()
    }];
    for method in METHODS {
        let mut open =
            |_| -> archive_core::Result<Box<dyn Read>> { Ok(Box::new(Cursor::new(b"payload"))) };
        let mut output = Cursor::new(Vec::new());
        create_from_readers(
            Format::Cab,
            &sources,
            &mut open,
            &mut output,
            Limits::default(),
            CreateOptions {
                cab_compression: method,
                entry_metadata: Some(&metadata),
                ..Default::default()
            },
        )
        .unwrap();
        let mut archive = Archive::open(output, Limits::default()).unwrap();
        let actual = archive.entry_metadata(EntryId(0)).unwrap();
        assert_eq!(actual.modified, metadata[0].modified, "{method:?}");
        assert_eq!(actual.unix_mode.unwrap() & 0o222, 0, "{method:?}");
        assert_eq!(archive.read_entry(EntryId(0), 7).unwrap(), b"payload");
    }
}

#[test]
fn cab_reader_creation_rejects_declared_limits_before_opening_input() {
    let sources = [CreateSource {
        name: "file".into(),
        kind: EntryKind::File,
        size: 100,
    }];
    let mut never_open = |_| -> archive_core::Result<Box<dyn Read>> {
        panic!("invalid creation limits must fail before source reads")
    };
    let mut output = Cursor::new(Vec::new());
    assert!(matches!(
        create_from_readers(
            Format::Cab,
            &sources,
            &mut never_open,
            &mut output,
            Limits {
                max_entry_bytes: 99,
                ..Limits::default()
            },
            CreateOptions::default(),
        ),
        Err(Error::ResourceLimit(_))
    ));
    assert!(output.get_ref().is_empty());
}

struct DiscardOutput {
    position: u64,
    length: u64,
    written: Rc<Cell<u64>>,
}
impl io::Write for DiscardOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.position += bytes.len() as u64;
        self.length = self.length.max(self.position);
        self.written.set(self.written.get() + bytes.len() as u64);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl io::Seek for DiscardOutput {
    fn seek(&mut self, from: io::SeekFrom) -> io::Result<u64> {
        self.position = match from {
            io::SeekFrom::Start(offset) => offset,
            io::SeekFrom::Current(offset) => self.position.checked_add_signed(offset).unwrap(),
            io::SeekFrom::End(offset) => self.length.checked_add_signed(offset).unwrap(),
        };
        Ok(self.position)
    }
}
struct StreamingProbe {
    position: usize,
    size: usize,
    initial_written: u64,
    written: Rc<Cell<u64>>,
}
impl Read for StreamingProbe {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.position >= 65536 {
            assert!(
                self.written.get() > self.initial_written,
                "CAB payload must reach the destination before source EOF"
            );
        }
        assert!(output.len() <= 65536);
        let count = output.len().min(self.size - self.position);
        output[..count].fill(17);
        self.position += count;
        Ok(count)
    }
}

#[test]
fn cab_payload_reaches_output_before_the_entire_source_is_read() {
    for method in METHODS {
        let size = 256 * 1024;
        let sources = [CreateSource {
            name: "large.bin".into(),
            kind: EntryKind::File,
            size: size as u64,
        }];
        let written = Rc::new(Cell::new(0));
        let mut open = |_| -> archive_core::Result<Box<dyn Read>> {
            Ok(Box::new(StreamingProbe {
                position: 0,
                size,
                initial_written: written.get(),
                written: Rc::clone(&written),
            }))
        };
        let mut output = DiscardOutput {
            position: 0,
            length: 0,
            written: Rc::clone(&written),
        };
        create_from_readers(
            Format::Cab,
            &sources,
            &mut open,
            &mut output,
            Limits::default(),
            CreateOptions {
                cab_compression: method,
                ..Default::default()
            },
        )
        .unwrap_or_else(|error| panic!("{method:?}: {error}"));
    }
}
