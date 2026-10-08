use std::process::Command;

#[test]
fn missing_later_media_does_not_publish_earlier_payload() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("installer.msi");
    let output = root.path().join("out");
    std::fs::write(&input, include_bytes!("fixtures/missing-second.msi")).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "extract"])
        .arg(input)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("missing.cab"));
    assert_eq!(std::fs::read_dir(output).unwrap().count(), 0);
}

#[test]
fn explicit_external_media_mapping_extracts_without_automatic_search() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("external.msi");
    let cabinet = root.path().join("data.cab");
    let output = root.path().join("out");
    std::fs::write(&input, include_bytes!("fixtures/external.msi")).unwrap();
    std::fs::write(&cabinet, include_bytes!("fixtures/data.cab")).unwrap();
    let missing = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "test"])
        .arg(&input)
        .output()
        .unwrap();
    assert!(!missing.status.success());
    let mapping = format!("data.cab={}", cabinet.display());
    let bounded = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args([
            "--json",
            "--max-input-bytes",
            "1",
            "--media",
            &mapping,
            "test",
        ])
        .arg(&input)
        .output()
        .unwrap();
    assert_eq!(bounded.status.code(), Some(5));
    let result = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "--media", &mapping, "extract"])
        .arg(input)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert_eq!(
        std::fs::read(output.join("payload.txt")).unwrap(),
        b"portable package payload\n"
    );
    assert_eq!(
        std::fs::read(cabinet).unwrap(),
        include_bytes!("fixtures/data.cab")
    );
}

#[test]
fn listing_msi_enforces_compound_input_budget() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("external.msi");
    std::fs::write(&input, include_bytes!("fixtures/external.msi")).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "--max-input-bytes", "1", "list"])
        .arg(input)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(5));
    let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(json["ok"], false);
}
