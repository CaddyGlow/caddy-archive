//! CLI regression gates for the published 0.2.2 explicit MSI media backend.
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "../../archive-wasm/tests/fixtures/package/media-migration/manifest.json"
    ))
    .unwrap()
}
fn fixture_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../archive-wasm/tests/fixtures/package/media-migration")
        .join(relative)
}
fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "{} / {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}
fn payload(file: &Value) -> Vec<u8> {
    file["payload_hex"]
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
fn mapped_command(profile: &Value, operation: &str, input: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_arc"));
    command.arg("--json");
    for media in profile["media"].as_array().unwrap() {
        command.arg("--media").arg(format!(
            "{}={}",
            media["name"].as_str().unwrap(),
            fixture_path(media["artifact"].as_str().unwrap()).display()
        ));
    }
    command.arg(operation).arg(input);
    command
}

#[test]
fn every_media_layout_lists_tests_and_extracts_exact_payloads_without_changing_sidecars() {
    let root = tempfile::tempdir().unwrap();
    let manifest = manifest();
    for (name, profile) in manifest["profiles"].as_object().unwrap() {
        let input = fixture_path(profile["msi"].as_str().unwrap());
        let source = std::fs::read(&input).unwrap();
        let originals: Vec<_> = profile["media"]
            .as_array()
            .unwrap()
            .iter()
            .map(|media| {
                let path = fixture_path(media["artifact"].as_str().unwrap());
                let bytes = std::fs::read(&path).unwrap();
                (path, bytes)
            })
            .collect();
        let listed = mapped_command(profile, "list", &input).output().unwrap();
        assert!(listed.status.success(), "{name}: {}", json(&listed));
        let listing = json(&listed);
        let actual_files = listing["entries"].as_array().unwrap();
        assert_eq!(
            actual_files.len(),
            profile["files"].as_array().unwrap().len()
        );
        for expected in profile["files"].as_array().unwrap() {
            let actual = actual_files
                .iter()
                .find(|entry| entry["id"] == expected["id"])
                .unwrap();
            assert_eq!(actual["name"], expected["path"]);
            assert_eq!(actual["source_path"], expected["source_path"]);
            assert_eq!(actual["cabinet"], expected["cabinet"]);
            assert_eq!(actual["sequence"], expected["sequence"]);
            assert_eq!(actual["size"], expected["size"]);
        }
        let tested = mapped_command(profile, "test", &input).output().unwrap();
        assert!(tested.status.success(), "{name}: {}", json(&tested));
        let destination = root.path().join(name);
        let extracted = mapped_command(profile, "extract", &input)
            .arg("--output")
            .arg(&destination)
            .output()
            .unwrap();
        assert!(extracted.status.success(), "{name}: {}", json(&extracted));
        let total: u64 = profile["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|file| file["size"].as_u64().unwrap())
            .sum();
        assert_eq!(json(&extracted)["bytes"], total);
        assert_eq!(json(&extracted)["files_verified"], 3);
        for file in profile["files"].as_array().unwrap() {
            assert_eq!(
                std::fs::read(destination.join(file["path"].as_str().unwrap())).unwrap(),
                payload(file)
            );
        }
        assert_eq!(std::fs::read(input).unwrap(), source);
        for (path, bytes) in originals {
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
    }
}

#[test]
fn external_and_loose_media_never_search_adjacent_files_or_the_working_directory() {
    let root = tempfile::tempdir().unwrap();
    let manifest = manifest();
    for (name, profile) in manifest["profiles"].as_object().unwrap() {
        if profile["media"].as_array().unwrap().is_empty() {
            continue;
        }
        let directory = root.path().join(name);
        std::fs::create_dir_all(&directory).unwrap();
        let input = directory.join("installer.msi");
        std::fs::copy(fixture_path(profile["msi"].as_str().unwrap()), &input).unwrap();
        for media in profile["media"].as_array().unwrap() {
            let target = directory.join(media["name"].as_str().unwrap());
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::copy(fixture_path(media["artifact"].as_str().unwrap()), target).unwrap();
        }
        let output = directory.join("out");
        let failed = Command::new(env!("CARGO_BIN_EXE_arc"))
            .current_dir(&directory)
            .args(["--json", "extract"])
            .arg(&input)
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap();
        assert!(!failed.status.success(), "{name}");
        assert!(
            json(&failed)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("media")
        );
        assert_eq!(std::fs::read_dir(output).unwrap().count(), 0);
    }
}

#[test]
fn missing_later_cabinet_and_loose_file_never_publish_earlier_embedded_payloads() {
    let root = tempfile::tempdir().unwrap();
    let manifest = manifest();
    for name in ["partitioned", "mixed", "mixed-loose"] {
        let profile = &manifest["profiles"][name];
        let input = fixture_path(profile["msi"].as_str().unwrap());
        let output = root.path().join(name);
        let mut command = Command::new(env!("CARGO_BIN_EXE_arc"));
        command.arg("--json");
        for media in profile["media"].as_array().unwrap() {
            let media_name = media["name"].as_str().unwrap();
            if media_name == "later.cab" || media_name == "SourceMedia/third.txt" || name == "mixed"
            {
                continue;
            }
            command.arg("--media").arg(format!(
                "{media_name}={}",
                fixture_path(media["artifact"].as_str().unwrap()).display()
            ));
        }
        let result = command
            .arg("extract")
            .arg(input)
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap();
        assert!(!result.status.success(), "{name}");
        assert_eq!(std::fs::read_dir(output).unwrap().count(), 0);
    }
}

#[test]
fn entry_total_and_row_limits_fail_with_resource_code_and_leave_no_payloads() {
    let root = tempfile::tempdir().unwrap();
    let manifest = manifest();
    for name in [
        "embedded",
        "external",
        "loose",
        "partitioned",
        "mixed",
        "mixed-loose",
    ] {
        let profile = &manifest["profiles"][name];
        let input = fixture_path(profile["msi"].as_str().unwrap());
        for (option, value) in [
            ("--max-entry-bytes", "1"),
            ("--max-total-bytes", "1000"),
            ("--max-entries", "1"),
        ] {
            let output = root
                .path()
                .join(format!("{name}-{}", option.trim_start_matches('-')));
            let result = mapped_command(profile, "extract", &input)
                .args([option, value])
                .arg("--output")
                .arg(&output)
                .output()
                .unwrap();
            assert_eq!(
                result.status.code(),
                Some(5),
                "{name} {option}: {}",
                json(&result)
            );
            if output.exists() {
                assert_eq!(std::fs::read_dir(output).unwrap().count(), 0);
            }
        }
    }
}

#[test]
fn external_media_actual_input_size_is_bounded_independently_of_msi_input_size() {
    let root = tempfile::tempdir().unwrap();
    let manifest = manifest();
    let profile = &manifest["profiles"]["external"];
    let input = fixture_path(profile["msi"].as_str().unwrap());
    let limit = std::fs::metadata(&input).unwrap().len();
    let media = &profile["media"][0];
    let path = root.path().join("oversized.cab");
    let mut bytes = std::fs::read(fixture_path(media["artifact"].as_str().unwrap())).unwrap();
    bytes.resize(usize::try_from(limit + 1).unwrap(), 0);
    std::fs::write(&path, &bytes).unwrap();
    let output = root.path().join("out");
    let result = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "--max-input-bytes"])
        .arg(limit.to_string())
        .arg("--media")
        .arg(format!(
            "{}={}",
            media["name"].as_str().unwrap(),
            path.display()
        ))
        .arg("extract")
        .arg(&input)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(5), "{}", json(&result));
    assert!(
        json(&result)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("media input")
    );
    assert_eq!(std::fs::read_dir(output).unwrap().count(), 0);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn duplicate_explicit_media_names_fail_before_creating_the_destination() {
    let root = tempfile::tempdir().unwrap();
    let manifest = manifest();
    let profile = &manifest["profiles"]["external"];
    let input = fixture_path(profile["msi"].as_str().unwrap());
    let media = &profile["media"][0];
    let mapping = format!(
        "{}={}",
        media["name"].as_str().unwrap(),
        fixture_path(media["artifact"].as_str().unwrap()).display()
    );
    let output = root.path().join("out");
    let result = Command::new(env!("CARGO_BIN_EXE_arc"))
        .args([
            "--json", "--media", &mapping, "--media", &mapping, "extract",
        ])
        .arg(&input)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(
        json(&result)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("duplicate explicit media")
    );
    assert!(!output.exists());
}
