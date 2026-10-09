#![cfg(all(
    feature = "zip",
    feature = "gzip",
    feature = "xz",
    feature = "streams",
    feature = "bzip2",
    feature = "brotli"
))]
use archive_core::{Archive, CreateEntry, EntryId, EntryKind, Error, Format, Limits, create};
use std::io::Cursor;

fn fixture(format: Format) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    let entries: Vec<_> = ["first", "other"]
        .into_iter()
        .map(|name| CreateEntry {
            name: name.into(),
            data: vec![0; 128 * 1024],
            kind: EntryKind::File,
        })
        .collect();
    create(format, &entries, &mut out, Limits::default()).unwrap();
    out.into_inner()
}

#[test]
fn zip_bomb_declared_size_and_count_limits() {
    let bytes = fixture(Format::Zip);
    for limits in [
        Limits {
            max_entry_bytes: 1024,
            ..Limits::default()
        },
        Limits {
            max_total_bytes: 200_000,
            ..Limits::default()
        },
        Limits {
            max_entries: 1,
            ..Limits::default()
        },
    ] {
        assert!(matches!(
            Archive::open(Cursor::new(&bytes), limits),
            Err(Error::ResourceLimit(_))
        ));
    }
}

#[test]
fn zip_false_decoded_size_cannot_escape_output_limit() {
    let mut bytes = fixture(Format::Zip);
    let central = bytes.windows(4).position(|v| v == b"PK\x01\x02").unwrap();
    // Small archives carry decoded sizes directly in the central header.
    let size = central + 24;
    bytes[size..size + 4].copy_from_slice(&1u32.to_le_bytes());
    let mut archive = Archive::open(Cursor::new(bytes), Limits::default()).unwrap();
    let mut output = Vec::new();
    assert!(matches!(
        archive.extract(EntryId(0), &mut output),
        Err(Error::ResourceLimit(_))
    ));
    assert!(output.len() <= 1);
}

#[test]
fn zip_overlapping_payloads_rejected_before_extraction() {
    let mut bytes = fixture(Format::Zip);
    let central = bytes.windows(4).position(|v| v == b"PK\x01\x02").unwrap();
    let size = central + 20;
    let compressed = u32::from_le_bytes(bytes[size..size + 4].try_into().unwrap());
    bytes[size..size + 4].copy_from_slice(&(compressed + 1).to_le_bytes());
    assert!(
        matches!(Archive::open(Cursor::new(bytes), Limits::default()),
        Err(Error::Malformed(message)) if message == "overlapping ZIP members")
    );
}

#[test]
fn indexed_compressed_tar_buffer_is_bounded() {
    for format in [
        Format::TarGzip,
        Format::TarXz,
        Format::TarBzip2,
        Format::TarBrotli,
    ] {
        let bytes = fixture(format);
        let limits = Limits {
            max_buffered_bytes: 1024,
            ..Limits::default()
        };
        let err = Archive::open_as(Cursor::new(&bytes), format, limits)
            .err()
            .unwrap();
        assert!(
            err.to_string().contains("buffered decoded bytes"),
            "{format:?}: {err}"
        );
        assert!(Archive::open_as(Cursor::new(&bytes), format, Limits::default()).is_ok());
    }
}

#[test]
fn scratch_spooling_avoids_decoded_memory_limit() {
    use std::io::Write;
    for format in [
        Format::TarGzip,
        Format::TarXz,
        Format::TarBzip2,
        Format::TarBrotli,
    ] {
        let mut source = tempfile::tempfile().unwrap();
        source.write_all(&fixture(format)).unwrap();
        let limits = Limits {
            max_buffered_bytes: 0,
            ..Limits::default()
        };
        let mut archive = Archive::open_with_scratch(
            source,
            tempfile::tempfile().unwrap(),
            limits,
            Some(format),
            None,
        )
        .unwrap();
        assert_eq!(archive.format(), format);
        assert_eq!(
            archive.read_entry(EntryId(1), 128 * 1024).unwrap(),
            vec![0; 128 * 1024]
        );
    }
}

#[test]
fn scratch_autodetection_preserves_raw_stream_and_tar_identity() {
    for format in [Format::TarGzip, Format::TarXz, Format::TarBzip2] {
        let mut archive = Archive::open_with_scratch(
            Cursor::new(fixture(format)),
            Cursor::new(Vec::new()),
            Limits {
                max_buffered_bytes: 0,
                ..Limits::default()
            },
            None,
            None,
        )
        .unwrap();
        assert_eq!(archive.format(), format);
        assert_eq!(archive.test().unwrap().entries, 2);
    }
    let mut raw = Cursor::new(Vec::new());
    create(
        Format::Gzip,
        &[CreateEntry {
            name: "data".into(),
            data: vec![42; 100],
            kind: EntryKind::File,
        }],
        &mut raw,
        Limits::default(),
    )
    .unwrap();
    let archive =
        Archive::open_with_scratch(raw, Cursor::new(Vec::new()), Limits::default(), None, None)
            .unwrap();
    assert_eq!(archive.format(), Format::Gzip);
}

#[test]
fn zip_comments_may_contain_end_record_signatures() {
    for comment in [
        b"PK\x05\x06".as_slice(),
        b"PK\x05\x06 followed by more than 22 bytes of comment",
    ] {
        let mut bytes = fixture(Format::Zip);
        let end = bytes.len();
        bytes[end - 2..].copy_from_slice(&(comment.len() as u16).to_le_bytes());
        bytes.extend_from_slice(comment);
        let mut archive = Archive::open(Cursor::new(bytes), Limits::default()).unwrap();
        assert_eq!(archive.test().unwrap().entries, 2);
    }
}
