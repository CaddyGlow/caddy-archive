#[cfg(unix)]
#[test]
fn creation_and_extraction_preserve_supported_times_and_permissions() {
    use std::{
        os::unix::fs::{MetadataExt, PermissionsExt},
        process::Command,
    };
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("source");
    let nested = input.join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    let file = nested.join("script");
    std::fs::write(&file, b"metadata payload").unwrap();
    let seconds = 1_700_000_000;
    let metadata = archive_core::EntryMetadata {
        modified: Some(archive_core::StoredTimestamp::UnixSeconds(seconds)),
        unix_mode: Some(0o751),
        ..Default::default()
    };
    archive_fs::apply_metadata(&std::fs::File::open(&file).unwrap(), &metadata).unwrap();
    let directory_metadata = archive_core::EntryMetadata {
        unix_mode: Some(0o550),
        ..metadata.clone()
    };
    archive_fs::apply_metadata(&std::fs::File::open(&nested).unwrap(), &directory_metadata)
        .unwrap();
    for extension in [
        "zip", "tar", "tar.gz", "tar.xz", "tar.bz2", "tar.br", "7z", "cab", "gz",
    ] {
        let archive = root.path().join(format!("metadata.{extension}"));
        let result = Command::new(env!("CARGO_BIN_EXE_arc"))
            .args(["-j", "create", "-i"])
            .arg(&input)
            .arg("-o")
            .arg(&archive)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{extension}: {}",
            String::from_utf8_lossy(&result.stdout)
        );
        let output = root.path().join(format!("out-{extension}"));
        let result = Command::new(env!("CARGO_BIN_EXE_arc"))
            .args(["-j", "x"])
            .arg(&archive)
            .arg("-o")
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{extension}: {}",
            String::from_utf8_lossy(&result.stdout)
        );
        let restored = output.join("nested/script");
        assert_eq!(std::fs::read(&restored).unwrap(), b"metadata payload");
        let file_metadata = std::fs::metadata(&restored).unwrap();
        assert_eq!(
            file_metadata.mtime(),
            seconds as i64,
            "{extension} timestamp"
        );
        if extension != "cab" && extension != "gz" {
            assert_eq!(
                file_metadata.mode() & 0o777,
                0o751,
                "{extension} permissions"
            );
            let directory = std::fs::metadata(output.join("nested")).unwrap();
            assert_eq!(
                directory.mode() & 0o777,
                0o550,
                "{extension} directory permissions"
            );
            assert_eq!(
                directory.mtime(),
                seconds as i64,
                "{extension} directory timestamp"
            );
        }
    }
    // Make temporary cleanup possible for the source and restored read-only directories.
    std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o750)).unwrap();
    for entry in std::fs::read_dir(root.path()).unwrap().flatten() {
        let directory = entry.path().join("nested");
        if directory.is_dir() {
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o750)).unwrap();
        }
    }
}

#[cfg(unix)]
#[test]
fn stdin_tar_restores_metadata_after_whole_batch_verification() {
    use std::{os::unix::fs::MetadataExt, process::Command};
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let file = source.join("script");
    std::fs::write(&file, b"stdin metadata").unwrap();
    archive_fs::apply_metadata(
        &std::fs::File::open(&file).unwrap(),
        &archive_core::EntryMetadata {
            modified: Some(archive_core::StoredTimestamp::UnixSeconds(1_700_000_000)),
            unix_mode: Some(0o755),
            ..Default::default()
        },
    )
    .unwrap();
    let archive = root.path().join("archive.tar");
    assert!(
        Command::new(env!("CARGO_BIN_EXE_arc"))
            .arg("cf")
            .arg(&archive)
            .arg(&source)
            .status()
            .unwrap()
            .success()
    );
    let output = root.path().join("output");
    let result = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["xf", "-", "-C"])
        .arg(&output)
        .stdin(std::fs::File::open(&archive).unwrap())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let metadata = std::fs::metadata(output.join("script")).unwrap();
    assert_eq!(metadata.mtime(), 1_700_000_000);
    assert_eq!(metadata.mode() & 0o777, 0o755);
}

#[cfg(unix)]
#[test]
fn encrypted_zip_and_sevenz_preserve_metadata_and_cab_preserves_readonly() {
    use std::{os::unix::fs::MetadataExt, process::Command};
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let file = source.join("readonly");
    std::fs::write(&file, b"metadata").unwrap();
    archive_fs::apply_metadata(
        &std::fs::File::open(&file).unwrap(),
        &archive_core::EntryMetadata {
            modified: Some(archive_core::StoredTimestamp::UnixSeconds(1_700_000_000)),
            unix_mode: Some(0o444),
            ..Default::default()
        },
    )
    .unwrap();
    let password = root.path().join("password");
    std::fs::write(&password, b"secret").unwrap();
    for extension in ["zip", "7z", "cab"] {
        let archive = root.path().join(format!("encrypted.{extension}"));
        let output = root.path().join(format!("out-{extension}"));
        let mut create = Command::new(env!("CARGO_BIN_EXE_arc"));
        create
            .args(["-j", "a", "-i"])
            .arg(&source)
            .arg("-o")
            .arg(&archive);
        let mut extract = Command::new(env!("CARGO_BIN_EXE_arc"));
        extract
            .args(["-j", "x"])
            .arg(&archive)
            .arg("-o")
            .arg(&output);
        if extension != "cab" {
            create.arg("-e").arg("-p").arg(&password);
            extract.arg("-p").arg(&password);
        }
        let result = create.output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        let result = extract.output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        let metadata = std::fs::metadata(output.join("readonly")).unwrap();
        assert_eq!(metadata.mtime(), 1_700_000_000, "{extension}");
        assert_eq!(metadata.mode() & 0o777, 0o444, "{extension}");
    }
}

#[cfg(unix)]
#[test]
fn wim_extraction_restores_the_selected_images_stored_modification_times() {
    use std::{os::unix::fs::MetadataExt, process::Command};
    let root = tempfile::tempdir().unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../archive-core/tests/fixtures/wimlib-lzms-solid.esd");
    let bytes = std::fs::read(&source).unwrap();
    let archive =
        archive_core::wim::WimArchive::open(&bytes, 1, archive_core::Limits::default()).unwrap();
    let output = root.path().join("output");
    let result = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["-j", "x", "-I", "1"])
        .arg(&source)
        .arg("-o")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    for entry in archive.entries() {
        let Some(archive_core::StoredTimestamp::UnixSeconds(seconds)) =
            archive.entry_metadata(entry.id).unwrap().modified
        else {
            panic!("fixture must have modification times");
        };
        assert_eq!(
            std::fs::metadata(output.join(&entry.name)).unwrap().mtime(),
            seconds as i64
        );
    }
}
