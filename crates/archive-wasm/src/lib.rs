//! Browser byte-input facade. Run synchronous operations in a dedicated Worker.
/// Version of this library, as declared in `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

use std::io::Cursor;

use archive_core::{Archive, CreateEntry, EntryId, EntryKind, Format, Limits};
use wasm_bindgen::prelude::*;
mod range;

/// Indexed archive backed by caller-supplied bytes copied into WASM memory.
#[wasm_bindgen]
pub struct ByteArchive {
    archive: Archive<Cursor<Vec<u8>>>,
}

#[derive(serde::Serialize)]
struct EntryMetadata<'a> {
    id: usize,
    name: &'a str,
    raw_name: &'a [u8],
    size: u64,
    compressed_size: Option<u64>,
    compression: &'a str,
    encrypted: bool,
    kind: String,
}

#[cfg(feature = "crypto")]
#[wasm_bindgen]
impl ByteArchive {
    /// Open encrypted ZIP bytes. Password bytes are never formatted.
    pub fn with_password(
        bytes: &[u8],
        password: &[u8],
        maximum: u64,
    ) -> Result<ByteArchive, JsValue> {
        if bytes.len() as u64 > maximum {
            return Err(JsValue::from_str("input byte budget exceeded"));
        }
        let limits = Limits {
            max_input_bytes: maximum,
            max_entry_bytes: maximum,
            max_total_bytes: maximum,
            ..Limits::default()
        };
        let archive = Archive::open_with_password(Cursor::new(bytes.to_vec()), limits, password)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        Ok(Self { archive })
    }
}

#[cfg(feature = "crypto")]
#[wasm_bindgen(
    inline_js = "export function archiveRandom(n) { const bytes = new Uint8Array(n); globalThis.crypto.getRandomValues(bytes); return bytes; }"
)]
extern "C" {
    #[wasm_bindgen(catch, js_name = archiveRandom)]
    fn browser_random(length: usize) -> Result<Vec<u8>, JsValue>;
}

#[cfg(feature = "crypto")]
struct BrowserRandom;
#[cfg(feature = "crypto")]
impl archive_core::RandomSource for BrowserRandom {
    fn fill(&mut self, output: &mut [u8]) -> archive_core::Result<()> {
        let bytes = browser_random(output.len()).map_err(|_| {
            archive_core::Error::Unsupported("Web Crypto randomness unavailable".into())
        })?;
        if bytes.len() != output.len() {
            return Err(archive_core::Error::Malformed(
                "randomness provider length".into(),
            ));
        }
        output.copy_from_slice(&bytes);
        Ok(())
    }
}

/// Create AES-256 ZIP with fresh Web Crypto randomness in a secure browser context.
#[cfg(feature = "crypto")]
#[wasm_bindgen]
pub fn create_encrypted_file(
    name: &str,
    data: &[u8],
    password: &[u8],
    maximum: u64,
) -> Result<Vec<u8>, JsValue> {
    create_encrypted_archive_file("zip", name, data, password, maximum, false)
}

/// Create encrypted ZIP or 7z bytes with an explicit encrypted-header choice.
#[cfg(feature = "crypto")]
#[wasm_bindgen]
pub fn create_encrypted_archive_file(
    format: &str,
    name: &str,
    data: &[u8],
    password: &[u8],
    maximum: u64,
    encrypt_headers: bool,
) -> Result<Vec<u8>, JsValue> {
    let format = match format {
        "zip" => Format::Zip,
        "7z" => Format::SevenZip,
        _ => return Err(JsValue::from_str("unsupported encrypted format")),
    };
    if data.len() as u64 > maximum {
        return Err(JsValue::from_str("decoded byte budget exceeded"));
    }
    let entry = CreateEntry {
        name: name.to_owned(),
        data: data.to_vec(),
        kind: EntryKind::File,
    };
    let limits = Limits {
        max_entry_bytes: maximum,
        max_total_bytes: maximum,
        ..Limits::default()
    };
    let mut output = Cursor::new(Vec::new());
    let mut randomness = BrowserRandom;
    archive_core::create_with_options(
        format,
        &[entry],
        &mut output,
        limits,
        archive_core::CreateOptions {
            password: Some(password),
            randomness: Some(&mut randomness),
            encrypt_headers,
            ..Default::default()
        },
    )
    .map_err(|error| JsValue::from_str(&error.to_string()))?;
    Ok(output.into_inner())
}

