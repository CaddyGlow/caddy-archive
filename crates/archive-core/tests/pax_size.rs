#![cfg(feature = "tar")]
use archive_core::{Archive, EntryId, Error, Limits};
use std::io::Cursor;

fn fixture(stored_size: u64, override_size: u64) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    let size = override_size.to_string();
    builder
        .append_pax_extensions([("size", size.as_bytes())])
        .unwrap();
    let mut header = tar::Header::new_ustar();
    header.set_path("payload").unwrap();
    header.set_size(stored_size);
    header.set_mode(0o644);
    header.set_mtime(0);
    header.set_uid(0);
    header.set_gid(0);
    header.set_cksum();
    // The PAX value, not the legacy header value, describes the actual payload.
    builder.append(&header, b"hello".as_slice()).unwrap();
    header.set_path("next").unwrap();
    header.set_size(1);
    header.set_cksum();
    builder.append(&header, b"!".as_slice()).unwrap();
    builder.into_inner().unwrap()
}

#[test]
fn indexed_tar_uses_pax_size_and_resets_it_for_next_member() {
    for stored in [0, 1024] {
        let mut archive =
            Archive::open(Cursor::new(fixture(stored, 5)), Limits::default()).unwrap();
        assert_eq!(archive.read_entry(EntryId(0), 5).unwrap(), b"hello");
        assert_eq!(archive.read_entry(EntryId(1), 1).unwrap(), b"!");
    }
}

#[test]
fn pax_size_still_obeys_input_and_decoded_limits() {
    assert!(Archive::open(Cursor::new(fixture(0, 1 << 20)), Limits::default()).is_err());
    assert!(matches!(
        Archive::open(
            Cursor::new(fixture(0, 5)),
            Limits {
                max_entry_bytes: 4,
                ..Limits::default()
            }
        ),
        Err(Error::ResourceLimit(_))
    ));
}
