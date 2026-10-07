use libmkiso::{FilenamePolicy, IsoLevel, IsoOptions, write_iso9660_with_options};
#[test]
fn complete_iso_seed_and_descriptor_mutations_reach_all_namespace_readers() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    std::fs::create_dir_all(source.join("Mixed Directory")).unwrap();
    std::fs::write(source.join("Mixed Directory/日本語.txt"), vec![0x61; 10001]).unwrap();
    let output = directory.path().join("image.iso");
    write_iso9660_with_options(
        &source,
        &output,
        &IsoOptions {
            level: IsoLevel::Level3,
            extent_bytes: 2048,
            rock_ridge: true,
            joliet: true,
            filename_policy: FilenamePolicy::Mangle,
            ..Default::default()
        },
    )
    .unwrap();
    let bytes = std::fs::read(output).unwrap();
    archive_fuzz::archives::optical(&bytes);
    for block in 16..bytes.len() / 2048 {
        for offset in [0, 2, 4, 10, 25, 32, 64, 80, 128, 156, 172] {
            for byte in [0, 1, 255] {
                let mut mutated = bytes.clone();
                mutated[block * 2048 + offset] = byte;
                archive_fuzz::archives::optical(&mutated);
            }
        }
    }
}
