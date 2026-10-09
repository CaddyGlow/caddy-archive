use std::process::Command;

fn fixture(root: &std::path::Path) -> std::path::PathBuf {
    let input = root.join("input");
    std::fs::create_dir_all(input.join("dir")).unwrap();
    std::fs::create_dir_all(input.join("directory")).unwrap();
    std::fs::write(input.join("dir/keep.txt"), b"keep").unwrap();
    std::fs::write(input.join("dir/drop.bin"), b"drop").unwrap();
    std::fs::write(input.join("directory/other.txt"), b"other").unwrap();
    let archive = root.join("source.zip");
    let output = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["create", "-i"])
        .arg(input)
        .arg("-o")
        .arg(&archive)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    archive
}

#[test]
fn shared_selector_controls_list_test_and_extract_with_directory_boundaries() {
    let root = tempfile::tempdir().unwrap();
    let archive = fixture(root.path());
    for operation in ["list", "test", "extract"] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_arc"));
        command
            .args([
                "--json",
                "--include",
                "dir",
                "--exclude",
                "dir/*.bin",
                operation,
            ])
            .arg(&archive);
        if operation == "extract" {
            command.arg("-o").arg(root.path().join("out"));
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{operation} failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        if operation == "list" {
            let names: Vec<_> = report["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| entry["name"].as_str().unwrap())
                .collect();
            assert!(names.contains(&"dir/keep.txt"));
            assert!(!names.contains(&"dir/drop.bin"));
            assert!(!names.iter().any(|name| name.starts_with("directory")));
        }
        if operation == "test" {
            assert_eq!(report["scope"], "selected-entries");
        }
    }
    assert_eq!(
        std::fs::read(root.path().join("out/dir/keep.txt")).unwrap(),
        b"keep"
    );
    assert!(!root.path().join("out/dir/drop.bin").exists());
    assert!(!root.path().join("out/directory").exists());
}

#[test]
fn literal_patterns_and_ascii_case_policy_are_explicit() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("literal.zip");
    archive_core::create(
        archive_core::Format::Zip,
        &[archive_core::CreateEntry {
            name: "literal*.txt".into(),
            data: b"literal".to_vec(),
            kind: archive_core::EntryKind::File,
        }],
        &mut std::fs::File::create(&archive).unwrap(),
        archive_core::Limits::default(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args([
            "--json",
            "--literal-names",
            "--ignore-ascii-case",
            "--include",
            "LITERAL*.TXT",
            "list",
        ])
        .arg(archive)
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["entries"].as_array().unwrap().len(), 1);
    assert_eq!(report["entries"][0]["name"], "literal*.txt");
}

#[test]
fn unsupported_patterns_fail_without_creating_a_destination() {
    let root = tempfile::tempdir().unwrap();
    let archive = fixture(root.path());
    let destination = root.path().join("out");
    let output = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--include", "**", "extract"])
        .arg(archive)
        .arg("-o")
        .arg(&destination)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!destination.exists());
}

#[test]
fn sevenz_unicode_name_selection_uses_decoded_names_for_all_operations() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("source.7z");
    archive_core::create(
        archive_core::Format::SevenZip,
        &[
            archive_core::CreateEntry {
                name: "dír/keep.txt".into(),
                data: b"keep".to_vec(),
                kind: archive_core::EntryKind::File,
            },
            archive_core::CreateEntry {
                name: "other.txt".into(),
                data: b"other".to_vec(),
                kind: archive_core::EntryKind::File,
            },
        ],
        &mut std::fs::File::create(&archive).unwrap(),
        archive_core::Limits::default(),
    )
    .unwrap();
    for operation in ["list", "test", "extract"] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_arc"));
        command
            .args(["--json", "--include", "dír", operation])
            .arg(&archive);
        if operation == "extract" {
            command.arg("-o").arg(root.path().join("out"));
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        if operation == "list" {
            assert_eq!(report["entries"].as_array().unwrap().len(), 1);
            assert_eq!(report["entries"][0]["name"], "dír/keep.txt");
        }
        if operation == "test" {
            assert_eq!(report["scope"], "selected-entries");
        }
    }
    assert_eq!(
        std::fs::read(root.path().join("out/dír/keep.txt")).unwrap(),
        b"keep"
    );
    assert!(!root.path().join("out/other.txt").exists());
}

#[test]
fn explicit_format_and_optical_view_cannot_silently_ignore_selection() {
    let root = tempfile::tempdir().unwrap();
    let archive = fixture(root.path());
    for operation in ["list", "test", "extract"] {
        for options in [
            ["--format", "zip", "--view", "udf"],
            ["--format", "iso", "--view", "iso"],
        ] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_arc"));
            command
                .args(["--json", "--include", "absent", operation])
                .arg(&archive)
                .args(options);
            if operation == "extract" {
                command.arg("-o").arg(root.path().join("out"));
            }
            let output = command.output().unwrap();
            assert_eq!(
                output.status.code(),
                Some(3),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            assert!(!root.path().join("out").exists());
        }
    }
}

#[test]
fn selected_test_does_not_claim_or_require_excluded_payload_integrity() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("corrupt.zip");
    let entries = [
        archive_core::CreateEntry {
            name: "keep".into(),
            data: b"keep".to_vec(),
            kind: archive_core::EntryKind::File,
        },
        archive_core::CreateEntry {
            name: "excluded".into(),
            data: b"EXCLUDED_PAYLOAD_SENTINEL_7249".to_vec(),
            kind: archive_core::EntryKind::File,
        },
    ];
    let mut cursor = std::io::Cursor::new(Vec::new());
    archive_core::create_with_options(
        archive_core::Format::Zip,
        &entries,
        &mut cursor,
        archive_core::Limits::default(),
        archive_core::CreateOptions {
            zip_compression: archive_core::ZipCompression::Copy,
            ..Default::default()
        },
    )
    .unwrap();
    let mut bytes = cursor.into_inner();
    let sentinel = b"EXCLUDED_PAYLOAD_SENTINEL_7249";
    let offset = bytes
        .windows(sentinel.len())
        .position(|window| window == sentinel)
        .unwrap();
    bytes[offset] ^= 1;
    std::fs::write(&archive, bytes).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "--include", "keep", "test"])
        .arg(&archive)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["scope"], "selected-entries");
    assert_eq!(report["verified"], true);
    assert_eq!(report["entries"], 1);
    assert_eq!(report["bytes"], 4);
    let output = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "test"])
        .arg(&archive)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
}

#[test]
fn unfiltered_listing_does_not_apply_new_selection_name_limits() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("long-name.zip");
    let long_name = "a".repeat(17_000);
    archive_core::create(
        archive_core::Format::Zip,
        &[archive_core::CreateEntry {
            name: long_name.clone(),
            data: Vec::new(),
            kind: archive_core::EntryKind::File,
        }],
        &mut std::fs::File::create(&archive).unwrap(),
        archive_core::Limits::default(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "list"])
        .arg(&archive)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["entries"][0]["name"], long_name);
    let output = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "--include", "a*", "list"])
        .arg(&archive)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(5));
}
