#![cfg(all(feature = "gzip", feature = "xz", feature = "bzip2"))]
use archive_core::{Archive, CreateEntry, EntryKind, Error, Format, Limits};
use std::{
    cell::Cell,
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
    rc::Rc,
};

struct Counted {
    inner: Cursor<Vec<u8>>,
    read: Rc<Cell<u64>>,
}
impl Read for Counted {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(bytes)?;
        self.read.set(self.read.get() + n as u64);
        Ok(n)
    }
}
impl Write for Counted {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for Counted {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.seek(position)
    }
}
fn payload() -> Vec<u8> {
    let mut state = 13u32;
    (0..128 * 1024)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect()
}
fn fixture(format: Format) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    archive_core::create(
        format,
        &[CreateEntry {
            name: "payload".into(),
            data: payload(),
            kind: EntryKind::File,
        }],
        &mut output,
        Limits::default(),
    )
    .unwrap();
    output.into_inner()
}

#[test]
fn detection_and_retention_share_one_decode_for_buffered_and_scratch_opening() {
    for format in [
        Format::TarGzip,
        Format::TarXz,
        Format::TarBzip2,
        Format::Gzip,
        Format::Xz,
        Format::Bzip2,
    ] {
        let bytes = fixture(format);
        for scratch in [false, true] {
            let reads = Rc::new(Cell::new(0));
            let input = Counted {
                inner: Cursor::new(bytes.clone()),
                read: reads.clone(),
            };
            let mut archive = if scratch {
                Archive::open_with_scratch(
                    input,
                    Counted {
                        inner: Cursor::new(Vec::new()),
                        read: Rc::default(),
                    },
                    Limits::default(),
                    None,
                    None,
                )
                .unwrap()
            } else {
                Archive::open(input, Limits::default()).unwrap()
            };
            assert_eq!(archive.format(), format);
            assert!(
                reads.get() <= bytes.len() as u64 + 4096,
                "{format:?} scratch={scratch}: read {} of {}",
                reads.get(),
                bytes.len()
            );
            assert_eq!(
                archive
                    .read_entry(archive.entries()[0].id, 1 << 20)
                    .unwrap(),
                payload()
            );
        }
    }
}

#[test]
fn buffered_tar_verification_checks_cancellation_between_chunks() {
    let mut archive =
        Archive::open(Cursor::new(fixture(Format::TarGzip)), Limits::default()).unwrap();
    let checks = Cell::new(0);
    let result = archive.test_cancellable(|| {
        checks.set(checks.get() + 1);
        checks.get() >= 3
    });
    assert!(matches!(result, Err(Error::Cancelled)));
    assert_eq!(checks.get(), 3);
}

#[test]
fn late_compressed_corruption_is_rejected_after_tar_detection() {
    let mut bytes = fixture(Format::TarGzip);
    let trailer = bytes.len() - 8;
    bytes[trailer] ^= 1;
    assert!(Archive::open(Cursor::new(bytes.clone()), Limits::default()).is_err());
    assert!(
        Archive::open_with_scratch(
            Cursor::new(bytes),
            Cursor::new(Vec::new()),
            Limits::default(),
            None,
            None
        )
        .is_err()
    );
}
