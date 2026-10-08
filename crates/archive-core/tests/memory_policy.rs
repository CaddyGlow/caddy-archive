#![cfg(all(feature = "sevenz", feature = "parallel", not(target_arch = "wasm32")))]

use archive_core::{
    Archive, CreateEntry, CreateOptions, EntryKind, Format, Limits, MemoryOperation, MemoryUsage,
    SevenZipCompression, create_with_options,
};
use std::io::Cursor;

#[test]
fn ram_policy_reduces_folder_workers_without_changing_verified_output() {
    let entries: Vec<_> = (0..4)
        .map(|index| CreateEntry {
            name: format!("{index}.bin"),
            data: vec![index; 128 * 1024],
            kind: EntryKind::File,
        })
        .collect();
    let mut bytes = Cursor::new(Vec::new());
    create_with_options(
        Format::SevenZip,
        &entries,
        &mut bytes,
        Limits::default(),
        CreateOptions {
            sevenz_compression: SevenZipCompression::Copy,
            ..Default::default()
        },
    )
    .unwrap();
    let mut used = Vec::new();
    for physical_ram in [1 << 20, 64 << 20] {
        let limits = Limits {
            max_active_workspace_bytes: MemoryUsage::Auto
                .budget(Some(physical_ram), MemoryOperation::Decompress),
            ..Limits::default()
        };
        let mut archive = Archive::open(Cursor::new(bytes.get_ref()), limits).unwrap();
        let ids: Vec<_> = archive.entries().iter().map(|entry| entry.id).collect();
        let mut actual = vec![Vec::new(); entries.len()];
        let report = archive
            .extract_selected_parallel(&ids, 4, &|| false, &mut |id, bytes| {
                actual[id.0].extend_from_slice(bytes);
                Ok(())
            })
            .unwrap();
        assert!(report.report.verified);
        for (actual, expected) in actual.iter().zip(&entries) {
            assert_eq!(actual, &expected.data);
        }
        used.push(report.workers_used);
    }
    assert_eq!(used[1], 4);
    assert!(used[0] >= 1 && used[0] < used[1]);
}
