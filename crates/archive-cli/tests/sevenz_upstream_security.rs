// Adapted from sevenz-rust2 0.23.0 tests/decompression_tests.rs (Apache-2.0).
// Source: https://docs.rs/crate/sevenz-rust2/0.23.0/source/tests/decompression_tests.rs
// Retained license: archive-core/tests/fixtures/sevenz-upstream/LICENSE-APACHE-2.0.
// This adaptation exercises arc and archive-fs without importing sevenz-rust2.
use std::{
    io::Cursor,
    path::Path,
    process::{Command, Output},
};

fn archive_with_name(name: &str) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    archive_core::create(
        archive_core::Format::SevenZip,
        &[archive_core::CreateEntry {
            name: name.into(),
            data: b"pwned".to_vec(),
            kind: archive_core::EntryKind::File,
        }],
        &mut bytes,
        archive_core::Limits::default(),
    )
    .unwrap();
    bytes.into_inner()
}
fn extract(bytes: &[u8], archive: &Path, destination: &Path) -> Output {
    std::fs::write(archive, bytes).unwrap();
    Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "extract"])
        .arg(archive)
        .arg("--output")
        .arg(destination)
        .output()
        .unwrap()
}

#[test]
fn path_traversal_relative_entry_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let escaped = root.path().join("sevenz_pwned_relative");
    let destination = root.path().join("out");
    let result = extract(
        &archive_with_name("../sevenz_pwned_relative"),
        &root.path().join("relative.7z"),
        &destination,
    );
    assert!(!result.status.success());
    assert!(!escaped.exists());
    assert_eq!(std::fs::read_dir(destination).unwrap().count(), 0);
}
#[test]
fn path_traversal_absolute_entry_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let escaped = root.path().join("sevenz_pwned_abs");
    let destination = root.path().join("out");
    let result = extract(
        &archive_with_name(escaped.to_str().unwrap()),
        &root.path().join("absolute.7z"),
        &destination,
    );
    assert!(!result.status.success());
    assert!(!escaped.exists());
    assert_eq!(std::fs::read_dir(destination).unwrap().count(), 0);
}
#[test]
fn backslash_traversal_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("out");
    let escaped = root.path().join("sevenz_pwned_backslash");
    let result = extract(
        &archive_with_name("..\\sevenz_pwned_backslash"),
        &root.path().join("backslash.7z"),
        &destination,
    );
    assert!(!result.status.success());
    assert!(!escaped.exists());
    assert_eq!(std::fs::read_dir(destination).unwrap().count(), 0);
}
#[test]
fn normal_nested_entry_still_extracts() {
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("out");
    let result = extract(
        &archive_with_name("a/b/c.txt"),
        &root.path().join("nested.7z"),
        &destination,
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert_eq!(
        std::fs::read(destination.join("a/b/c.txt")).unwrap(),
        b"pwned"
    );
}
#[test]
fn existing_file_is_preserved() {
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("out");
    std::fs::create_dir(&destination).unwrap();
    std::fs::write(destination.join("existing"), b"user data").unwrap();
    let result = extract(
        &archive_with_name("existing"),
        &root.path().join("existing.7z"),
        &destination,
    );
    assert!(!result.status.success());
    assert_eq!(
        std::fs::read(destination.join("existing")).unwrap(),
        b"user data"
    );
    assert_eq!(std::fs::read_dir(destination).unwrap().count(), 1);
}
#[test]
fn native_destination_defense_in_depth_rejects_parent_component() {
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("out");
    std::fs::create_dir(&destination).unwrap();
    let mut native = archive_fs::Destination::open(&destination).unwrap();
    assert!(
        native
            .file(b"../sevenz_pwned_direct", |sink| {
                use std::io::Write;
                sink.write_all(b"pwned")?;
                Ok(5)
            })
            .is_err()
    );
    assert!(!root.path().join("sevenz_pwned_direct").exists());
}
