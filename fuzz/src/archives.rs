//! Archive and package parsing targets with small decode and KDF budgets.
use archive_core::{Archive, Format, Limits};
use std::io::Cursor;

struct CallerBytes<'a>(&'a [u8]);
impl ms_package::MediaResolver for CallerBytes<'_> {
    fn resolve(&mut self, _name: &str, maximum: u64) -> ms_package::Result<Vec<u8>> {
        if self.0.len() as u64 > maximum {
            return Err(ms_package::Error::Limit("fuzz caller media bytes"));
        }
        Ok(self.0.to_vec())
    }
}

fn limits() -> Limits {
    Limits {
        max_buffered_bytes: 2 << 20,
        max_entries: 128,
        max_metadata_bytes: 1 << 20,
        max_entry_bytes: 1 << 20,
        max_total_bytes: 2 << 20,
        max_dictionary_bytes: 32 << 20,
        max_input_bytes: 1 << 20,
        max_active_workspace_bytes: 96 << 20,
        max_pending_output_bytes: 1 << 20,
        max_password_iterations: 1024,
        max_nesting_depth: 16,
        max_workers: 1,
    }
}

/// Probe, index and integrity-test mutated containers, including crypto properties.
pub fn archive(data: &[u8]) {
    if data.len() > 1 << 20 {
        return;
    }
    let _ = archive_core::probe(data);
    if let Ok(mut archive) = Archive::open_with_password(Cursor::new(data), limits(), b"fixture") {
        let _ = archive.test();
    }
    let _ = archive_core::wim::WimArchive::open(data, 1, limits());
    if let Some(selector) = data.first() {
        let format = match selector % 5 {
            0 => Format::Deflate,
            1 => Format::Brotli,
            2 => Format::Bzip2,
            3 => Format::TarBrotli,
            _ => Format::TarBzip2,
        };
        if let Ok(mut archive) = Archive::open_as(Cursor::new(&data[1..]), format, limits()) {
            let _ = archive.test();
        }
    }
}

/// Exercise APPX metadata/block maps and MSI compound-storage/database relationships.
pub fn package(data: &[u8]) {
    if data.len() > 1 << 20 {
        return;
    }
    if let Ok(mut package) = ms_package::AppxPackage::open(
        Cursor::new(data),
        package_core::Limits {
            max_buffered_bytes: 2 << 20,
            max_entries: 128,
            max_metadata_bytes: 1 << 20,
            max_entry_bytes: 1 << 20,
            max_total_bytes: 2 << 20,
            max_dictionary_bytes: 32 << 20,
            max_input_bytes: 1 << 20,
            max_active_workspace_bytes: 96 << 20,
            max_pending_output_bytes: 1 << 20,
            max_password_iterations: 1024,
            max_nesting_depth: 16,
            max_workers: 1,
        },
        1 << 20,
    ) {
        let _ = package.validate(2 << 20);
    }
    if let Ok(mut package) =
        ms_package::InstallerPackage::open_bounded(Cursor::new(data), 128, 1 << 20)
    {
        if let Ok(files) = package.files() {
            let mut remaining = 2 << 20;
            for file in files.into_iter().take(8) {
                if file.size > 1 << 20 || file.size > remaining {
                    continue;
                }
                // The mutated input is the only external source. Never discover
                // host files while exercising the backend's bounded media path.
                if let Ok(bytes) =
                    package.read_file(&file, &mut CallerBytes(data), remaining.min(1 << 20))
                {
                    remaining -= bytes.len() as u64;
                }
            }
        }
        for table in package.tables().into_iter().take(16) {
            let _ = package.table(&table);
        }
        for stream in package.streams().into_iter().take(16) {
            let _ = package.read_stream(&stream, 1 << 20);
        }
    }
}

/// Exercise ISO9660 byte orders and UDF descriptor checks/extent graphs.
pub fn optical(data: &[u8]) {
    if data.len() > 1 << 20 {
        return;
    }
    use libmkiso::iso9660::{IsoReader, Namespace, ReadOptions};
    for namespace in [
        Namespace::Primary,
        Namespace::Joliet,
        Namespace::RockRidge,
        Namespace::PreferRockRidge,
    ] {
        let options = ReadOptions {
            namespace,
            limits: libmkiso::iso9660::Limits {
                max_entries: 128,
                max_metadata_bytes: 1 << 20,
                max_nesting_depth: 16,
            },
        };
        if let Ok(mut reader) = IsoReader::open_with_options(Cursor::new(data), options) {
            let sizes: Vec<_> = reader.entries().iter().map(|entry| entry.size).collect();
            let mut remaining = 2 << 20;
            for (index, size) in sizes.into_iter().enumerate() {
                if size > 1 << 20 || size > remaining {
                    continue;
                }
                let count = reader
                    .extract(index, &mut std::io::sink())
                    .expect("accepted ISO extent must extract");
                assert_eq!(count, size);
                remaining -= size;
            }
        }
    }
    let _ = archive_core::udf::UdfArchive::open(data, limits());
}
