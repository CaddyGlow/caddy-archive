//! Adapted security cases from sevenz-rust2 0.23.0 tests/security_tests.rs.
//! Copyright sevenz-rust2 contributors. Apache-2.0; see tests/fixtures/sevenz-upstream/NOTICE.md.
#![cfg(feature = "sevenz")]
use archive_core::{Archive, EntryId, Limits};
use std::io::Cursor;
const END: u8 = 0;
const HEADER: u8 = 1;
const STREAMS: u8 = 4;
const FILES: u8 = 5;
const PACK: u8 = 6;
const UNPACK: u8 = 7;
const SUB: u8 = 8;
const SIZE: u8 = 9;
const FOLDER: u8 = 11;
const UNPACK_SIZE: u8 = 12;
const NUM_UNPACK: u8 = 13;
const NAME: u8 = 17;
fn number(out: &mut Vec<u8>, value: u64) {
    let mut first = 0u8;
    let mut mask = 128u8;
    let mut low = Vec::new();
    for i in 0..8 {
        if value < (1u64 << (7 * (i + 1))) {
            first |= (value >> (8 * i)) as u8;
            break;
        }
        first |= mask;
        mask >>= 1;
        low.push((value >> (8 * i)) as u8);
    }
    out.push(first);
    out.extend_from_slice(&low);
}
fn crc(bytes: &[u8]) -> u32 {
    ms_compress::zlib::crc32::crc32(0, bytes)
}
fn raw(packed: &[u8], header: &[u8], offset: u64, size: u64) -> Vec<u8> {
    let mut start = Vec::new();
    start.extend_from_slice(&offset.to_le_bytes());
    start.extend_from_slice(&size.to_le_bytes());
    start.extend_from_slice(&crc(header).to_le_bytes());
    let mut bytes = b"7z\xbc\xaf\x27\x1c\0\x04".to_vec();
    bytes.extend_from_slice(&crc(&start).to_le_bytes());
    bytes.extend_from_slice(&start);
    bytes.extend_from_slice(packed);
    bytes.extend_from_slice(header);
    bytes
}
fn exact(header: &[u8]) -> Vec<u8> {
    raw(&[], header, 0, header.len() as u64)
}
fn packed(data: &[u8], header: &[u8]) -> Vec<u8> {
    raw(data, header, data.len() as u64, header.len() as u64)
}
fn open_error(bytes: &[u8]) -> bool {
    Archive::open(Cursor::new(bytes), Limits::default()).is_err()
}
fn decode_error(bytes: &[u8]) -> bool {
    match Archive::open(Cursor::new(bytes), Limits::default()) {
        Err(_) => true,
        Ok(mut archive) => archive.test().is_err(),
    }
}
#[test]
fn oversized_next_header_size_is_rejected() {
    assert!(open_error(&raw(&[], &[], 0, 1u64 << 63)));
}
#[test]
fn overflowing_next_header_offset_is_rejected() {
    assert!(open_error(&raw(&[], &[HEADER, END], u64::MAX, 2)));
}
#[test]
fn oversized_num_files_is_rejected() {
    let mut h = vec![HEADER, FILES];
    number(&mut h, 1u64 << 62);
    assert!(open_error(&exact(&h)));
}
#[test]
fn oversized_properties_size_is_rejected() {
    let mut h = vec![HEADER, STREAMS, UNPACK, FOLDER, 1, 0, 1, 0x21, 0];
    number(&mut h, 1u64 << 62);
    assert!(open_error(&exact(&h)));
}
#[test]
fn names_blob_longer_than_num_files_is_rejected() {
    let h = [HEADER, FILES, 1, NAME, 5, 0, 0, 0, 0, 0, END];
    assert!(open_error(&exact(&h)));
}
#[test]
fn more_streamed_files_than_substreams_is_rejected() {
    let h = [
        HEADER,
        STREAMS,
        PACK,
        0,
        1,
        SIZE,
        1,
        END,
        UNPACK,
        FOLDER,
        1,
        0,
        1,
        0,
        UNPACK_SIZE,
        5,
        END,
        SUB,
        END,
        END,
        FILES,
        2,
        END,
        END,
    ];
    assert!(open_error(&exact(&h)));
}
#[test]
fn more_substreams_than_files_is_rejected() {
    let h = [
        HEADER,
        STREAMS,
        PACK,
        0,
        1,
        SIZE,
        5,
        END,
        UNPACK,
        FOLDER,
        1,
        0,
        1,
        1,
        0,
        UNPACK_SIZE,
        5,
        END,
        SUB,
        NUM_UNPACK,
        2,
        SIZE,
        2,
        END,
        END,
        FILES,
        1,
        END,
        END,
    ];
    assert!(decode_error(&packed(&[0xaa; 5], &h)));
}
#[test]
fn block_pack_stream_span_beyond_pack_sizes_is_rejected() {
    let mut h = vec![
        HEADER, STREAMS, PACK, 0, 1, SIZE, 5, END, UNPACK, FOLDER, 1, 0,
    ];
    h.extend_from_slice(&[1, 0x11, 0, 2, 1, 0, 1]);
    h.extend_from_slice(&[UNPACK_SIZE, 5, END, SUB, END, END, FILES, 1, END, END]);
    assert!(decode_error(&packed(&[0xaa; 5], &h)));
}
#[test]
fn files_info_property_with_unbounded_skip_is_rejected() {
    let mut h = vec![HEADER, FILES, 1, 0x19];
    number(&mut h, u64::MAX - 9);
    assert!(open_error(&exact(&h)));
}
#[test]
fn bcj2_coder_with_wrong_input_count_is_rejected() {
    let mut h = vec![
        HEADER, STREAMS, PACK, 0, 2, SIZE, 1, 1, END, UNPACK, FOLDER, 1, 0,
    ];
    h.extend_from_slice(&[1, 0x14, 3, 3, 1, 0x1b, 2, 1, 0, 1]);
    h.extend_from_slice(&[UNPACK_SIZE, 1, END, SUB, END, END, FILES, 1, END, END]);
    assert!(decode_error(&packed(&[0xaa, 0xbb], &h)));
}
#[test]
fn cyclic_coder_bind_pairs_are_rejected() {
    let mut h = vec![0x17, PACK, 0, 1, SIZE, 1, END, UNPACK, FOLDER, 1, 0];
    h.extend_from_slice(&[3, 1, 0, 1, 0, 0x11, 0, 3, 1, 1, 0, 0, 1, 0, 2, 3]);
    h.extend_from_slice(&[UNPACK_SIZE, 1, 1, 1, END, END]);
    assert!(open_error(&exact(&h)));
}
#[test]
fn lzma_coder_with_short_properties_is_rejected() {
    let h = [
        HEADER,
        STREAMS,
        PACK,
        0,
        1,
        SIZE,
        1,
        END,
        UNPACK,
        FOLDER,
        1,
        0,
        1,
        0x23,
        3,
        1,
        1,
        2,
        0,
        0,
        UNPACK_SIZE,
        5,
        END,
        SUB,
        END,
        END,
        FILES,
        1,
        END,
        END,
    ];
    assert!(decode_error(&exact(&h)));
}
#[cfg(feature = "crypto")]
#[test]
fn aes_raw_key_mode_does_not_panic() {
    let h = [
        HEADER,
        STREAMS,
        PACK,
        0,
        1,
        SIZE,
        16,
        END,
        UNPACK,
        FOLDER,
        1,
        0,
        1,
        0x24,
        6,
        0xf1,
        7,
        1,
        2,
        0x3f,
        0,
        UNPACK_SIZE,
        5,
        END,
        SUB,
        END,
        END,
        FILES,
        1,
        END,
        END,
    ];
    let bytes = packed(&[0; 16], &h);
    if let Ok(mut archive) =
        Archive::open_with_password(Cursor::new(bytes), Limits::default(), b"x")
    {
        let _ = archive.test();
    }
}
#[cfg(feature = "crypto")]
#[test]
fn aes_decode_survives_one_byte_reads() {
    use archive_core::{
        CreateEntry, CreateOptions, EntryKind, Format, RandomSource, create_with_options,
    };
    struct Drip<R>(R);
    impl<R: std::io::Read> std::io::Read for Drip<R> {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            if bytes.is_empty() {
                return Ok(0);
            }
            self.0.read(&mut bytes[..1])
        }
    }
    impl<R: std::io::Seek> std::io::Seek for Drip<R> {
        fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
            self.0.seek(position)
        }
    }
    struct Random(u8);
    impl RandomSource for Random {
        fn fill(&mut self, bytes: &mut [u8]) -> archive_core::Result<()> {
            for b in bytes {
                self.0 = self.0.wrapping_add(1);
                *b = self.0;
            }
            Ok(())
        }
    }
    let original: Vec<_> = (0..200u32).map(|i| i as u8).collect();
    let entry = CreateEntry {
        name: "data".into(),
        data: original.clone(),
        kind: EntryKind::File,
    };
    let mut output = Cursor::new(Vec::new());
    create_with_options(
        Format::SevenZip,
        &[entry],
        &mut output,
        Limits::default(),
        CreateOptions {
            password: Some(b"pw"),
            randomness: Some(&mut Random(0)),
            ..Default::default()
        },
    )
    .unwrap();
    let mut archive = Archive::open_with_password(
        Drip(Cursor::new(output.into_inner())),
        Limits::default(),
        b"pw",
    )
    .unwrap();
    assert_eq!(archive.read_entry(EntryId(0), 200).unwrap(), original);
    let fixture = include_bytes!("fixtures/sevenz-upstream/resources/aes_small_test.7z");
    let mut archive = Archive::open_with_password(
        Drip(Cursor::new(fixture.as_slice())),
        Limits::default(),
        b"iBlm8NTigvru0Jr0",
    )
    .unwrap();
    assert!(archive.test().unwrap().verified);
}
#[test]
fn delta_distance_256_decodes_without_wrapping() {
    let original: Vec<_> = (0..1024u32).map(|i| i.wrapping_mul(31) as u8).collect();
    let encoded: Vec<_> = original
        .iter()
        .enumerate()
        .map(|(i, b)| b.wrapping_sub(if i >= 256 { original[i - 256] } else { 0 }))
        .collect();
    let mut h = vec![HEADER, STREAMS, PACK, 0, 1, SIZE];
    number(&mut h, encoded.len() as u64);
    h.extend_from_slice(&[END, UNPACK, FOLDER, 1, 0, 1, 0x21, 3, 1, 255, UNPACK_SIZE]);
    number(&mut h, original.len() as u64);
    h.extend_from_slice(&[END, SUB, END, END, FILES, 1, NAME, 11, 0]);
    for unit in "data\0".encode_utf16() {
        h.extend_from_slice(&unit.to_le_bytes());
    }
    h.extend_from_slice(&[END, END]);
    let mut archive = Archive::open(Cursor::new(packed(&encoded, &h)), Limits::default()).unwrap();
    assert_eq!(archive.read_entry(EntryId(0), 1024).unwrap(), original);
}
