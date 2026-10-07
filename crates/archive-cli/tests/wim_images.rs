use std::process::Command;

#[test]
fn wim_numeric_and_named_selectors_conflict() {
    let result = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args([
            "--json",
            "--image",
            "1",
            "--image-name",
            "Example",
            "list",
            "archive.wim",
        ])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
}

#[test]
fn wim_unselected_listing_reports_container_images() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../archive-core/tests/fixtures/wimlib-lzms-solid.esd");
    let result = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "list"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let images = json["images"].as_array().unwrap();
    assert!(!images.is_empty());
    assert_eq!(images[0]["index"], 1);
    {
        let name = images[0]["name"]
            .as_str()
            .expect("fixture image has XML name");
        let result = Command::new(env!("CARGO_BIN_EXE_arc"))
            .args(["--json", "--image-name", name, "list"])
            .arg(source)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        let selected: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(selected["image"], 1);
        assert_eq!(selected["entries"].as_array().unwrap().len(), 3);
    }
}
