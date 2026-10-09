#![cfg(unix)]
use archive_core::{Archive, Limits, StoredTimestamp};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn tempdir() -> tempfile::TempDir {
    // macOS /var and other configured temporary roots can be symlink aliases.
    // Admit a canonical root without relaxing the transaction's no-follow policy.
    tempfile::tempdir_in(std::fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap()
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "{} / {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}
fn success(output: Output) -> serde_json::Value {
    assert!(output.status.success(), "{}", json(&output));
    json(&output)
}
fn run(archive: &Path, args: &[&str], old_password: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_arc"));
    command.arg("--json");
    if let Some(password) = old_password {
        command.arg("--password-file").arg(password);
    }
    command
        .arg("edit")
        .arg(archive)
        .args(args)
        .output()
        .unwrap()
}
fn fixture(root: &Path, format: &str) -> PathBuf {
    let input = root.join(format!("input-{format}"));
    std::fs::create_dir(&input).unwrap();
    std::fs::write(input.join("private"), b"private payload\n".repeat(100)).unwrap();
    std::fs::write(input.join("public"), b"public payload\n".repeat(100)).unwrap();
    let archive = root.join(format!("archive.{format}"));
    success(
        Command::new(env!("CARGO_BIN_EXE_arc"))
            .args(["--json", "create", "--compression", "copy", "--input"])
            .arg(input)
            .arg("--output")
            .arg(&archive)
            .output()
            .unwrap(),
    );
    archive
}
fn password(root: &Path, name: &str, value: &[u8]) -> PathBuf {
    let path = root.join(name);
    std::fs::write(&path, value).unwrap();
    path
}
fn extract(archive: &Path, destination: &Path, name: &str, password: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_arc"));
    command.args(["--json", "--include", name, "--literal-names"]);
    if let Some(password) = password {
        command.arg("--password-file").arg(password);
    }
    command
        .arg("extract")
        .arg(archive)
        .arg("--output")
        .arg(destination)
        .output()
        .unwrap()
}

#[test]
fn timestamps_change_selected_zip_and_sevenz_entries_with_verified_payloads() {
    let root = tempdir();
    for format in ["zip", "7z"] {
        let archive = fixture(root.path(), format);
        let mut before =
            Archive::open(std::fs::File::open(&archive).unwrap(), Limits::default()).unwrap();
        let public = before
            .entries()
            .iter()
            .find(|entry| entry.name == "public")
            .unwrap()
            .id;
        let old_public = before.entry_metadata(public).unwrap();
        let report = success(run(
            &archive,
            &[
                "--name",
                "private",
                "--modified-unix-seconds",
                "1700000123",
                "--verify",
            ],
            None,
        ));
        assert_eq!(report["payloads_verified"], true);
        let mut after =
            Archive::open(std::fs::File::open(&archive).unwrap(), Limits::default()).unwrap();
        let private = after
            .entries()
            .iter()
            .find(|entry| entry.name == "private")
            .unwrap()
            .id;
        assert_eq!(
            after.entry_metadata(private).unwrap().modified,
            Some(StoredTimestamp::UnixSeconds(1700000123))
        );
        assert_eq!(after.entry_metadata(public).unwrap(), old_public);
        after.test().unwrap();
    }
}

fn mixed_password_edits(format: &str) {
    let root = tempdir();
    let archive = fixture(root.path(), format);
    let first = password(root.path(), "first", b"first-password\n");
    let second = password(root.path(), "second", b"second-password\n");
    let third = password(root.path(), "third", b"third-password\n");
    success(run(
        &archive,
        &[
            "--name",
            "private",
            "--encrypt",
            "--new-password-file",
            first.to_str().unwrap(),
        ],
        None,
    ));
    success(run(
        &archive,
        &[
            "--name",
            "public",
            "--encrypt",
            "--new-password-file",
            second.to_str().unwrap(),
        ],
        None,
    ));
    success(extract(
        &archive,
        &root.path().join("first-extract"),
        "private",
        Some(&first),
    ));
    success(extract(
        &archive,
        &root.path().join("second-extract"),
        "public",
        Some(&second),
    ));
    let before_failure = std::fs::read(&archive).unwrap();
    let failed = run(
        &archive,
        &[
            "--name",
            "private",
            "--encrypt",
            "--new-password-file",
            third.to_str().unwrap(),
        ],
        Some(&second),
    );
    assert!(!failed.status.success());
    assert_eq!(std::fs::read(&archive).unwrap(), before_failure);
    assert!(!String::from_utf8_lossy(&failed.stdout).contains("first-password"));
    success(run(
        &archive,
        &[
            "--name",
            "private",
            "--encrypt",
            "--new-password-file",
            third.to_str().unwrap(),
        ],
        Some(&first),
    ));
    success(extract(
        &archive,
        &root.path().join("third-extract"),
        "private",
        Some(&third),
    ));
    success(extract(
        &archive,
        &root.path().join("retained-extract"),
        "public",
        Some(&second),
    ));
    success(run(
        &archive,
        &["--name", "private", "--decrypt"],
        Some(&third),
    ));
    success(extract(
        &archive,
        &root.path().join("clear-extract"),
        "private",
        None,
    ));
    success(extract(
        &archive,
        &root.path().join("still-encrypted"),
        "public",
        Some(&second),
    ));
    assert_eq!(
        std::fs::read(root.path().join("clear-extract/private")).unwrap(),
        b"private payload\n".repeat(100)
    );
}

#[test]
fn selected_zip_entries_keep_distinct_passwords_and_can_be_rekeyed_and_decrypted() {
    mixed_password_edits("zip");
}

#[test]
fn selected_non_solid_sevenz_entries_keep_distinct_passwords_and_can_be_rekeyed_and_decrypted() {
    mixed_password_edits("7z");
}

#[test]
fn whole_sevenz_encryption_hides_names_and_whole_decryption_removes_header_password() {
    let root = tempdir();
    let archive = fixture(root.path(), "7z");
    let credential = password(root.path(), "password", b"hidden-secret\n");
    let report = success(run(
        &archive,
        &[
            "--encrypt",
            "--encrypt-headers",
            "--new-password-file",
            credential.to_str().unwrap(),
            "--verify",
        ],
        None,
    ));
    assert_eq!(report["details"]["headers_encrypted"], true);
    let no_password = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "list"])
        .arg(&archive)
        .output()
        .unwrap();
    assert!(!no_password.status.success());
    assert!(!String::from_utf8_lossy(&no_password.stdout).contains("private"));
    success(extract(
        &archive,
        &root.path().join("decrypted"),
        "private",
        Some(&credential),
    ));
    let metadata = success(run(
        &archive,
        &[
            "--name",
            "private",
            "--modified-unix-seconds",
            "1700000123",
            "--verify",
        ],
        Some(&credential),
    ));
    assert_eq!(metadata["details"]["headers_encrypted"], true);
    let report = success(run(&archive, &["--decrypt", "--verify"], Some(&credential)));
    assert_eq!(report["details"]["headers_encrypted"], false);
    success(
        Command::new(env!("CARGO_BIN_EXE_arc"))
            .args(["--json", "list"])
            .arg(&archive)
            .output()
            .unwrap(),
    );
    success(extract(
        &archive,
        &root.path().join("clear"),
        "private",
        None,
    ));
}

