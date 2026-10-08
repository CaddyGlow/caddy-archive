use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn tar_stdout_and_stdin_commands_round_trip() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    std::fs::create_dir(&input).unwrap();
    std::fs::write(input.join("file"), b"streamed").unwrap();
    let create = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["create", "--format", "tar", "--input"])
        .arg(&input)
        .args(["--output", "-"])
        .output()
        .unwrap();
    assert!(
        create.status.success(),
        "{}",
        String::from_utf8_lossy(&create.stderr)
    );
    assert_eq!(create.stdout.len() % 512, 0);
    for operation in ["list", "test", "extract"] {
        let output = root.path().join("output");
        let mut command = Command::new(env!("CARGO_BIN_EXE_arc"));
        command.args(["--json", operation, "-", "--format", "tar"]);
        if operation == "extract" {
            command.arg("--output").arg(&output);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&create.stdout)
            .unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(json["ok"], true);
        if operation == "extract" {
            assert_eq!(std::fs::read(output.join("file")).unwrap(), b"streamed");
        }
    }
}

#[test]
fn json_argument_errors_remain_machine_readable() {
    let result = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "extract"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(json["error"]["code"], 2);
    assert!(result.stderr.is_empty());
}

#[test]
fn inflate_and_deflate_forward_stdin_stdout_round_trip() {
    for format in ["deflate", "gzip", "zlib"] {
        let mut compressed = Vec::new();
        for operation in ["deflate", "inflate"] {
            let input = if operation == "deflate" {
                b"forward codec payload".as_slice()
            } else {
                compressed.as_slice()
            };
            let mut child = Command::new(env!("CARGO_BIN_EXE_arc"))
                .args([
                    operation, "--format", format, "--input", "-", "--output", "-",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(input).unwrap();
            let result = child.wait_with_output().unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            if operation == "deflate" {
                compressed = result.stdout;
            } else {
                assert_eq!(result.stdout, b"forward codec payload");
            }
        }
    }
}

#[test]
fn failed_inflate_does_not_publish_file() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("broken.gz");
    let output = root.path().join("output");
    std::fs::write(&source, b"broken gzip").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "inflate", "--format", "gzip", "--input"])
        .arg(source)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn sigint_during_partial_tar_header_exits_instead_of_retrying() {
    use std::io::{BufRead, BufReader};
    use std::time::{Duration, Instant};
    let mut header = [0u8; 512];
    header[..5].copy_from_slice(b"ready");
    for range in [100..108, 108..116, 116..124, 124..136, 136..148] {
        header[range.clone()].fill(b'0');
        header[range.end - 1] = 0;
    }
    header[148..156].fill(b' ');
    header[156] = b'0';
    let checksum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
    header[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
    let mut child = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["list", "--format", "tar", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(&header).unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert!(line.contains("ready"));
    input.write_all(b"x").unwrap();
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGINT) }, 0);
    let _ = input.write_all(b"y");
    drop(input);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(status.code(), Some(130));
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("SIGINT left the TAR reader retrying after EOF");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
#[test]
fn interrupt_during_file_creation_does_not_publish_output() {
    use std::time::{Duration, Instant};
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    std::fs::create_dir(&input).unwrap();
    // Sparse input keeps fixture construction cheap while making encoding long enough
    // to interrupt after the provisional file proves the handler is installed.
    std::fs::File::create(input.join("large"))
        .unwrap()
        .set_len(2 << 30)
        .unwrap();
    let output = root.path().join("output.tar.gz");
    let mut child = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "create", "--input"])
        .arg(input)
        .arg("--output")
        .arg(&output)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if std::fs::read_dir(root.path()).unwrap().count() > 1 {
            break;
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "creation exited before staging"
        );
        if Instant::now() > deadline {
            child.kill().unwrap();
            panic!("creation did not stage output");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) };
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            child.kill().unwrap();
            panic!("creation ignored cancellation");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let result = child.wait_with_output().unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("cancelled"));
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}