#[wasm_bindgen]
impl ByteArchive {
    /// Open a small archive with explicit input and decoded-byte budgets.
    #[wasm_bindgen(constructor)]
    pub fn new(
        bytes: &[u8],
        max_input_bytes: u64,
        max_decoded_bytes: u64,
    ) -> Result<Self, JsValue> {
        if bytes.len() as u64 > max_input_bytes {
            return Err(JsValue::from_str("input byte budget exceeded"));
        }
        let limits = Limits {
            max_input_bytes,
            max_entry_bytes: max_decoded_bytes,
            max_total_bytes: max_decoded_bytes,
            ..Limits::default()
        };
        let archive = Archive::open(Cursor::new(bytes.to_vec()), limits)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        Ok(Self { archive })
    }

    /// Open with an explicit interpretation, including headerless single-stream formats.
    pub fn with_format(bytes: &[u8], format: &str, maximum: u64) -> Result<ByteArchive, JsValue> {
        if bytes.len() as u64 > maximum {
            return Err(JsValue::from_str("input byte budget exceeded"));
        }
        let selected = match format {
            "gzip" | "gz" => Format::Gzip,
            "zlib" => Format::Zlib,
            "deflate" => Format::Deflate,
            "lzma" => Format::Lzma,
            "bzip2" | "bz2" => Format::Bzip2,
            "brotli" | "br" => Format::Brotli,
            "tar.bz2" | "tbz2" => Format::TarBzip2,
            "tar.br" => Format::TarBrotli,
            "xz" => Format::Xz,
            "tar" => Format::Tar,
            "tar.gz" => Format::TarGzip,
            "tar.xz" => Format::TarXz,
            "zip" => Format::Zip,
            "7z" => Format::SevenZip,
            "cab" => Format::Cab,
            "iso" => Format::Iso,
            _ => return Err(JsValue::from_str("unsupported interpretation")),
        };
        let limits = Limits {
            max_input_bytes: maximum,
            max_entry_bytes: maximum,
            max_total_bytes: maximum,
            ..Limits::default()
        };
        let archive = Archive::open_as(Cursor::new(bytes.to_vec()), selected, limits)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        Ok(Self { archive })
    }

