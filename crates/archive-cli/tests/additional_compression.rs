use std::process::Command;

#[test]
fn bzip2_and_brotli_profiles_create_list_test_and_extract() {
    for format in ["bz2", "br", "tar.bz2", "tar.br"] {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input");
        std::fs::create_dir(&input).unwrap();
        std::fs::write(input.join("payload"), b"portable compression payload").unwrap();
        let archive = root.path().join(format!("archive.{format}"));
        let result = Command::new(env!("CARGO_BIN_EXE_arc"))
            .args(["--json", "create", "--format", format, "--input"])
            .arg(&input)
            .arg("--output")
            .arg(&archive)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        for operation in ["list", "test", "extract"] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_arc"));
            command
                .args(["--json", operation])
                .arg(&archive)
                .args(["--format", format]);
            let output = root.path().join("out");
            if operation == "extract" {
                command.arg("--output").arg(&output);
            }
            let result = command.output().unwrap();
            assert!(
                result.status.success(),
                "{format} {operation}: {}",
                String::from_utf8_lossy(&result.stdout)
            );
            if operation == "extract" {
                let files = std::fs::read_dir(&output)
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                assert_eq!(files.len(), 1);
                assert_eq!(
                    std::fs::read(files[0].path()).unwrap(),
                    b"portable compression payload"
                );
            }
        }
    }
}

#[test]
fn sevenz_selectable_compression_round_trips() {
    for codec in ["copy", "deflate", "lzma", "lzma2", "bzip2", "brotli"] {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input");
        std::fs::create_dir(&input).unwrap();
        std::fs::write(input.join("payload"), b"selected 7z codec payload").unwrap();
        let archive = root.path().join("archive.7z");
        let result = Command::new(env!("CARGO_BIN_EXE_arc"))
            .args([
                "--json",
                "create",
                "--format",
                "7z",
                "--compression",
                codec,
                "--input",
            ])
            .arg(&input)
            .arg("--output")
            .arg(&archive)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{codec}: {}",
            String::from_utf8_lossy(&result.stdout)
        );
        for operation in ["list", "test", "extract"] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_arc"));
            command.args(["--json", operation]).arg(&archive);
            let output = root.path().join("out");
            if operation == "extract" {
                command.arg("--output").arg(&output);
            }
            let result = command.output().unwrap();
            assert!(
                result.status.success(),
                "{codec} {operation}: {}",
                String::from_utf8_lossy(&result.stdout)
            );
            if operation == "list" {
                let listed: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
                assert_eq!(
                    listed["entries"][0]["compression"]
                        .as_str()
                        .unwrap()
                        .to_lowercase(),
                    codec,
                    "requested codec must be stored in the archive"
                );
            }
            if operation == "extract" {
                assert_eq!(
                    std::fs::read(output.join("payload")).unwrap(),
                    b"selected 7z codec payload"
                );
            }
        }
    }
}

#[test]
fn sevenz_compression_option_is_not_silently_ignored_for_zip() {
    let root = tempfile::tempdir().unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args([
            "--json",
            "create",
            "--format",
            "zip",
            "--compression",
            "bzip2",
            "--input",
        ])
        .arg(root.path())
        .arg("--output")
        .arg(root.path().join("archive.zip"))
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(3));
    assert!(!root.path().join("archive.zip").exists());
}
