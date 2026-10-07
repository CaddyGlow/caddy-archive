use std::process::Command;

fn run(
    arguments: &[&str],
    archive: &std::path::Path,
    output: Option<&std::path::Path>,
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_arc"));
    command.arg("--json").args(arguments).arg(archive);
    if let Some(output) = output {
        command.arg("--output").arg(output);
    }
    command.output().unwrap()
}

#[test]
fn bundle_listing_and_exact_selection_extract_expected_payload() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("valid.msixbundle");
    std::fs::write(&archive, include_bytes!("fixtures/valid.msixbundle")).unwrap();
    let listed = run(&["list"], &archive, None);
    assert!(listed.status.success());
    let json: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(json["packages"].as_array().unwrap().len(), 2);
    for operation in ["test", "extract"] {
        let output = root.path().join("out");
        let result = run(
            &["--bundle-entry", "app-x64.msix", operation],
            &archive,
            if operation == "extract" {
                Some(&output)
            } else {
                None
            },
        );
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        if operation == "extract" {
            assert_eq!(
                std::fs::read(output.join("payload.txt")).unwrap(),
                b"portable package payload\n"
            );
        }
    }
}

#[test]
fn absent_unknown_and_corrupt_bundle_selection_publish_nothing() {
    for (fixture, selector) in [
        (include_bytes!("fixtures/valid.msixbundle").as_slice(), None),
        (
            include_bytes!("fixtures/valid.msixbundle").as_slice(),
            Some("missing.msix"),
        ),
        (
            include_bytes!("fixtures/corrupt.msixbundle").as_slice(),
            Some("app-x64.msix"),
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("archive.msixbundle");
        let output = root.path().join("out");
        std::fs::write(&archive, fixture).unwrap();
        let mut args = Vec::new();
        if let Some(selector) = selector {
            args.extend(["--bundle-entry", selector]);
        }
        args.push("extract");
        let result = run(&args, &archive, Some(&output));
        assert!(!result.status.success());
        assert!(!output.exists());
    }
}

#[test]
fn corrupt_outer_bundle_block_hash_prevents_selected_extraction() {
    let limits = archive_core::Limits::default();
    let mut source = archive_core::Archive::open(
        std::io::Cursor::new(include_bytes!("fixtures/valid.msixbundle")),
        limits,
    )
    .unwrap();
    let mut entries = Vec::new();
    for entry in source.entries().to_vec() {
        let mut data = source.read_entry(entry.id, limits.max_entry_bytes).unwrap();
        if entry.name == "AppxMetadata/AppxBundleManifest.xml" {
            let index = data
                .windows(b"Browser.Test".len())
                .position(|bytes| bytes == b"Browser.Test")
                .unwrap();
            data[index + 8] = b'B';
        }
        entries.push(archive_core::CreateEntry {
            name: entry.name,
            data,
            kind: entry.kind,
        });
    }
    let mut archive = std::io::Cursor::new(Vec::new());
    archive_core::create(archive_core::Format::Zip, &entries, &mut archive, limits).unwrap();
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("outer-corrupt.msixbundle");
    let output = root.path().join("out");
    std::fs::write(&input, archive.into_inner()).unwrap();
    let result = run(
        &["--bundle-entry", "app-x64.msix", "extract"],
        &input,
        Some(&output),
    );
    assert_eq!(result.status.code(), Some(4));
    assert!(!output.exists());
}
