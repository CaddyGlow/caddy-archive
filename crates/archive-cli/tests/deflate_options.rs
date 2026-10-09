use std::process::Command;

#[test]
fn native_deflate_level_changes_encoding_and_reports_effective_settings() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    let payload = b"configurable bounded compression\n".repeat(4096);
    std::fs::write(&input, &payload).unwrap();
    let mut sizes = Vec::new();
    for level in [0, 9] {
        let packed = root.path().join(format!("level-{level}.gz"));
        let result = Command::new(env!("CARGO_BIN_EXE_arc"))
            .args(["--json", "deflate", "--input"])
            .arg(&input)
            .arg("--output")
            .arg(&packed)
            .arg("--compression-level")
            .arg(level.to_string())
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(json["effective_settings"]["level"], level);
        sizes.push(std::fs::metadata(&packed).unwrap().len());
        let restored = root.path().join(format!("restored-{level}"));
        let result = Command::new(env!("CARGO_BIN_EXE_arc"))
            .args(["inflate", "--input"])
            .arg(&packed)
            .arg("--output")
            .arg(&restored)
            .output()
            .unwrap();
        assert!(result.status.success());
        assert_eq!(std::fs::read(restored).unwrap(), payload);
    }
    assert!(sizes[0] > sizes[1] * 10);
    let output = root.path().join("invalid.gz");
    let result = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["deflate", "--input"])
        .arg(input)
        .arg("--output")
        .arg(&output)
        .args(["--compression-level", "10"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert!(!output.exists());
}
