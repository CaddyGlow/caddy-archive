#![cfg(unix)]

use archive_core::{CreateEntry, CreateOptions, EntryKind, Format, Limits, ZipCompression};
use std::{
    io::Cursor,
    path::Path,
    process::{Command, Output},
};

fn tempdir() -> tempfile::TempDir {
    // Use an admitted physical path when the platform temporary root is an alias.
    tempfile::tempdir_in(std::fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap()
}

fn command(args: &[&str], archive: &Path) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_arc"));
    command.args(["--json", args[0]]).arg(archive);
    match args[0] {
        "delete" => {
            command.arg("--name");
        }
        "rename" => {
            command.arg("--pair");
        }
        _ => {}
    }
    command.args(&args[1..]).output().unwrap()
}

fn fixture(path: &Path, names: &[&str]) {
    let entries: Vec<_> = names
        .iter()
        .map(|name| CreateEntry {
            name: (*name).into(),
            data: if name.ends_with('/') {
                Vec::new()
            } else {
                name.as_bytes().repeat(10)
            },
            kind: if name.ends_with('/') {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
        })
        .collect();
    let mut output = Cursor::new(Vec::new());
    archive_core::create_with_options(
        Format::Zip,
        &entries,
        &mut output,
        Limits::default(),
        CreateOptions {
            zip_compression: ZipCompression::Copy,
            ..Default::default()
        },
    )
    .unwrap();
    std::fs::write(path, output.into_inner()).unwrap();
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn u16at(bytes: &[u8], at: usize) -> usize {
    usize::from(u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap()))
}
fn u32at(bytes: &[u8], at: usize) -> usize {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize
}
fn end(bytes: &[u8]) -> usize {
    bytes
        .windows(4)
        .rposition(|window| window == b"PK\x05\x06")
        .unwrap()
}
fn packed(bytes: &[u8]) -> &[u8] {
    let directory = u32at(bytes, end(bytes) + 16);
    let local = u32at(bytes, directory + 42);
    let start = local + 30 + u16at(bytes, local + 26) + u16at(bytes, local + 28);
    &bytes[start..start + u32at(bytes, directory + 20)]
}

#[test]
fn encrypted_rename_reuses_ciphertext_without_credentials_and_extracts_independently() {
    let root = tempdir();
    let input = root.path().join("input");
    std::fs::create_dir(&input).unwrap();
    std::fs::write(input.join("payload"), b"secret content").unwrap();
    let password = root.path().join("password");
    std::fs::write(&password, b"correct\n").unwrap();
    let archive = root.path().join("archive.zip");
    let created = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "--password-file"])
        .arg(&password)
        .args(["create", "--encrypt", "--format", "zip", "--input"])
        .arg(input)
        .arg("--output")
        .arg(&archive)
        .output()
        .unwrap();
    assert!(created.status.success(), "{}", json(&created));
    let original = std::fs::read(&archive).unwrap();
    let renamed = command(&["rename", "payload", "renamed"], &archive);
    assert!(renamed.status.success(), "{}", json(&renamed));
    assert_eq!(json(&renamed)["payloads_verified"], false);
    assert_eq!(packed(&original), packed(&std::fs::read(&archive).unwrap()));
    let extraction = root.path().join("independent");
    let reference = std::env::var_os("ARCHIVE_TEST_7ZIP").unwrap_or_else(|| "7z".into());
    let extracted = Command::new(reference)
        .args(["x", "-y", "-pcorrect"])
        .arg(&archive)
        .arg(format!("-o{}", extraction.display()))
        .output();
    if let Err(error) = &extracted
        && error.kind() == std::io::ErrorKind::NotFound
    {
        eprintln!("independent 7-Zip unavailable; ciphertext reuse still verified");
        return;
    }
    let extracted = extracted.unwrap();
    assert!(
        extracted.status.success(),
        "{}",
        String::from_utf8_lossy(&extracted.stderr)
    );
    assert_eq!(
        std::fs::read(extraction.join("renamed")).unwrap(),
        b"secret content"
    );
}

#[test]
fn directory_delete_respects_path_boundaries_and_can_verify_remaining_payloads() {
    let root = tempdir();
    let archive = root.path().join("archive.zip");
    fixture(&archive, &["dir/", "dir/a", "dir/sub/b", "dir2/a"]);
    let deleted = command(&["delete", "dir/", "--verify"], &archive);
    assert!(deleted.status.success(), "{}", json(&deleted));
    assert_eq!(json(&deleted)["removed_entries"], 3);
    assert_eq!(json(&deleted)["payloads_verified"], true);
    let listed = command(&["list"], &archive);
    assert_eq!(json(&listed)["entries"].as_array().unwrap().len(), 1);
    assert_eq!(json(&listed)["entries"][0]["name"], "dir2/a");
}