#[test]
fn dry_run_has_no_outputs_and_alias_or_missing_credentials_preserve_sources() {
    let root = tempdir();
    let archive = fixture(root.path(), "zip");
    let new_password = password(root.path(), "new", b"preview-password\n");
    let original = std::fs::read(&archive).unwrap();
    let output = root.path().join("new.zip");
    let before_count = std::fs::read_dir(root.path()).unwrap().count();
    let dry = success(run(
        &archive,
        &[
            "--name",
            "private",
            "--encrypt",
            "--new-password-file",
            new_password.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--dry-run",
        ],
        None,
    ));
    assert_eq!(dry["published"], false);
    assert!(!output.exists());
    assert_eq!(
        std::fs::read_dir(root.path()).unwrap().count(),
        before_count
    );
    assert!(!dry.to_string().contains("preview-password"));
    assert!(!dry.to_string().contains(new_password.to_str().unwrap()));
    let alias = root.path().join("alias.zip");
    std::fs::hard_link(&archive, &alias).unwrap();
    let failed = run(
        &archive,
        &[
            "--modified-unix-seconds",
            "1700000123",
            "--output",
            alias.to_str().unwrap(),
        ],
        None,
    );
    assert!(!failed.status.success());
    assert_eq!(std::fs::read(&archive).unwrap(), original);
    std::fs::remove_file(alias).unwrap();
    success(run(
        &archive,
        &[
            "--name",
            "private",
            "--encrypt",
            "--new-password-file",
            new_password.to_str().unwrap(),
        ],
        None,
    ));
    let encrypted = std::fs::read(&archive).unwrap();
    let failed = run(
        &archive,
        &[
            "--name",
            "private",
            "--decrypt",
            "--output",
            output.to_str().unwrap(),
        ],
        None,
    );
    assert!(!failed.status.success());
    assert!(!output.exists());
    assert_eq!(std::fs::read(&archive).unwrap(), encrypted);
}

