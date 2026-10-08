use std::process::Command;

#[test]
fn memory_policy_spelling_and_small_budget_are_enforced() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    std::fs::create_dir(&input).unwrap();
    std::fs::write(input.join("payload"), b"memory policy").unwrap();
    for (index, option) in ["--memuse=0", "-mmemuse=64m", "--mmemuse=p80"]
        .into_iter()
        .enumerate()
    {
        let output = root.path().join(format!("output{index}.zip"));
        let result = Command::new(env!("CARGO_BIN_EXE_arc"))
            .args(["--json", option, "create", "--input"])
            .arg(&input)
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap();
        if index == 0 {
            assert_eq!(
                result.status.code(),
                Some(5),
                "{}",
                String::from_utf8_lossy(&result.stdout)
            );
            assert!(!output.exists());
        } else {
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stdout)
            );
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn sevenzip_creation_streams_payload_larger_than_process_address_limit() {
    use std::os::unix::process::CommandExt;
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    std::fs::create_dir(&input).unwrap();
    std::fs::File::create(input.join("large"))
        .unwrap()
        .set_len(128 << 20)
        .unwrap();
    let output = root.path().join("output.7z");
    let mut command = Command::new(env!("CARGO_BIN_EXE_arc"));
    command
        .args([
            "--json",
            "--memuse=32m",
            "create",
            "--compression",
            "copy",
            "--input",
        ])
        .arg(&input)
        .arg("--output")
        .arg(&output);
    unsafe {
        command.pre_exec(|| {
            let limit = libc::rlimit {
                rlim_cur: 96 << 20,
                rlim_max: 96 << 20,
            };
            if libc::setrlimit(libc::RLIMIT_AS, &limit) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let result = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "test"])
        .arg(output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
}
