use std::process::Command;

#[test]
fn oversized_password_source_is_not_echoed_or_published() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    std::fs::create_dir(&input).unwrap();
    std::fs::write(input.join("payload"), b"payload").unwrap();
    let password = root.path().join("password");
    std::fs::write(&password, b"secret-canary-".repeat(2000)).unwrap();
    let archive = root.path().join("archive.zip");
    let result = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "--password-file"])
        .arg(password)
        .args(["create", "--format", "zip", "--encrypt", "--input"])
        .arg(input)
        .arg("--output")
        .arg(&archive)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!archive.exists());
    assert!(!String::from_utf8_lossy(&result.stdout).contains("secret-canary"));
    assert!(!String::from_utf8_lossy(&result.stderr).contains("secret-canary"));
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
}

#[test]
fn encrypted_creation_and_failed_password_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    std::fs::create_dir(&input).unwrap();
    std::fs::write(input.join("payload"), b"secret payload").unwrap();
    let password = root.path().join("password");
    std::fs::write(&password, b"correct\n").unwrap();
    let wrong = root.path().join("wrong");
    std::fs::write(&wrong, b"incorrect\n").unwrap();
    let archive = root.path().join("encrypted.zip");
    let output = root.path().join("output");
    let binary = env!("CARGO_BIN_EXE_arc");
    let create = Command::new(binary)
        .args(["--json", "--password-file"])
        .arg(&password)
        .args(["create", "--encrypt", "--format", "zip", "--input"])
        .arg(&input)
        .arg("--output")
        .arg(&archive)
        .output()
        .unwrap();
    assert!(
        create.status.success(),
        "{}",
        String::from_utf8_lossy(&create.stdout)
    );
    let extract = Command::new(binary)
        .args(["--json", "--password-file"])
        .arg(&wrong)
        .arg("extract")
        .arg(&archive)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(!extract.status.success());
    assert!(!output.join("payload").exists());
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0);
    let extract = Command::new(binary)
        .args(["--json", "--password-file"])
        .arg(&password)
        .arg("extract")
        .arg(&archive)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        extract.status.success(),
        "{}",
        String::from_utf8_lossy(&extract.stdout)
    );
    assert_eq!(
        std::fs::read(output.join("payload")).unwrap(),
        b"secret payload"
    );
}
