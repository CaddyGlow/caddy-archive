use std::process::Command;

#[test]
fn tar_style_create_list_extract_and_verbose_keep_json_separate() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("payload"), b"tar-style flags").unwrap();
    for (create, list, extract, name) in [
        ("cvf", "tf", "xvf", "plain.tar"),
        ("czvf", "tzf", "xzvf", "gzip.tar.gz"),
        ("cJvf", "tJf", "xJvf", "xz.tar.xz"),
        ("cjvf", "tjf", "xjvf", "bzip.tar.bz2"),
        ("cavf", "tf", "xvf", "auto.tar.gz"),
    ] {
        let archive = root.path().join(name);
        let result = Command::new(env!("CARGO_BIN_EXE_arc"))
            .args(["-j", create])
            .arg(&archive)
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        assert!(String::from_utf8_lossy(&result.stderr).contains("payload"));
        let result = Command::new(env!("CARGO_BIN_EXE_arc"))
            .args(["-j", list])
            .arg(&archive)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        let listed: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(listed["entries"][0]["name"], "payload");
        let output = root.path().join(format!("out-{name}"));
        let result = Command::new(env!("CARGO_BIN_EXE_arc"))
            .arg(format!("-{extract}"))
            .arg(&archive)
            .arg("-C")
            .arg(&output)
            .arg("-j")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        let extracted: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(extracted["ok"], true);
        assert!(String::from_utf8_lossy(&result.stderr).contains("payload"));
        assert_eq!(
            std::fs::read(output.join("payload")).unwrap(),
            b"tar-style flags"
        );
    }
}

#[test]
fn invalid_tar_style_words_fail_before_creating_output() {
    let root = tempfile::tempdir().unwrap();
    for word in ["cxvf", "xv", "xzJf", "cf"] {
        let result = Command::new(env!("CARGO_BIN_EXE_arc"))
            .current_dir(root.path())
            .args(["-j", word])
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(2));
        let error: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(error["ok"], false);
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
