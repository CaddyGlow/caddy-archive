use std::path::Path;
use std::process::Command;

fn fixture(path: &Path) {
    let entries = [
        ("dir/keep.txt", b"keep".as_slice()),
        ("dir/drop.bin", b"drop".as_slice()),
        ("directory/other.txt", b"other".as_slice()),
        ("literal*.txt", b"literal".as_slice()),
        ("@entry", b"marker".as_slice()),
        ("--json", b"dash".as_slice()),
    ]
    .into_iter()
    .map(|(name, data)| archive_core::CreateEntry {
        name: name.into(),
        data: data.to_vec(),
        kind: archive_core::EntryKind::File,
    })
    .collect::<Vec<_>>();
    archive_core::create(
        archive_core::Format::Zip,
        &entries,
        &mut std::fs::File::create(path).unwrap(),
        archive_core::Limits::default(),
    )
    .unwrap();
}

#[test]
fn read_frontend_uses_shared_selection_and_native_publication() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("source.zip");
    fixture(&archive);
    for operation in ["l", "t", "x"] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_arc"));
        command
            .args(["--json", "7z", operation])
            .arg(&archive)
            .args(["-tzip", "-i!dir", "-x!dir/*.bin"]);
        if operation == "x" {
            command.arg(format!("-o{}", root.path().join("out").display()));
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["ok"], true);
        if operation == "l" {
            let entries = report["entries"].as_array().unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0]["name"], "dir/keep.txt");
        }
        if operation == "t" {
            assert_eq!(report["scope"], "selected-entries");
        }
    }
    assert_eq!(
        std::fs::read(root.path().join("out/dir/keep.txt")).unwrap(),
        b"keep"
    );
    assert!(!root.path().join("out/dir/drop.bin").exists());
    assert!(!root.path().join("out/directory").exists());

    // Frontend lowering retains no-overwrite behavior and original output bytes.
    let output = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["7z", "x"])
        .arg(&archive)
        .arg("-i!dir/keep.txt")
        .arg(format!("-o{}", root.path().join("out").display()))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        std::fs::read(root.path().join("out/dir/keep.txt")).unwrap(),
        b"keep"
    );
}

#[test]
fn literal_ascii_and_terminated_marker_names_are_supported_explicitly() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("source.zip");
    fixture(&archive);
    let output = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["7z", "l", "--json", "-spd", "-ssc-"])
        .arg(&archive)
        .arg("LITERAL*.TXT")
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["entries"][0]["name"], "literal*.txt");
    assert_eq!(report["entries"].as_array().unwrap().len(), 1);

    let output = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["7z", "l", "-j", "-i!@entry", "--"])
        .arg(&archive)
        .arg("--json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let names = report["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, ["@entry", "--json"]);
}

#[test]
fn frontend_errors_never_echo_password_or_unsupported_values() {
    const SECRET: &str = "FRONTEND_SECRET_SENTINEL_94271";
    for prefix in [vec!["7z"], vec!["--json", "7z"], vec!["-j", "7z"]] {
        for option in [
            format!("-p{SECRET}"),
            format!("-m{SECRET}"),
            format!("--unknown={SECRET}"),
        ] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_arc"));
            command
                .args(&prefix)
                .args(["l", "missing-archive"])
                .arg(&option);
            let output = command.output().unwrap();
            assert_eq!(output.status.code(), Some(3));
            assert!(!String::from_utf8_lossy(&output.stdout).contains(SECRET));
            assert!(!String::from_utf8_lossy(&output.stderr).contains(SECRET));
            if prefix.len() == 2 {
                let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                assert_eq!(report["ok"], false);
                assert!(output.stderr.is_empty());
            }
        }
    }
    let output = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["7z", "l", "missing-archive", "--json"])
        .arg(format!("-p{SECRET}"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert!(!String::from_utf8_lossy(&output.stdout).contains(SECRET));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(SECRET));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ok"], false);
}

#[test]
fn editing_unknown_profiles_and_grammar_fail_before_output() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("source.zip");
    fixture(&archive);
    let original = std::fs::read(&archive).unwrap();
    for operation in ["a", "u", "d", "rn", "e"] {
        let output = Command::new(env!("CARGO_BIN_EXE_arc"))
            .args(["7z", operation])
            .arg(&archive)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(3));
        assert_eq!(std::fs::read(&archive).unwrap(), original);
    }
    for option in ["-tbr", "-i!**", "-i![ab]", "-ir!dir", "-x@list"] {
        let destination = root.path().join("out");
        let output = Command::new(env!("CARGO_BIN_EXE_arc"))
            .args(["7z", "x"])
            .arg(&archive)
            .arg(option)
            .arg(format!("-o{}", destination.display()))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(3));
        assert!(!destination.exists());
    }
}

// Linux filesystems support arbitrary non-NUL filename bytes; macOS rejects this
// fixture with EILSEQ. Parser-only coverage remains enabled on every Unix target.
#[cfg(target_os = "linux")]
#[test]
fn non_utf8_dash_archive_paths_survive_switch_termination() {
    use std::os::unix::ffi::OsStringExt;
    let root = tempfile::tempdir().unwrap();
    let name = std::ffi::OsString::from_vec(vec![b'-', 0xff, b'.', b'z', b'i', b'p']);
    fixture(&root.path().join(&name));
    let output = Command::new(env!("CARGO_BIN_EXE_arc"))
        .current_dir(root.path())
        .args(["7z", "t", "--"])
        .arg(name)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn unsupported_native_prefixes_are_redacted_before_clap_validation() {
    const SENTINEL: &str = "PREFIX_SECRET_SENTINEL_92013";
    for args in [
        vec![
            "--json".into(),
            format!("-mmemuse={SENTINEL}"),
            "7z".into(),
            "l".into(),
            "missing".into(),
            format!("-p{SENTINEL}"),
        ],
        vec![
            "--json".into(),
            "--progress".into(),
            SENTINEL.into(),
            "7z".into(),
            "l".into(),
            "missing".into(),
            format!("-p{SENTINEL}"),
        ],
        vec![
            "--json".into(),
            "--view".into(),
            SENTINEL.into(),
            "7z".into(),
            "l".into(),
            "missing".into(),
            format!("-p{SENTINEL}"),
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_arc"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(3));
        assert!(!String::from_utf8_lossy(&output.stdout).contains(SENTINEL));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(SENTINEL));
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["ok"], false);
    }
}
