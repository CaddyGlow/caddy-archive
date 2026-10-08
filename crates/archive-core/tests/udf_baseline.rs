#![cfg(feature = "udf")]
use archive_core::{Limits, udf::UdfArchive};

fn fixture() -> (tempfile::TempDir, Vec<u8>) {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    std::fs::create_dir_all(source.join("boot")).unwrap();
    std::fs::create_dir_all(source.join("efi/microsoft/boot")).unwrap();
    std::fs::write(source.join("boot/etfsboot.com"), [1; 4096]).unwrap();
    std::fs::write(source.join("efi/microsoft/boot/efisys.bin"), [2; 4096]).unwrap();
    std::fs::write(source.join("payload.txt"), b"UDF payload").unwrap();
    let output = directory.path().join("media.iso");
    libmkiso::write_iso(&source, &output).unwrap();
    let bytes = std::fs::read(output).unwrap();
    (directory, bytes)
}

#[test]
fn udf_writer_payload_is_read_by_portable_reader_and_7z() {
    let (directory, bytes) = fixture();
    let archive = UdfArchive::open(&bytes, Limits::default()).unwrap();
    let file = archive
        .entries()
        .iter()
        .find(|file| file.name == "payload.txt")
        .unwrap();
    assert_eq!(archive.read_entry(file.id, 100).unwrap(), b"UDF payload");
    assert!(archive.test().unwrap().verified);
    if std::process::Command::new("7z").arg("i").output().is_ok() {
        let output = std::process::Command::new("7z")
            .args(["e", "-so"])
            .arg(directory.path().join("media.iso"))
            .arg("payload.txt")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"UDF payload");
    }
}

#[test]
fn udf_descriptor_corruption_and_metadata_budget_are_rejected() {
    let (_, mut bytes) = fixture();
    let limits = Limits {
        max_metadata_bytes: 1,
        ..Limits::default()
    };
    assert!(UdfArchive::open(&bytes, limits).is_err());
    let last = bytes.len() / 2048 - 1;
    for block in [256, last, last - 256] {
        bytes[block * 2048 + 20] ^= 1;
    }
    assert!(UdfArchive::open(&bytes, Limits::default()).is_err());
}

#[test]
fn seekable_udf_matches_borrowed_input_without_whole_image_read() {
    use std::{
        cell::Cell,
        io::{self, Read, Seek, SeekFrom},
        rc::Rc,
    };
    struct Counted {
        file: std::fs::File,
        count: Rc<Cell<usize>>,
    }
    impl Read for Counted {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            let n = self.file.read(bytes)?;
            self.count.set(self.count.get() + n);
            Ok(n)
        }
    }
    impl Seek for Counted {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            self.file.seek(position)
        }
    }
    let (directory, bytes) = fixture();
    let count = Rc::new(Cell::new(0));
    let source = Counted {
        file: std::fs::File::open(directory.path().join("media.iso")).unwrap(),
        count: count.clone(),
    };
    let archive = UdfArchive::open_reader(source, Limits::default()).unwrap();
    let file = archive
        .entries()
        .iter()
        .find(|entry| entry.name == "payload.txt")
        .unwrap();
    assert_eq!(archive.read_entry(file.id, 100).unwrap(), b"UDF payload");
    assert!(
        count.get() < bytes.len(),
        "read {} of {} bytes",
        count.get(),
        bytes.len()
    );
}
