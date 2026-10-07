#![cfg(all(feature = "sevenz", feature = "parallel", feature = "crypto"))]
use archive_core::{Archive, Limits};
use std::{collections::BTreeMap, fs, process::Command};

#[test]
#[ignore = "requires independent 7z command"]
fn independent_filtered_encrypted_folders_match_one_two_four_workers() {
    let temp = tempfile::tempdir().unwrap();
    let mut expected = BTreeMap::new();
    for id in 0..4u8 {
        let name = format!("{id}.bin");
        let mut data = vec![id; 256 * 1024];
        for offset in (0..data.len() - 5).step_by(11) {
            data[offset] = 0xe8;
            data[offset + 1..offset + 5].copy_from_slice(&(offset as u32).to_le_bytes());
        }
        fs::write(temp.path().join(&name), &data).unwrap();
        expected.insert(name, data);
    }
    for solid in [false, true] {
        let filename = if solid { "solid.7z" } else { "folders.7z" };
        let output = Command::new("7z")
            .current_dir(temp.path())
            .args([
                "a",
                "-t7z",
                "-m0=BCJ",
                "-m1=LZMA2",
                "-pworker-test",
                "-mhe=on",
            ])
            .arg(if solid { "-ms=on" } else { "-ms=off" })
            .arg(filename)
            .args(expected.keys())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        for workers in [1, 2, 4] {
            let mut archive = Archive::open_with_password(
                fs::File::open(temp.path().join(filename)).unwrap(),
                Limits::default(),
                b"worker-test",
            )
            .unwrap();
            assert!(archive.entries().iter().all(|entry| entry.encrypted));
            let ids: Vec<_> = archive.entries().iter().map(|entry| entry.id).collect();
            let names: BTreeMap<_, _> = archive
                .entries()
                .iter()
                .map(|entry| (entry.id.0, entry.name.clone()))
                .collect();
            let mut actual = BTreeMap::<usize, Vec<u8>>::new();
            let report = archive
                .extract_selected_parallel(&ids, workers, &|| false, &mut |id, bytes| {
                    actual.entry(id.0).or_default().extend_from_slice(bytes);
                    Ok(())
                })
                .unwrap();
            assert!(report.report.verified);
            assert_eq!(report.workers_used, if solid { 1 } else { workers });
            assert_eq!(report.folder_tasks, if solid { 1 } else { 4 });
            assert_eq!(report.decoded_bytes, 4 * 256 * 1024);
            if solid && workers > 1 {
                assert!(report.fallback_reason.as_deref().unwrap().contains("solid"));
            }
            for (id, bytes) in actual {
                assert_eq!(bytes, expected[&names[&id]]);
            }
        }
    }
}
