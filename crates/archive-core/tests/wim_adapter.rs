#![cfg(feature = "wim")]
use archive_core::{Limits, wim::WimArchive};

#[test]
fn selected_image_payload_matches_existing_independently_generated_fixture() {
    let bytes = include_bytes!("fixtures/xpress-resource.wim");
    let archive = WimArchive::open(bytes, 1, Limits::default()).unwrap();
    assert_eq!(archive.image_count(), 1);
    let file = archive
        .entries()
        .iter()
        .find(|entry| entry.size > 0)
        .unwrap();
    let mut expected = (0..300).flat_map(|_| 0u8..=255).collect::<Vec<_>>();
    expected.extend_from_slice(b"last chunk");
    assert_eq!(
        archive.read_entry(file.id, expected.len() as u64).unwrap(),
        expected
    );
}

#[test]
fn image_selector_and_metadata_budget_are_enforced() {
    let bytes = include_bytes!("fixtures/xpress-resource.wim");
    assert!(WimArchive::open(bytes, 0, Limits::default()).is_err());
    assert!(WimArchive::open(bytes, 2, Limits::default()).is_err());
    let limits = Limits {
        max_metadata_bytes: 1,
        ..Limits::default()
    };
    assert!(WimArchive::open(bytes, 1, limits).is_err());
    let limits = Limits {
        max_active_workspace_bytes: 1,
        ..Limits::default()
    };
    assert!(WimArchive::open(bytes, 1, limits).is_err());
}

#[test]
fn independently_generated_lzms_solid_esd_verifies_every_payload() {
    let bytes = include_bytes!("fixtures/wimlib-lzms-solid.esd");
    let archive = WimArchive::open(bytes, 1, Limits::default()).unwrap();
    let report = archive.test().unwrap();
    assert_eq!((report.entries, report.bytes), (3, 39365));
    assert!(report.verified);
    assert_eq!(
        archive
            .entries()
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["iso9660.rs", "lib.rs", "writer.rs"]
    );
}

#[test]
fn xml_image_listing_and_exact_name_selection_agree() {
    let bytes = include_bytes!("fixtures/wimlib-lzms-solid.esd");
    let images = archive_core::wim::images(bytes, Limits::default()).unwrap();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].index, 1);
    let name = images[0].name.as_deref().unwrap();
    assert!(!name.is_empty());
    let archive = WimArchive::open_by_name(bytes, name, Limits::default()).unwrap();
    assert_eq!(archive.image(), images[0].index);
    assert!(archive.test().unwrap().verified);
    assert!(WimArchive::open_by_name(bytes, "missing image", Limits::default()).is_err());
    assert!(
        archive_core::wim::images(
            bytes,
            Limits {
                max_entries: 0,
                ..Limits::default()
            }
        )
        .is_err()
    );
    assert!(
        archive_core::wim::images(
            bytes,
            Limits {
                max_metadata_bytes: 1,
                ..Limits::default()
            }
        )
        .is_err()
    );
}

#[test]
fn seekable_wim_does_not_read_whole_input_or_emit_whole_file() {
    use std::{
        cell::Cell,
        io::{self, Cursor, Read, Seek, SeekFrom, Write},
        rc::Rc,
    };
    struct Counted {
        source: Cursor<Vec<u8>>,
        count: Rc<Cell<usize>>,
    }
    impl Read for Counted {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            let n = self.source.read(bytes)?;
            self.count.set(self.count.get() + n);
            Ok(n)
        }
    }
    impl Seek for Counted {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            self.source.seek(position)
        }
    }
    struct Sink(usize);
    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            assert!(bytes.len() <= 65536);
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut bytes = include_bytes!("fixtures/xpress-resource.wim").to_vec();
    bytes.resize(4 << 20, 0);
    let count = Rc::new(Cell::new(0));
    let reader = Counted {
        source: Cursor::new(bytes),
        count: count.clone(),
    };
    let archive =
        archive_core::wim::FileWimArchive::open_reader(reader, 1, Limits::default()).unwrap();
    let mut sink = Sink(0);
    for entry in archive.entries() {
        archive.extract(entry.id, &mut sink).unwrap();
    }
    assert_eq!(sink.0, 76810);
    assert!(count.get() < 1 << 20, "read {} bytes", count.get());
}
