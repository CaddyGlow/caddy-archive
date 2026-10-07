fn both32(bytes: &mut [u8], value: u32) {
    bytes[..4].copy_from_slice(&value.to_le_bytes());
    bytes[4..8].copy_from_slice(&value.to_be_bytes());
}
fn both16(bytes: &mut [u8], value: u16) {
    bytes[..2].copy_from_slice(&value.to_le_bytes());
    bytes[2..4].copy_from_slice(&value.to_be_bytes());
}
fn record(name: &[u8], sector: u32, size: u32, directory: bool) -> Vec<u8> {
    let length = (33 + name.len() + 1) & !1;
    let mut bytes = vec![0; length];
    bytes[0] = length as u8;
    both32(&mut bytes[2..10], sector);
    both32(&mut bytes[10..18], size);
    bytes[18..25].copy_from_slice(&[126, 10, 5, 12, 0, 0, 0]);
    bytes[25] = if directory { 2 } else { 0 };
    both16(&mut bytes[28..32], 1);
    bytes[32] = name.len() as u8;
    bytes[33..33 + name.len()].copy_from_slice(name);
    bytes
}

#[test]
fn nested_iso_uses_full_normalized_paths() {
    let mut image = vec![0; 23 * 2048];
    let primary = &mut image[16 * 2048..17 * 2048];
    primary[0] = 1;
    primary[1..6].copy_from_slice(b"CD001");
    primary[6] = 1;
    both32(&mut primary[80..88], 23);
    both16(&mut primary[120..124], 1);
    both16(&mut primary[124..128], 1);
    both16(&mut primary[128..132], 2048);
    let root = record(&[0], 20, 2048, true);
    primary[156..156 + root.len()].copy_from_slice(&root);
    image[17 * 2048] = 255;
    image[17 * 2048 + 1..17 * 2048 + 6].copy_from_slice(b"CD001");
    image[17 * 2048 + 6] = 1;
    for (sector, records) in [
        (
            20,
            vec![
                root,
                record(&[1], 20, 2048, true),
                record(b"NESTED", 21, 2048, true),
            ],
        ),
        (
            21,
            vec![
                record(&[0], 21, 2048, true),
                record(&[1], 20, 2048, true),
                record(b"HELLO.TXT;1", 22, 5, false),
            ],
        ),
    ] {
        let mut offset = sector * 2048;
        for record in records {
            image[offset..offset + record.len()].copy_from_slice(&record);
            offset += record.len();
        }
    }
    image[22 * 2048..22 * 2048 + 5].copy_from_slice(b"hello");
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("nested.iso");
    std::fs::write(&archive, image).unwrap();
    let output = root.path().join("output");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_arc"))
        .args(["--json", "extract"])
        .arg(&archive)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert_eq!(
        std::fs::read(output.join("NESTED/HELLO.TXT")).unwrap(),
        b"hello"
    );
    assert!(!output.join("HELLO.TXT;1").exists());
}
