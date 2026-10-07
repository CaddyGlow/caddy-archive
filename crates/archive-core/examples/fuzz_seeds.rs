//! Generate bounded archive/optical seeds; source fixtures remain unchanged.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use archive_core::{CreateEntry, EntryKind, Format, Limits};
    use std::{fs, io::Cursor, path::Path};
    let output = std::env::args().nth(1).ok_or("output directory required")?;
    let output = Path::new(&output);
    fs::create_dir_all(output.join("archive"))?;
    fs::create_dir_all(output.join("optical"))?;
    let entry = CreateEntry {
        name: format!("{}/payload", "long".repeat(50)),
        kind: EntryKind::File,
        data: b"fuzz seed".repeat(100),
    };
    for format in [
        Format::Zip,
        Format::Tar,
        Format::TarGzip,
        Format::Xz,
        Format::TarXz,
    ] {
        let mut bytes = Cursor::new(Vec::new());
        archive_core::create(
            format,
            std::slice::from_ref(&entry),
            &mut bytes,
            Limits::default(),
        )?;
        fs::write(
            output.join("archive").join(format!("generated-{format:?}")),
            bytes.into_inner(),
        )?;
    }
    for (selector, format) in [
        (0, Format::Deflate),
        (1, Format::Brotli),
        (2, Format::Bzip2),
        (3, Format::TarBrotli),
        (4, Format::TarBzip2),
    ] {
        let mut bytes = Cursor::new(Vec::new());
        archive_core::create(
            format,
            std::slice::from_ref(&entry),
            &mut bytes,
            Limits::default(),
        )?;
        let mut selected = vec![selector];
        selected.extend_from_slice(&bytes.into_inner());
        fs::write(
            output.join("archive").join(format!("selected-{format:?}")),
            selected,
        )?;
    }
    let source = tempfile::tempdir()?;
    fs::create_dir_all(source.path().join("boot"))?;
    fs::create_dir_all(source.path().join("efi/microsoft/boot"))?;
    fs::write(source.path().join("boot/etfsboot.com"), [1; 4096])?;
    fs::write(
        source.path().join("efi/microsoft/boot/efisys.bin"),
        [2; 4096],
    )?;
    fs::write(source.path().join("payload.txt"), b"optical seed")?;
    let destination = tempfile::tempdir()?;
    let image = destination.path().join("fuzz-seed-media.iso");
    libmkiso::write_iso(source.path(), &image)?;
    let bytes = fs::read(&image)?;
    // The bounded harness admits at most 1 MiB; retain the descriptor-bearing prefix.
    fs::write(
        output.join("optical/generated-iso-udf-prefix"),
        &bytes[..bytes.len().min(1 << 20)],
    )?;
    fs::remove_file(image)?;
    Ok(())
}
