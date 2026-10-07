use std::{
    path::Path,
    process::{Command, Output},
};

fn success(output: Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn create(input: &Path, output: &Path, format: Option<&str>) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_arc"));
    command
        .args(["--json", "create", "--input"])
        .arg(input)
        .arg("--output")
        .arg(output);
    if let Some(format) = format {
        command.args(["--format", format]);
    }
    success(command.output().unwrap());
}

#[test]
fn output_extensions_infer_creation_formats_and_compound_suffixes() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    std::fs::create_dir(&input).unwrap();
    std::fs::write(input.join("payload"), b"detected format payload").unwrap();
    for (suffix, expected) in [
        ("ZIP", "zip"),
        ("7z", "sevenzip"),
        ("tar", "tar"),
        ("tar.gz", "targzip"),
        ("TGZ", "targzip"),
        ("tar.xz", "tarxz"),
        ("txz", "tarxz"),
        ("tar.bz2", "tarbzip2"),
        ("tbz2", "tarbzip2"),
        ("tar.br", "tarbrotli"),
        ("cab", "cab"),
        ("gz", "gzip"),
        ("zlib", "zlib"),
        ("xz", "xz"),
        ("bz2", "bzip2"),
        ("br", "brotli"),
        ("lzma", "lzma"),
        ("deflate", "deflate"),
    ] {
        let archive = root.path().join(format!("archive.{suffix}"));
        create(&input, &archive, None);
        let mut command = Command::new(env!("CARGO_BIN_EXE_arc"));
        command.args(["--json", "list"]).arg(&archive);
        let listed = success(command.output().unwrap());
        assert_eq!(listed["format"], expected, "{suffix}");
        success(
            Command::new(env!("CARGO_BIN_EXE_arc"))
                .args(["--json", "test"])
                .arg(&archive)
                .output()
                .unwrap(),
        );
    }
}

#[test]
fn content_detection_and_explicit_formats_override_misleading_extensions() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    std::fs::create_dir(&input).unwrap();
    std::fs::write(input.join("payload"), b"payload").unwrap();
    for suffix in ["bin", "tar", "wim", "msi"] {
        let archive = root.path().join(format!("zip.{suffix}"));
        create(&input, &archive, Some("zip"));
        let listed = success(
            Command::new(env!("CARGO_BIN_EXE_arc"))
                .args(["--json", "list"])
                .arg(&archive)
                .output()
                .unwrap(),
        );
        assert_eq!(listed["format"], "zip");
        let output = root.path().join(format!("out-{suffix}"));
        success(
            Command::new(env!("CARGO_BIN_EXE_arc"))
                .args(["--json", "extract"])
                .arg(&archive)
                .arg("--output")
                .arg(&output)
                .output()
                .unwrap(),
        );
        assert_eq!(std::fs::read(output.join("payload")).unwrap(), b"payload");
    }
    let archive = root.path().join("explicit.msix");
    create(&input, &archive, Some("zip"));
    let listed = success(
        Command::new(env!("CARGO_BIN_EXE_arc"))
            .args(["--json", "list"])
            .arg(&archive)
            .args(["--format", "zip"])
            .output()
            .unwrap(),
    );
    assert_eq!(listed["format"], "zip");
}

#[test]
fn ambiguous_creation_outputs_require_an_explicit_format_without_publication() {
    let root = tempfile::tempdir().unwrap();
    for name in ["archive", "archive.unknown", "-"] {
        let output = if name == "-" {
            "-".into()
        } else {
            root.path().join(name)
        };
        let result = Command::new(env!("CARGO_BIN_EXE_arc"))
            .args(["--json", "create", "--input"])
            .arg(root.path())
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap();
        assert!(!result.status.success());
        let result: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert!(
            result["error"]["message"]
                .as_str()
                .unwrap()
                .contains("specify --format")
        );
        assert!(!output.exists());
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn stream_commands_infer_output_wrapper_and_detect_input_wrapper() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::write(&source, b"stream wrapper detection").unwrap();
    for extension in ["gz", "zlib", "deflate"] {
        let compressed = root.path().join(format!("compressed.{extension}"));
        success(
            Command::new(env!("CARGO_BIN_EXE_arc"))
                .args(["--json", "deflate", "--input"])
                .arg(&source)
                .arg("--output")
                .arg(&compressed)
                .output()
                .unwrap(),
        );
        let renamed = root.path().join("compressed.bin");
        std::fs::rename(&compressed, &renamed).unwrap();
        let output = root.path().join(format!("decoded-{extension}"));
        success(
            Command::new(env!("CARGO_BIN_EXE_arc"))
                .args(["--json", "inflate", "--input"])
                .arg(&renamed)
                .arg("--output")
                .arg(&output)
                .output()
                .unwrap(),
        );
        assert_eq!(std::fs::read(output).unwrap(), b"stream wrapper detection");
    }
}

#[test]
fn stdin_tar_is_detected_without_a_format_option() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    std::fs::create_dir(&input).unwrap();
    std::fs::write(input.join("payload"), b"stdin detection").unwrap();
    let archive = root.path().join("archive.tar");
    create(&input, &archive, None);
    for operation in ["list", "test", "extract"] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_arc"));
        command
            .args(["--json", operation, "-"])
            .stdin(std::fs::File::open(&archive).unwrap());
        if operation == "extract" {
            command.arg("--output").arg(root.path().join("output"));
        }
        assert_eq!(success(command.output().unwrap())["format"], "tar");
    }
    assert_eq!(
        std::fs::read(root.path().join("output/payload")).unwrap(),
        b"stdin detection"
    );
}

#[test]
fn renamed_bzip2_tar_detects_inner_tar_but_explicit_bzip2_keeps_raw_view() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    std::fs::create_dir(&input).unwrap();
    std::fs::write(input.join("payload"), b"inner tar detection").unwrap();
    let archive = root.path().join("renamed.bin");
    create(&input, &archive, Some("tar.bz2"));
    let listed = success(
        Command::new(env!("CARGO_BIN_EXE_arc"))
            .args(["--json", "list"])
            .arg(&archive)
            .output()
            .unwrap(),
    );
    assert_eq!(listed["format"], "tarbzip2");
    let raw = success(
        Command::new(env!("CARGO_BIN_EXE_arc"))
            .args(["--json", "list"])
            .arg(&archive)
            .args(["--format", "bz2"])
            .output()
            .unwrap(),
    );
    assert_eq!(raw["format"], "bzip2");
    let output = root.path().join("output");
    success(
        Command::new(env!("CARGO_BIN_EXE_arc"))
            .args(["--json", "extract"])
            .arg(&archive)
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap(),
    );
    assert_eq!(
        std::fs::read(output.join("payload")).unwrap(),
        b"inner tar detection"
    );
}

#[test]
fn renamed_wim_and_msi_use_content_signatures() {
    let root = tempfile::tempdir().unwrap();
    for (source, expected) in [
        (
            "../archive-core/tests/fixtures/wimlib-lzms-solid.esd",
            "wim",
        ),
        ("tests/fixtures/external.msi", "msi"),
    ] {
        let renamed = root.path().join(format!("{expected}.bin"));
        std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join(source), &renamed).unwrap();
        let listed = success(
            Command::new(env!("CARGO_BIN_EXE_arc"))
                .args(["--json", "list"])
                .arg(renamed)
                .output()
                .unwrap(),
        );
        assert_eq!(listed["format"], expected);
    }
}