#[test]
fn zip_header_encryption_and_selected_sevenz_header_encryption_fail_without_changes() {
    let root = tempdir();
    let credential = password(root.path(), "new", b"new\n");
    for format in ["zip", "7z"] {
        let archive = fixture(root.path(), format);
        let original = std::fs::read(&archive).unwrap();
        let mut args = vec![
            "--encrypt",
            "--encrypt-headers",
            "--new-password-file",
            credential.to_str().unwrap(),
        ];
        if format == "7z" {
            args.extend(["--name", "private"]);
        }
        let failed = run(&archive, &args, None);
        assert!(!failed.status.success());
        assert_eq!(std::fs::read(archive).unwrap(), original);
    }
}

#[test]
fn timestamp_new_output_keeps_original_bytes() {
    let root = tempdir();
    let archive = fixture(root.path(), "zip");
    let original = std::fs::read(&archive).unwrap();
    let output = root.path().join("edited.zip");
    success(run(
        &archive,
        &[
            "--modified-unix-seconds",
            "1700000123",
            "--output",
            output.to_str().unwrap(),
        ],
        None,
    ));
    assert_eq!(std::fs::read(archive).unwrap(), original);
    let mut edited =
        Archive::open(std::fs::File::open(output).unwrap(), Limits::default()).unwrap();
    let ids: Vec<_> = edited.entries().iter().map(|entry| entry.id).collect();
    for id in ids {
        assert_eq!(
            edited.entry_metadata(id).unwrap().modified,
            Some(StoredTimestamp::UnixSeconds(1700000123))
        );
    }
}

#[test]
fn directory_only_and_empty_sevenz_archives_support_whole_header_encryption_and_decryption() {
    let root = tempdir();
    let credential = password(root.path(), "password", b"header-only-password\n");
    for directory_only in [true, false] {
        let archive = root.path().join(if directory_only {
            "directory.7z"
        } else {
            "empty.7z"
        });
        let entries = if directory_only {
            vec![archive_core::CreateEntry {
                name: "hidden-directory/".into(),
                data: Vec::new(),
                kind: archive_core::EntryKind::Directory,
            }]
        } else {
            Vec::new()
        };
        let mut bytes = std::io::Cursor::new(Vec::new());
        archive_core::create_with_options(
            archive_core::Format::SevenZip,
            &entries,
            &mut bytes,
            Limits::default(),
            archive_core::CreateOptions::default(),
        )
        .unwrap();
        std::fs::write(&archive, bytes.into_inner()).unwrap();
        let report = success(run(
            &archive,
            &[
                "--encrypt",
                "--encrypt-headers",
                "--new-password-file",
                credential.to_str().unwrap(),
            ],
            None,
        ));
        assert_eq!(report["details"]["headers_encrypted"], true);
        let listed = Command::new(env!("CARGO_BIN_EXE_arc"))
            .args(["--json", "list"])
            .arg(&archive)
            .output()
            .unwrap();
        assert!(!listed.status.success());
        let report = success(run(&archive, &["--decrypt"], Some(&credential)));
        assert_eq!(report["details"]["headers_encrypted"], false);
        success(
            Command::new(env!("CARGO_BIN_EXE_arc"))
                .args(["--json", "list"])
                .arg(&archive)
                .output()
                .unwrap(),
        );
    }
}