#[test]
fn dry_run_has_decisions_and_collisions_preserve_original() {
    let root = tempdir();
    let archive = root.path().join("archive.zip");
    fixture(&archive, &["a", "b"]);
    let original = std::fs::read(&archive).unwrap();
    let planned = command(&["rename", "a", "c", "--dry-run"], &archive);
    assert!(planned.status.success(), "{}", json(&planned));
    assert_eq!(json(&planned)["published"], false);
    assert_eq!(json(&planned)["entries"][0]["result_name"], "c");
    let collision = command(&["rename", "a", "b", "--dry-run"], &archive);
    assert!(!collision.status.success());
    assert_eq!(json(&collision)["error"]["code"], 4);
    assert_eq!(std::fs::read(&archive).unwrap(), original);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn output_alias_and_existing_output_are_rejected() {
    let root = tempdir();
    let archive = root.path().join("archive.zip");
    fixture(&archive, &["a"]);
    let original = std::fs::read(&archive).unwrap();
    let alias = root.path().join("alias.zip");
    std::fs::hard_link(&archive, &alias).unwrap();
    let output = command(
        &["rename", "a", "b", "--output", alias.to_str().unwrap()],
        &archive,
    );
    assert!(!output.status.success());
    let other = root.path().join("other.zip");
    std::fs::write(&other, b"do not overwrite").unwrap();
    let output = command(
        &["rename", "a", "b", "--output", other.to_str().unwrap()],
        &archive,
    );
    assert!(!output.status.success());
    assert_eq!(std::fs::read(other).unwrap(), b"do not overwrite");
    assert_eq!(std::fs::read(archive).unwrap(), original);
}

#[test]
fn new_output_leaves_original_byte_identical() {
    let root = tempdir();
    let archive = root.path().join("archive.zip");
    fixture(&archive, &["a", "b"]);
    let original = std::fs::read(&archive).unwrap();
    let output = root.path().join("new.zip");
    let result = command(
        &["delete", "a", "--output", output.to_str().unwrap()],
        &archive,
    );
    assert!(result.status.success(), "{}", json(&result));
    assert_eq!(std::fs::read(archive).unwrap(), original);
    assert_eq!(
        json(&command(&["list"], &output))["entries"][0]["name"],
        "b"
    );
}

#[test]
fn unsupported_extra_field_is_rejected_before_creating_output() {
    let root = tempdir();
    let archive = root.path().join("archive.zip");
    fixture(&archive, &["a"]);
    let mut bytes = std::fs::read(&archive).unwrap();
    let old_end = end(&bytes);
    let directory = u32at(&bytes, old_end + 16);
    let directory_size = u32at(&bytes, old_end + 12);
    let local_extra = u16at(&bytes, 28);
    let central_extra = u16at(&bytes, directory + 30);
    let central_extra_end = directory + 46 + u16at(&bytes, directory + 28) + central_extra;
    bytes[directory + 30..directory + 32]
        .copy_from_slice(&((central_extra + 4) as u16).to_le_bytes());
    bytes.splice(central_extra_end..central_extra_end, [0xef, 0xbe, 0, 0]);
    bytes[28..30].copy_from_slice(&((local_extra + 4) as u16).to_le_bytes());
    let local_extra_end = 30 + u16at(&bytes, 26) + local_extra;
    bytes.splice(local_extra_end..local_extra_end, [0xef, 0xbe, 0, 0]);
    let new_end = end(&bytes);
    bytes[new_end + 12..new_end + 16].copy_from_slice(&((directory_size + 4) as u32).to_le_bytes());
    bytes[new_end + 16..new_end + 20].copy_from_slice(&((directory + 4) as u32).to_le_bytes());
    std::fs::write(&archive, &bytes).unwrap();
    let output = root.path().join("new.zip");
    let result = command(
        &["rename", "a", "b", "--output", output.to_str().unwrap()],
        &archive,
    );
    assert!(!result.status.success());
    assert!(!output.exists());
    assert_eq!(json(&result)["error"]["code"], 3);
    assert_eq!(std::fs::read(archive).unwrap(), bytes);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn verification_failure_preserves_encrypted_original() {
    let root = tempdir();
    let input = root.path().join("input");
    std::fs::create_dir(&input).unwrap();
    std::fs::write(input.join("a"), b"secret").unwrap();
    let password = root.path().join("password");
    std::fs::write(&password, b"correct\n").unwrap();
    let archive = root.path().join("archive.zip");
    let output = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "--password-file"])
        .arg(password)
        .args(["create", "--encrypt", "--input"])
        .arg(input)
        .arg("--output")
        .arg(&archive)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", json(&output));
    let original = std::fs::read(&archive).unwrap();
    let failed = command(&["rename", "a", "b", "--verify"], &archive);
    assert!(!failed.status.success());
    assert_eq!(json(&failed)["error"]["code"], 6);
    assert_eq!(std::fs::read(archive).unwrap(), original);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 3);
}

#[test]
fn generic_zip_edits_refuse_package_markers() {
    for marker in [
        "AppxManifest.xml",
        "AppxMetadata/AppxBundleManifest.xml",
        "META-INF/SIGNER.RSA",
    ] {
        let root = tempdir();
        let archive = root.path().join("archive.zip");
        fixture(&archive, &[marker, "payload"]);
        let original = std::fs::read(&archive).unwrap();
        let result = command(&["delete", "payload", "--dry-run"], &archive);
        assert!(!result.status.success());
        assert_eq!(json(&result)["error"]["code"], 3);
        assert_eq!(std::fs::read(archive).unwrap(), original);
    }
}

#[test]
fn irrelevant_effective_options_are_rejected_before_source_io() {
    let root = tempdir();
    let missing = root.path().join("missing.zip");
    for options in [
        vec!["--password-file", "missing-password"],
        vec!["--media", "disk=missing"],
        vec!["--bundle-entry", "payload"],
        vec!["--image", "1"],
        vec!["--image-name", "Windows"],
        vec!["--view", "iso"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_arc"))
            .arg("--json")
            .args(options)
            .arg("delete")
            .arg(&missing)
            .args(["--name", "payload", "--dry-run"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(json(&output)["error"]["code"], 3);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
}
