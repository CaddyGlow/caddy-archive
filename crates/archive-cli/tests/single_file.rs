use std::process::Command;
fn arc(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn single_file_codecs_roundtrip_with_explicit_raw_settings() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("input");
    let bytes = b"hello archive codecs hello archive codecs\n".repeat(20);
    std::fs::write(&original, &bytes).unwrap();
    for codec in [
        "deflate",
        "gzip",
        "zlib",
        "lzma",
        "lzma2",
        "xz",
        "bzip2",
        "brotli",
        "xpress",
        "xpress-plain",
        "lzx",
        "lzms",
        "lznt1",
        "quantum",
    ] {
        let packed = root.path().join(format!("packed.{codec}"));
        let decoded = root.path().join(format!("decoded-{codec}"));
        let result = arc(&[
            "compress",
            "-i",
            original.to_str().unwrap(),
            "-o",
            packed.to_str().unwrap(),
        ]);
        assert!(
            result.status.success(),
            "{codec}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let mut args = vec![
            "decompress",
            "-i",
            packed.to_str().unwrap(),
            "-o",
            decoded.to_str().unwrap(),
        ];
        let size = bytes.len().to_string();
        if ["xpress", "xpress-plain", "lzx", "lzms", "lznt1", "quantum"].contains(&codec) {
            args.extend(["--output-size", &size]);
        }
        let result = arc(&args);
        assert!(
            result.status.success(),
            "{codec}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(std::fs::read(decoded).unwrap(), bytes, "{codec}");
    }
}
#[test]
fn raw_missing_size_and_invalid_codec_do_not_publish() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("data.bin");
    let output = root.path().join("output");
    std::fs::write(&source, b"data").unwrap();
    for args in [
        vec!["decompress", "-f", "xpress"],
        vec!["compress", "-f", "unknown"],
    ] {
        let mut args = args;
        args.extend([
            "-i",
            source.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
        ]);
        assert!(!arc(&args).status.success());
        assert!(!output.exists());
    }
}
#[test]
fn archive_codec_selection_roundtrips_and_rejects_incompatible_codecs() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let bytes = b"archive codec selection".repeat(200);
    std::fs::write(source.join("payload"), &bytes).unwrap();
    for (format, codec) in [
        ("zip", "copy"),
        ("zip", "deflate"),
        ("7z", "deflate"),
        ("cab", "copy"),
        ("cab", "ms-zip"),
        ("cab", "lzx"),
        ("cab", "quantum"),
    ] {
        let packed = root.path().join(format!("{codec}.{format}"));
        let output = root.path().join(format!("out-{format}-{codec}"));
        let result = arc(&[
            "create",
            "-i",
            source.to_str().unwrap(),
            "-o",
            packed.to_str().unwrap(),
            "-c",
            codec,
        ]);
        assert!(
            result.status.success(),
            "{format}/{codec}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let result = arc(&[
            "extract",
            packed.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
        ]);
        assert!(
            result.status.success(),
            "{format}/{codec}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(std::fs::read(output.join("payload")).unwrap(), bytes);
    }
    let packed = root.path().join("bad.zip");
    let result = arc(&[
        "create",
        "-i",
        source.to_str().unwrap(),
        "-o",
        packed.to_str().unwrap(),
        "-c",
        "lzma2",
    ]);
    assert!(!result.status.success());
    assert!(!packed.exists());
}

#[test]
fn single_file_stdout_is_only_payload_and_magic_overrides_suffix() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("input");
    let packed = root.path().join("misleading.lzma");
    std::fs::write(&original, b"stdout payload").unwrap();
    let result = arc(&[
        "compress",
        "-f",
        "gzip",
        "-i",
        original.to_str().unwrap(),
        "-o",
        "-",
    ]);
    assert!(result.status.success());
    std::fs::write(&packed, &result.stdout).unwrap();
    let result = arc(&["decompress", "-i", packed.to_str().unwrap(), "-o", "-"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, b"stdout payload");
}

#[test]
fn stored_zip_roundtrips_with_both_encryption_modes() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("payload"), b"encrypted stored file").unwrap();
    let password = root.path().join("password");
    std::fs::write(&password, "test-password").unwrap();
    for mode in ["aes256", "zipcrypto"] {
        let packed = root.path().join(format!("{mode}.zip"));
        let output = root.path().join(mode);
        let result = arc(&[
            "create",
            "-i",
            source.to_str().unwrap(),
            "-o",
            packed.to_str().unwrap(),
            "-c",
            "copy",
            "-e",
            "-z",
            mode,
            "-p",
            password.to_str().unwrap(),
        ]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let result = arc(&[
            "extract",
            packed.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
            "-p",
            password.to_str().unwrap(),
        ]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            std::fs::read(output.join("payload")).unwrap(),
            b"encrypted stored file"
        );
    }
}
