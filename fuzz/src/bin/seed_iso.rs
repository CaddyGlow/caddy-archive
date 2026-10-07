//! Seed complete ISO images for primary, Joliet and Rock Ridge parser fuzzing.
use libmkiso::{
    AdvancedBootOptions, BootEntry, FilenamePolicy, HybridLayout, HybridOptions, IsoLevel,
    IsoOptions, write_iso9660_with_options,
};
use std::{error::Error, fs, io::Cursor, path::Path};
fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args()
        .nth(1)
        .ok_or("usage: seed_iso OUTPUT_DIRECTORY")?;
    let output = Path::new(&output);
    fs::create_dir_all(output)?;
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("Mixed Directory"))?;
    fs::write(source.join("Mixed Directory/日本語.txt"), vec![0x61; 10001])?;
    fs::write(source.join("a".repeat(100)), b"long name")?;
    fs::write(source.join("empty"), [])?;
    fs::write(source.join("bios.bin"), vec![0x31; 4096])?;
    fs::write(source.join("efi.img"), vec![0x72; 8192])?;
    let mut count = 0;
    for level in [IsoLevel::Level1, IsoLevel::Level2, IsoLevel::Level3] {
        for joliet_level in 1..=3 {
            for rock_ridge in [false, true] {
                let options = IsoOptions {
                    level,
                    joliet: true,
                    joliet_level,
                    joliet_max_name: 103,
                    rock_ridge,
                    filename_policy: FilenamePolicy::Mangle,
                    extent_bytes: 2048,
                    ..Default::default()
                };
                seed(
                    &source,
                    output,
                    &format!("native-{level:?}-joliet-{joliet_level}-rr-{rock_ridge}.iso"),
                    &options,
                )?;
                count += 1;
            }
        }
    }
    for layout in [HybridLayout::Mbr, HybridLayout::Gpt, HybridLayout::MbrGpt] {
        let mut bios = BootEntry::bios("bios.bin");
        bios.boot_info_table = true;
        bios.grub2_boot_info = true;
        let options = IsoOptions {
            rock_ridge: true,
            joliet: true,
            joliet_max_name: 103,
            filename_policy: FilenamePolicy::Mangle,
            advanced_boot: AdvancedBootOptions {
                entries: vec![bios, BootEntry::efi("efi.img")],
                ..Default::default()
            },
            hybrid: Some(HybridOptions {
                layout,
                efi_partition: Some("efi.img".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        seed(
            &source,
            output,
            &format!("native-hybrid-{layout:?}.iso"),
            &options,
        )?;
        count += 1;
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("Mixed Directory/日本語.txt", source.join("link"))?;
        let long = std::iter::repeat_n("x".repeat(200), 15)
            .collect::<Vec<_>>()
            .join("/");
        std::os::unix::fs::symlink(long, source.join("long-link"))?;
        fs::hard_link(
            source.join("Mixed Directory/日本語.txt"),
            source.join("alias"),
        )?;
        seed(
            &source,
            output,
            "native-rock-ridge-links.iso",
            &IsoOptions {
                rock_ridge: true,
                filename_policy: FilenamePolicy::Mangle,
                ..Default::default()
            },
        )?;
        count += 1;
    }
    println!("Generated {count} complete ISO fuzz seeds");
    Ok(())
}
fn seed(
    source: &Path,
    output: &Path,
    name: &str,
    options: &IsoOptions,
) -> Result<(), Box<dyn Error>> {
    let file = output.join(name);
    write_iso9660_with_options(source, &file, options)?;
    let bytes = fs::read(file)?;
    assert!(bytes.len() <= 1 << 20);
    libmkiso::iso9660::IsoReader::open(Cursor::new(&bytes), Default::default())?;
    archive_fuzz::archives::optical(&bytes);
    Ok(())
}
