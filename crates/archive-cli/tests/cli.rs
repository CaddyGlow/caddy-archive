use std::process::Command;

#[test]
fn json_create_list_test_extract_round_trip() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    std::fs::create_dir(&input).unwrap();
    std::fs::write(input.join("hello.txt"), b"hello").unwrap();
    let archive = root.path().join("test.zip");
    let output = root.path().join("output");
    let binary = env!("CARGO_BIN_EXE_arc");
    let create = Command::new(binary)
        .args(["-j", "create", "-f", "zip", "-i"])
        .arg(&input)
        .arg("-o")
        .arg(&archive)
        .output()
        .unwrap();
    assert!(
        create.status.success(),
        "{}",
        String::from_utf8_lossy(&create.stderr)
    );
    for operation in ["list", "test"] {
        let result = Command::new(binary)
            .args(["-j", operation, "-f", "zip"])
            .arg(&archive)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["ok"], true);
        assert!(result.stderr.is_empty());
    }
    let extract = Command::new(binary)
        .args(["-j", "extract", "-t", "1"])
        .arg(&archive)
        .arg("-o")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        extract.status.success(),
        "{}",
        String::from_utf8_lossy(&extract.stderr)
    );
    assert_eq!(std::fs::read(output.join("hello.txt")).unwrap(), b"hello");
}

#[test]
fn traversal_is_rejected_before_any_publication() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("bad.zip");
    let mut file = std::fs::File::create(&archive).unwrap();
    archive_core::create(
        archive_core::Format::Zip,
        &[archive_core::CreateEntry {
            name: "../escaped".into(),
            data: b"bad".to_vec(),
            kind: archive_core::EntryKind::File,
        }],
        &mut file,
        archive_core::Limits::default(),
    )
    .unwrap();
    let output = root.path().join("output");
    let extract = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "extract"])
        .arg(&archive)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(!extract.status.success());
    assert!(!root.path().join("escaped").exists());
    assert_eq!(std::fs::read_dir(output).unwrap().count(), 0);
}

#[cfg(not(feature = "progress"))]
#[test]
fn always_progress_without_feature_has_clear_error() {
    let result = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "--progress", "always", "list", "absent.zip"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("progress rendering unavailable")
    );
}

#[test]
fn decoded_budget_rejects_bomb_without_publication() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("bomb.zip");
    archive_core::create(
        archive_core::Format::Zip,
        &[archive_core::CreateEntry {
            name: "zeros".into(),
            data: vec![0; 128 * 1024],
            kind: archive_core::EntryKind::File,
        }],
        &mut std::fs::File::create(&path).unwrap(),
        archive_core::Limits::default(),
    )
    .unwrap();
    let destination = root.path().join("out");
    let result = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--max-total-bytes", "1024", "extract"])
        .arg(path)
        .arg("--output")
        .arg(&destination)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!destination.join("zeros").exists());
    assert!(String::from_utf8_lossy(&result.stderr).contains("limit"));
}
