#![cfg(feature = "wim")]
use archive_core::{Limits, wim::WimArchive};

#[test]
fn selected_image_payload_matches_existing_independently_generated_fixture() {
    let bytes =
        include_bytes!("../../../../wim-rs/crates/wim-format/tests/fixtures/xpress-resource.wim");
    let archive = WimArchive::open(bytes, 1, Limits::default()).unwrap();
    assert_eq!(archive.image_count(), 1);
    let file = archive
        .entries()
        .iter()
        .find(|entry| entry.size > 0)
        .unwrap();
    let mut expected = (0..300).flat_map(|_| 0u8..=255).collect::<Vec<_>>();
    expected.extend_from_slice(b"last chunk");
    assert_eq!(
        archive.read_entry(file.id, expected.len() as u64).unwrap(),
        expected
    );
}

#[test]
fn image_selector_and_metadata_budget_are_enforced() {
    let bytes =
        include_bytes!("../../../../wim-rs/crates/wim-format/tests/fixtures/xpress-resource.wim");
    assert!(WimArchive::open(bytes, 0, Limits::default()).is_err());
    assert!(WimArchive::open(bytes, 2, Limits::default()).is_err());
    let limits = Limits {
        max_metadata_bytes: 1,
        ..Limits::default()
    };
    assert!(WimArchive::open(bytes, 1, limits).is_err());
    let limits = Limits {
        max_active_workspace_bytes: 1,
        ..Limits::default()
    };
    assert!(WimArchive::open(bytes, 1, limits).is_err());
}

#[test]
fn independently_generated_lzms_solid_esd_verifies_every_payload() {
    let bytes = include_bytes!("fixtures/wimlib-lzms-solid.esd");
    let archive = WimArchive::open(bytes, 1, Limits::default()).unwrap();
    let report = archive.test().unwrap();
    assert_eq!((report.entries, report.bytes), (3, 39365));
    assert!(report.verified);
    assert_eq!(
        archive
            .entries()
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["iso9660.rs", "lib.rs", "writer.rs"]
    );
}

#[test]
fn xml_image_listing_and_exact_name_selection_agree() {
    let bytes = include_bytes!("fixtures/wimlib-lzms-solid.esd");
    let images = archive_core::wim::images(bytes, Limits::default()).unwrap();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].index, 1);
    let name = images[0].name.as_deref().unwrap();
    assert!(!name.is_empty());
    let archive = WimArchive::open_by_name(bytes, name, Limits::default()).unwrap();
    assert_eq!(archive.image(), images[0].index);
    assert!(archive.test().unwrap().verified);
    assert!(WimArchive::open_by_name(bytes, "missing image", Limits::default()).is_err());
    assert!(
        archive_core::wim::images(
            bytes,
            Limits {
                max_entries: 0,
                ..Limits::default()
            }
        )
        .is_err()
    );
    assert!(
        archive_core::wim::images(
            bytes,
            Limits {
                max_metadata_bytes: 1,
                ..Limits::default()
            }
        )
        .is_err()
    );
}
