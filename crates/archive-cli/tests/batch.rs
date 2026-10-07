#[test]
fn later_integrity_failure_does_not_publish_earlier_batch_files() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("bad.zip");
    let entries: Vec<_> = [("one", b"first".as_slice()), ("two", b"second".as_slice())]
        .into_iter()
        .map(|(name, data)| archive_core::CreateEntry {
            name: name.into(),
            data: data.to_vec(),
            kind: archive_core::EntryKind::File,
        })
        .collect();
    let mut bytes = std::io::Cursor::new(Vec::new());
    archive_core::create(
        archive_core::Format::Zip,
        &entries,
        &mut bytes,
        archive_core::Limits::default(),
    )
    .unwrap();
    let mut bytes = bytes.into_inner();
    let second = bytes
        .windows(4)
        .enumerate()
        .filter_map(|(index, signature)| (signature == b"PK\x01\x02").then_some(index))
        .nth(1)
        .unwrap();
    bytes[second + 16] ^= 1;
    std::fs::write(&archive, bytes).unwrap();
    let output = root.path().join("output");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "extract"])
        .arg(&archive)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!output.join("one").exists());
    assert!(!output.join("two").exists());
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn many_files_extract_with_a_small_descriptor_limit() {
    use std::os::unix::process::CommandExt;
    let root = tempfile::tempdir().unwrap();
    let entries: Vec<_> = (0..100)
        .map(|id| archive_core::CreateEntry {
            name: format!("nested/{id}"),
            data: format!("payload {id}").into_bytes(),
            kind: archive_core::EntryKind::File,
        })
        .collect();
    for format in [archive_core::Format::Zip, archive_core::Format::Tar] {
        let path = root.path().join(format!("{format:?}.archive"));
        archive_core::create(
            format,
            &entries,
            &mut std::fs::File::create(&path).unwrap(),
            archive_core::Limits::default(),
        )
        .unwrap();
        let output = root.path().join(format!("{format:?}-output"));
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_arc"));
        command.args(["--json", "extract"]);
        if format == archive_core::Format::Tar {
            command.args(["-", "--format", "tar"]);
            command.stdin(std::fs::File::open(&path).unwrap());
        } else {
            command.arg(&path);
        }
        command.arg("--output").arg(&output);
        // This runs in the child only, so parallel tests keep their descriptor limits.
        unsafe {
            command.pre_exec(|| {
                let limit = libc::rlimit {
                    rlim_cur: 64,
                    rlim_max: 64,
                };
                if libc::setrlimit(libc::RLIMIT_NOFILE, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let result = command.output().unwrap();
        assert!(
            result.status.success(),
            "{format:?}: {}",
            String::from_utf8_lossy(&result.stdout)
        );
        for entry in &entries {
            assert_eq!(std::fs::read(output.join(&entry.name)).unwrap(), entry.data);
        }
    }
}