    /// Return metadata as JSON without decoding payloads.
    pub fn entries_json(&self) -> Result<String, JsValue> {
        let entries: Vec<_> = self
            .archive
            .entries()
            .iter()
            .map(|entry| EntryMetadata {
                id: entry.id.0,
                name: &entry.name,
                raw_name: &entry.raw_name,
                size: entry.size,
                compressed_size: entry.compressed_size,
                compression: &entry.compression,
                encrypted: entry.encrypted,
                kind: format!("{:?}", entry.kind),
            })
            .collect();
        serde_json::to_string(&entries).map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Decode one entry with an explicit allocation ceiling.
    pub fn read_entry(&mut self, id: usize, maximum: u64) -> Result<Vec<u8>, JsValue> {
        self.archive
            .read_entry(EntryId(id), maximum)
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Verify every supported entry before reporting success.
    pub fn test(&mut self) -> Result<(), JsValue> {
        self.archive
            .test()
            .map(|_| ())
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }
}

/// Create a one-file archive without accessing a host filesystem.
#[wasm_bindgen]
pub fn create_file(
    format: &str,
    name: &str,
    data: &[u8],
    maximum: u64,
) -> Result<Vec<u8>, JsValue> {
    if data.len() as u64 > maximum {
        return Err(JsValue::from_str("decoded byte budget exceeded"));
    }
    let format = match format {
        "zip" => Format::Zip,
        "tar" => Format::Tar,
        "tar.gz" | "tgz" => Format::TarGzip,
        "cab" => Format::Cab,
        "7z" => Format::SevenZip,
        "xz" => Format::Xz,
        "tar.xz" => Format::TarXz,
        "gzip" | "gz" => Format::Gzip,
        "zlib" => Format::Zlib,
        "deflate" => Format::Deflate,
        "lzma" => Format::Lzma,
        "bzip2" | "bz2" => Format::Bzip2,
        "brotli" | "br" => Format::Brotli,
        "tar.bz2" | "tbz2" => Format::TarBzip2,
        "tar.br" => Format::TarBrotli,
        _ => return Err(JsValue::from_str("unsupported creation format")),
    };
    let entry = CreateEntry {
        name: name.to_owned(),
        data: data.to_vec(),
        kind: EntryKind::File,
    };
    let limits = Limits {
        max_entry_bytes: maximum,
        max_total_bytes: maximum,
        ..Limits::default()
    };
    let mut output = Cursor::new(Vec::new());
    archive_core::create(format, &[entry], &mut output, limits)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    Ok(output.into_inner())
}

/// Create a one-file 7z archive using an explicitly selected codec.
#[cfg(feature = "sevenz")]
#[wasm_bindgen]
pub fn create_sevenz_file(
    name: &str,
    data: &[u8],
    compression: &str,
    maximum: u64,
) -> Result<Vec<u8>, JsValue> {
    use archive_core::{CreateOptions, SevenZipCompression};
    if data.len() as u64 > maximum {
        return Err(JsValue::from_str("decoded byte budget exceeded"));
    }
    let compression = match compression {
        "copy" => SevenZipCompression::Copy,
        "lzma" => SevenZipCompression::Lzma,
        "lzma2" => SevenZipCompression::Lzma2,
        "bzip2" => SevenZipCompression::Bzip2,
        "brotli" => SevenZipCompression::Brotli,
        _ => return Err(JsValue::from_str("unsupported 7z compression")),
    };
    let entry = CreateEntry {
        name: name.to_owned(),
        data: data.to_vec(),
        kind: EntryKind::File,
    };
    let limits = Limits {
        max_entry_bytes: maximum,
        max_total_bytes: maximum,
        ..Limits::default()
    };
    let mut output = Cursor::new(Vec::new());
    archive_core::create_with_options(
        Format::SevenZip,
        &[entry],
        &mut output,
        limits,
        CreateOptions {
            sevenz_compression: compression,
            ..CreateOptions::default()
        },
    )
    .map_err(|error| JsValue::from_str(&error.to_string()))?;
    Ok(output.into_inner())
}

/// Read-only APPX/MSIX facade. Integrity does not imply publisher trust.
#[cfg(feature = "packages")]
#[wasm_bindgen]
pub struct BytePackage {
    package: ms_package::AppxPackage<Cursor<Vec<u8>>>,
    maximum: u64,
}

#[cfg(feature = "packages")]
#[wasm_bindgen]
impl BytePackage {
    /// Open a single package with explicit input and decoded-byte limits.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8], maximum: u64) -> Result<Self, JsValue> {
        if bytes.len() as u64 > maximum {
            return Err(JsValue::from_str("package input limit exceeded"));
        }
        let limits = Limits {
            max_input_bytes: maximum,
            max_entry_bytes: maximum,
            max_total_bytes: maximum,
            ..Limits::default()
        };
        let package = ms_package::AppxPackage::open(
            Cursor::new(bytes.to_vec()),
            limits,
            maximum.min(16 << 20),
        )
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
        Ok(Self { package, maximum })
    }

    /// Original manifest bytes including namespace declarations.
    pub fn manifest(&self) -> Vec<u8> {
        self.package.manifest().to_vec()
    }

    /// List package entries without claiming integrity.
    pub fn entries_json(&self) -> Result<String, JsValue> {
        serde_json::to_string(self.package.entries())
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Validate decoded block hashes and complete payload coverage.
    pub fn validate(&mut self) -> Result<(), JsValue> {
        self.package
            .validate(self.maximum)
            .map(|_| ())
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Validate the package before returning selected payload bytes.
    pub fn read_entry(&mut self, id: usize, maximum: u64) -> Result<Vec<u8>, JsValue> {
        self.validate()?;
        self.package
            .read_entry(EntryId(id), maximum.min(self.maximum))
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }
}

/// Read-only bundle reader with explicit nested-package selection.
#[cfg(feature = "packages")]
#[wasm_bindgen]
pub struct ByteBundle {
    bundle: ms_package::AppxBundle<Cursor<Vec<u8>>>,
    maximum: u64,
}

#[cfg(feature = "packages")]
#[wasm_bindgen]
impl ByteBundle {
    /// Open a bounded bundle without choosing a package implicitly.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8], maximum: u64) -> Result<Self, JsValue> {
        if bytes.len() as u64 > maximum {
            return Err(JsValue::from_str("bundle input limit exceeded"));
        }
        let limits = Limits {
            max_input_bytes: maximum,
            max_entry_bytes: maximum,
            max_total_bytes: maximum,
            ..Limits::default()
        };
        let bundle = ms_package::AppxBundle::open(
            Cursor::new(bytes.to_vec()),
            limits,
            maximum.min(16 << 20),
        )
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
        Ok(Self { bundle, maximum })
    }

    /// Preserve original bundle metadata bytes.
    pub fn manifest(&self) -> Vec<u8> {
        self.bundle.manifest().to_vec()
    }

    /// List declared identities, without claiming signature trust.
    pub fn packages_json(&self) -> Result<String, JsValue> {
        let packages: Vec<_> = self
            .bundle
            .packages()
            .iter()
            .map(|package| {
                serde_json::json!({
                    "file_name": package.file_name,
                    "architecture": package.architecture,
                    "resource_id": package.resource_id,
                    "package_type": package.package_type,
                    "size": package.size,
                })
            })
            .collect();
        serde_json::to_string(&packages).map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Open exactly one declared member; validate it before reading payloads.
    pub fn select(&mut self, file_name: &str, maximum: u64) -> Result<BytePackage, JsValue> {
        self.validate()?;
        let maximum = maximum.min(self.maximum);
        let limits = Limits {
            max_input_bytes: maximum,
            max_entry_bytes: maximum,
            max_total_bytes: maximum,
            ..Limits::default()
        };
        let package = self
            .bundle
            .select(file_name, limits, maximum, maximum.min(16 << 20))
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        Ok(BytePackage { package, maximum })
    }

    /// Verify outer block-map coverage; nested packages require their own validation.
    pub fn validate(&mut self) -> Result<(), JsValue> {
        self.bundle
            .validate(self.maximum)
            .map(|_| ())
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }
}

/// Portable MSI reader with explicitly supplied, bounded external media.
#[cfg(feature = "packages")]
#[wasm_bindgen]
pub struct ByteInstaller {
    package: ms_package::InstallerPackage<Cursor<Vec<u8>>>,
    maximum: u64,
    media: std::collections::BTreeMap<String, Vec<u8>>,
    media_bytes: u64,
}

#[cfg(feature = "packages")]
struct CallerMedia<'a>(&'a std::collections::BTreeMap<String, Vec<u8>>);
#[cfg(feature = "packages")]
impl ms_package::MediaResolver for CallerMedia<'_> {
    fn resolve(&mut self, name: &str, maximum: u64) -> ms_package::Result<Vec<u8>> {
        let bytes = self
            .0
            .get(name)
            .ok_or_else(|| ms_package::Error::MissingMedia(name.to_owned()))?;
        if bytes.len() as u64 > maximum {
            return Err(ms_package::Error::Limit("external media bytes"));
        }
        Ok(bytes.clone())
    }
}

#[cfg(feature = "packages")]
#[wasm_bindgen]
impl ByteInstaller {
    /// Open bounded compound storage with a maximum database row count.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8], maximum: u64, max_rows: usize) -> Result<Self, JsValue> {
        if bytes.len() as u64 > maximum {
            return Err(JsValue::from_str("MSI input limit exceeded"));
        }
        let package = ms_package::InstallerPackage::open_bounded(
            Cursor::new(bytes.to_vec()),
            max_rows,
            maximum,
        )
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
        Ok(Self {
            package,
            maximum,
            media: Default::default(),
            media_bytes: 0,
        })
    }

