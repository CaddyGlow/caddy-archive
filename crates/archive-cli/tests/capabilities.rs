use std::process::Command;

#[test]
fn capability_report_distinguishes_native_operations_and_planned_compatibility() {
    let output = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "capabilities"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["operation"], "capabilities");
    assert_eq!(report["ok"], true);
    assert_eq!(report["compatibility"]["reference_version"], "7-Zip 26.04");
    let rows = report["compatibility"]["rows"].as_array().unwrap();
    let add = rows.iter().find(|row| row["id"] == "command.add").unwrap();
    assert_eq!(add["native_status"], "implemented");
    assert_eq!(add["compatibility_status"], "planned");
    assert_eq!(report["packages"]["edit"], false);
    assert_eq!(report["packages"]["extract"], true);
    let wim = report["formats"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["format"] == "Wim")
        .unwrap();
    assert_eq!(wim["read"], true);
    assert_eq!(wim["write"], false);
}

#[test]
fn plain_capability_report_is_complete_parseable_json() {
    let output = Command::new(env!("CARGO_BIN_EXE_arc"))
        .arg("capabilities")
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(report["compatibility"]["rows"].as_array().unwrap().len() > 40);
}
