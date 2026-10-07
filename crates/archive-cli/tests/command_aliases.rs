use std::process::Command;

#[test]
fn aliases_and_archive_named_output_work_for_plain_and_compound_extensions() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    std::fs::create_dir(&input).unwrap();
    std::fs::write(input.join("payload"), b"named extraction").unwrap();
    let binary = env!("CARGO_BIN_EXE_arc");
    for (name, folder) in [("backup.v1.zip", "backup.v1"), ("backup.TAR.GZ", "backup")] {
        let archive = root.path().join(name);
        let result = Command::new(binary)
            .args(["-j", "a", "-i"])
            .arg(&input)
            .arg("-o")
            .arg(&archive)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        for alias in ["l", "t"] {
            let result = Command::new(binary)
                .args(["-j", alias])
                .arg(&archive)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stdout)
            );
        }
        for (id, args, expected) in [
            ("subfolder", vec!["-s"], folder),
            ("wildcard", vec!["-o*"], folder),
            ("unzip", vec!["-d", "chosen"], "chosen"),
            ("attached", vec!["-ochosen"], "chosen"),
            ("default", vec![], "."),
            ("parent", vec!["-o", "parent", "--archive-folder"], "parent"),
            ("wildcard-parent", vec!["-o", "parent/*"], "parent"),
        ] {
            let working = root.path().join(format!("{name}-{id}"));
            std::fs::create_dir(&working).unwrap();
            let result = Command::new(binary)
                .current_dir(&working)
                .args(["-j", "x"])
                .arg(&archive)
                .args(args)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{name} {id}: {}",
                String::from_utf8_lossy(&result.stdout)
            );
            let directory = if matches!(id, "parent" | "wildcard-parent") {
                working.join(expected).join(folder)
            } else {
                working.join(expected)
            };
            assert_eq!(
                std::fs::read(directory.join("payload")).unwrap(),
                b"named extraction"
            );
        }
    }
}

#[test]
fn stdin_cannot_supply_an_archive_folder_name() {
    let root = tempfile::tempdir().unwrap();
    for option in ["-s", "-o*"] {
        let result = Command::new(env!("CARGO_BIN_EXE_arc"))
            .current_dir(root.path())
            .args(["-j", "x", "-", option])
            .output()
            .unwrap();
        assert!(!result.status.success());
        let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert!(
            json["error"]["message"]
                .as_str()
                .unwrap()
                .contains("requires a filename")
        );
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