    /// Supply an exact media name; no implicit path or network lookup occurs.
    pub fn provide_media(&mut self, name: &str, bytes: &[u8]) -> Result<(), JsValue> {
        if name.is_empty() || name.len() > 4096 || name.contains('\0') {
            return Err(JsValue::from_str("invalid media name"));
        }
        if self.media.contains_key(name) {
            return Err(JsValue::from_str("duplicate media name"));
        }
        if self.media.len() >= 64 {
            return Err(JsValue::from_str("media count limit exceeded"));
        }
        let total = self
            .media_bytes
            .checked_add(bytes.len() as u64)
            .filter(|total| *total <= self.maximum)
            .ok_or_else(|| JsValue::from_str("external media byte limit exceeded"))?;
        self.media.insert(name.to_owned(), bytes.to_vec());
        self.media_bytes = total;
        Ok(())
    }

    /// Release all caller-supplied media without changing the original MSI.
    pub fn clear_media(&mut self) {
        self.media.clear();
        self.media_bytes = 0;
    }

    /// Table names, separate from archive streams and payloads.
    pub fn tables_json(&self) -> Result<String, JsValue> {
        serde_json::to_string(&self.package.tables())
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Declared payload paths; installer conditions are not evaluated.
    pub fn files_json(&mut self) -> Result<String, JsValue> {
        let files = self
            .package
            .files()
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let files: Vec<_> = files
            .iter()
            .map(|file| {
                serde_json::json!({
                    "id": file.id, "path": file.path, "size": file.size, "cabinet": file.cabinet,
                })
            })
            .collect();
        serde_json::to_string(&files).map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Extract an embedded or explicitly supplied payload by File table identity.
    pub fn read_file(&mut self, id: &str, maximum: u64) -> Result<Vec<u8>, JsValue> {
        let files = self
            .package
            .files()
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let file = files
            .iter()
            .find(|file| file.id == id)
            .ok_or_else(|| JsValue::from_str("unknown MSI file ID"))?;
        self.package
            .read_file(
                file,
                &mut CallerMedia(&self.media),
                maximum.min(self.maximum),
            )
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }
}
