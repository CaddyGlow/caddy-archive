#![cfg(target_os = "linux")]

use std::{os::unix::process::CommandExt, process::Command};

#[test]
fn cabinet_creation_streams_payload_larger_than_process_address_limit() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    std::fs::create_dir(&input).unwrap();
    std::fs::File::create(input.join("large"))
        .unwrap()
        .set_len(128 << 20)
        .unwrap();
    let output = root.path().join("output.cab");
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
    // Constrain only the CLI child. The old whole-payload creation path cannot
    // fit this 128 MiB source inside a 96 MiB process address space.
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
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["verified"], true);
    assert_eq!(report["bytes"], 128u64 << 20);
    assert_eq!(report["entries"], 1);
}
