//! Compatibility cases adapted from sevenz-rust2 0.23.0 (Apache-2.0).
//! See fixtures/sevenz-upstream/NOTICE.md for provenance and scope.
#![cfg(feature = "sevenz")]

use archive_core::{Archive, EntryKind, Limits};
use std::{collections::BTreeMap, fs, io::Cursor, path::PathBuf};

fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sevenz-upstream/resources")
            .join(name),
    )
    .unwrap()
}

fn contents(name: &str) -> BTreeMap<String, Vec<u8>> {
    let mut archive = Archive::open(Cursor::new(fixture(name)), Limits::default()).unwrap();
    let entries = archive.entries().to_vec();
    entries
        .into_iter()
        .filter(|entry| entry.kind == EntryKind::File)
        .map(|entry| {
            (
                entry.name,
                archive.read_entry(entry.id, 16 * 1024 * 1024).unwrap(),
            )
        })
        .collect()
}

macro_rules! content_case {
    ($test:ident, $fixture:literal, $($name:literal => $bytes:expr),+ $(,)?) => {
        #[test]
        fn $test() {
            let expected = BTreeMap::from([$((String::from($name), Vec::from($bytes))),+]);
            assert_eq!(contents($fixture), expected);
        }
    };
}

content_case!(single_empty_file_unencoded_header, "single_empty_file.7z", "empty.txt" => b"");
content_case!(two_empty_files_unencoded_header, "two_empty_file.7z", "file1.txt" => b"", "file2.txt" => b"");
content_case!(lzma_single_file_unencoded_header, "single_file_with_content_lzma.7z", "file.txt" => b"this is a file\n");
content_case!(lzma_multiple_files_encoded_header, "two_files_with_content_lzma.7z", "file1.txt" => b"file one content\n", "file2.txt" => b"file two content\n");
content_case!(delta_lzma_payload, "delta.7z", "delta.txt" => b"aaaabbbbcccc");
content_case!(copy_payload, "copy.7z", "copy.txt" => b"simple copy encoding");
content_case!(lzma2_bcj_x86_payload, "decompress_example_lzma2_bcj_x86.7z", "decompress.exe" => fixture("decompress_x86.exe"));
content_case!(bcj_arm64_payload, "decompress_example_bcj_arm64.7z", "decompress_arm64.exe" => fixture("decompress_arm64.exe"));

#[test]
fn solid_and_non_solid_archives_have_identical_content() {
    let non_solid = contents("non_solid.7z");
    assert!(!non_solid.is_empty());
    assert!(non_solid.values().all(|data| !data.is_empty()));
    assert_eq!(contents("solid.7z"), non_solid);
}

#[test]
fn malformed_coder_stream_counts_are_rejected() {
    assert!(
        Archive::open(
            Cursor::new(fixture("issue_127_coder_stream_overflow.bin")),
            Limits::default()
        )
        .is_err()
    );
}

#[test]
fn bcj2_archive_verifies_every_payload() {
    let mut archive = Archive::open(
        Cursor::new(fixture("7za433_7zip_lzma2_bcj2.7z")),
        Limits::default(),
    )
    .unwrap();
    archive.test().unwrap();
}

#[test]
fn delta_bcj2_graph_preserves_file_sizes_and_checksums() {
    let decoded = contents("delta_bcj2.7z");
    let sizes: BTreeMap<_, _> = decoded
        .into_iter()
        .map(|(name, bytes)| (name, bytes.len()))
        .collect();
    assert_eq!(
        sizes,
        BTreeMap::from([
            ("c/code1.bin".into(), 9000),
            ("c/code2.bin".into(), 7000),
            ("c/wave.bin".into(), 16000),
        ])
    );
}

#[cfg(feature = "bzip2")]
#[test]
fn bzip2_upstream_fixture_matches_expected_payloads() {
    assert_eq!(
        contents("bzip2_file.7z"),
        BTreeMap::from([
            ("hello.txt".into(), b"world\n".to_vec()),
            ("foo.txt".into(), b"bar\n".to_vec()),
        ])
    );
}

#[cfg(feature = "brotli")]
#[test]
fn zstdmt_brotli_upstream_fixture_decodes_license() {
    let decoded = contents("zstdmt-brotli.7z");
    let license = decoded.get("LICENSE").unwrap();
    assert!(String::from_utf8_lossy(license).contains("Apache License"));
}

#[cfg(feature = "crypto")]
#[test]
fn encrypted_fixture_decodes_expected_text() {
    let mut archive = Archive::open_with_password(
        Cursor::new(fixture("encrypted.7z")),
        Limits::default(),
        b"sevenz-rust",
    )
    .unwrap();
    let id = archive
        .entries()
        .iter()
        .find(|entry| entry.name == "encripted/7zFormat.txt")
        .unwrap()
        .id;
    let data = archive.read_entry(id, 16 * 1024 * 1024).unwrap();
    assert!(data.starts_with(b"7z is the new archive format, providing high compression ratio."));
    archive.test().unwrap();
}

#[cfg(feature = "crypto")]
#[test]
fn small_aes_fixture_verifies_actual_payload() {
    let mut archive = Archive::open_with_password(
        Cursor::new(fixture("aes_small_test.7z")),
        Limits::default(),
        b"iBlm8NTigvru0Jr0",
    )
    .unwrap();
    archive.test().unwrap();
}
