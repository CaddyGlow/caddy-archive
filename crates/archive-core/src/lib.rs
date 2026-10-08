//! Portable, bounded archive operations on caller-owned I/O.
/// Version of this library, as declared in `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod incremental;
pub mod progress;
#[path = "io.rs"]
pub mod range;
#[cfg(feature = "tar")]
pub mod sequential_tar;
#[cfg(feature = "sevenz")]
mod sevenz_backend;
#[cfg(feature = "streams")]
pub mod single_stream;
#[cfg(feature = "udf")]
pub mod udf;
#[cfg(feature = "wim")]
pub mod wim;
#[cfg(feature = "xz")]
mod xz_backend;
use serde::Serialize;
use std::io::{self, Read, Seek, SeekFrom, Write};

#[cfg(any(
    feature = "zip",
    feature = "gzip",
    feature = "streams",
    feature = "sevenz"
))]
mod codec;
#[cfg(any(feature = "gzip", feature = "streams"))]
mod stream_backend;
#[cfg(any(feature = "gzip", feature = "streams"))]
pub use stream_backend::GzipHeader;
#[cfg(feature = "brotli")]
mod brotli_budget;
#[cfg(all(feature = "crypto", feature = "zip"))]
mod crypto;
#[cfg(feature = "iso")]
mod iso_backend;
/// Supplies cryptographically secure bytes; callers own native/browser integration.
pub trait RandomSource {
    fn fill(&mut self, output: &mut [u8]) -> Result<()>;
}
#[derive(Default)]
pub struct CreateOptions<'a> {
    pub password: Option<&'a [u8]>,
    pub randomness: Option<&'a mut dyn RandomSource>,
    pub encrypt_headers: bool,
    pub zip_encryption: ZipEncryption,
    pub sevenz_compression: SevenZipCompression,
    pub zip_compression: ZipCompression,
    pub cab_compression: CabCompression,
    /// Source metadata in the same order as the creation entries.
    pub entry_metadata: Option<&'a [EntryMetadata]>,
}
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SevenZipCompression {
    Copy,
    Deflate,
    Lzma,
    #[default]
    Lzma2,
    Bzip2,
    Brotli,
}
/// Codec used for ZIP creation.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZipCompression {
    Copy,
    #[default]
    Deflate,
}
/// Codec used for CAB creation. LZX/Quantum use a 2 MiB window.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CabCompression {
    Copy,
    #[default]
    MsZip,
    Lzx,
    Quantum,
}
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZipEncryption {
    #[default]
    Aes256,
    ZipCrypto,
}
#[cfg(feature = "tar")]
mod tar_backend;
#[cfg(feature = "zip")]
mod zip_backend;

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O: {0}")]
    Io(#[from] io::Error),
    #[error("malformed archive: {0}")]
    Malformed(String),
    #[error("unsupported feature: {0}")]
    Unsupported(String),
    #[error("password required")]
    PasswordRequired,
    #[error("integrity failure: {0}")]
    Integrity(String),
    #[error("resource limit exceeded: {0}")]
    ResourceLimit(&'static str),
    #[error("operation cancelled")]
    Cancelled,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Format {
    Zip,
    Tar,
    TarGzip,
    Cab,
    SevenZip,
    Xz,
    TarXz,
    Wim,
    Iso,
    Udf,
    Appx,
    Msix,
    Msi,
    Gzip,
    Zlib,
    Lzma,
    Deflate,
    Bzip2,
    Brotli,
    TarBzip2,
    TarBrotli,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EntryId(pub usize);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum EntryKind {
    File,
    Directory,
    Link,
    Other,
}
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id: EntryId,
    pub raw_name: Vec<u8>,
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
    pub compressed_size: Option<u64>,
    pub compression: String,
    pub encrypted: bool,
}
/// Stored timestamp with explicit timezone semantics; DOS dates are local wall time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum StoredTimestamp {
    UnixSeconds(u64),
    DosLocal {
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    },
}
/// Optional source metadata. Unknown values are never synthesized from defaults.
/// TAR values describe its stored file header, not PAX timestamp overrides.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EntryMetadata {
    pub modified: Option<StoredTimestamp>,
    pub unix_mode: Option<u32>,
    pub user_id: Option<u64>,
    pub group_id: Option<u64>,
    pub link_target: Option<Vec<u8>>,
    pub format: Option<EntryFormatMetadata>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum EntryFormatMetadata {
    Zip {
        crc32: u32,
        compression_method: u16,
        aes_version: Option<u16>,
        aes_strength: Option<u8>,
    },
    Tar {
        stored_type: u8,
    },
}
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Maximum decoded TAR bytes retained by the indexed compressed-TAR API.
    /// Separate from decoder workspace; this is not a process-wide memory cap.
    pub max_buffered_bytes: u64,
    pub max_entries: u64,
    pub max_metadata_bytes: u64,
    pub max_entry_bytes: u64,
    pub max_total_bytes: u64,
    pub max_dictionary_bytes: u64,
    pub max_input_bytes: u64,
    pub max_active_workspace_bytes: u64,
    pub max_pending_output_bytes: u64,
    pub max_workers: usize,
    pub max_password_iterations: u64,
    pub max_nesting_depth: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_buffered_bytes: 256 << 20,
            max_entries: 100_000,
            max_metadata_bytes: 16 << 20,
            max_entry_bytes: 8 << 30,
            max_total_bytes: 32 << 30,
            max_dictionary_bytes: 64 << 20,
            max_input_bytes: 64 << 30,
            max_active_workspace_bytes: 256 << 20,
            max_pending_output_bytes: 4 << 20,
            max_workers: 4,
            max_password_iterations: 1 << 24,
            max_nesting_depth: 64,
        }
    }
}
#[derive(Debug, Clone, Copy, Default, Serialize)]
/// Verification covers declared size and available format checksums, not authenticity.
pub struct ExtractReport {
    pub bytes: u64,
    pub entries: u64,
    pub verified: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct BatchReport {
    pub report: ExtractReport,
    pub workers_used: usize,
    pub folder_tasks: usize,
    pub decoded_bytes: u64,
    pub fallback_reason: Option<String>,
}
struct FnWriter<'a, F> {
    sink: &'a mut F,
    id: EntryId,
    error: Option<Error>,
}
impl<F: FnMut(EntryId, &[u8]) -> Result<()>> Write for FnWriter<'_, F> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match (self.sink)(self.id, bytes) {
            Ok(()) => Ok(bytes.len()),
            Err(error) => {
                self.error = Some(error);
                Err(io::Error::other("archive sink failed"))
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct CreateEntry {
    pub name: String,
    pub data: Vec<u8>,
    pub kind: EntryKind,
}
#[derive(Debug, Clone, Serialize)]
pub struct Capabilities {
    pub format: Format,
    pub read: bool,
    pub write: bool,
    pub seek: bool,
    pub encryption: bool,
    pub solid: bool,
    pub volumes: bool,
    /// Link metadata can be listed; native link materialization is never implied.
    pub links: bool,
    pub browser: bool,
}
pub fn capabilities(format: Format) -> Capabilities {
    let enabled = (format == Format::Zip && cfg!(feature = "zip"))
        || (format == Format::Tar && cfg!(feature = "tar"))
        || (format == Format::TarGzip && cfg!(feature = "gzip"))
        || (format == Format::Gzip && cfg!(any(feature = "gzip", feature = "streams")))
        || (matches!(format, Format::Zlib | Format::Lzma | Format::Deflate)
            && cfg!(feature = "streams"))
        || (matches!(format, Format::Bzip2 | Format::TarBzip2) && cfg!(feature = "bzip2"))
        || (matches!(format, Format::Brotli | Format::TarBrotli) && cfg!(feature = "brotli"))
        || (format == Format::Cab && cfg!(feature = "cab"))
        || (format == Format::Iso && cfg!(feature = "iso"))
        || (matches!(format, Format::Xz | Format::TarXz) && cfg!(feature = "xz"))
        || (format == Format::SevenZip && cfg!(feature = "sevenz"))
        || (format == Format::Wim && cfg!(feature = "wim"))
        || (format == Format::Udf && cfg!(feature = "udf"));
    Capabilities {
        format,
        read: enabled,
        write: enabled && !matches!(format, Format::Iso | Format::Wim | Format::Udf),
        seek: enabled,
        encryption: enabled
            && cfg!(feature = "crypto")
            && matches!(format, Format::Zip | Format::SevenZip),
        solid: enabled && matches!(format, Format::Cab | Format::SevenZip | Format::Wim),
        volumes: false,
        links: enabled
            && matches!(
                format,
                Format::Tar | Format::TarGzip | Format::TarXz | Format::Zip
            ),
        browser: enabled && format != Format::Wim,
    }
}
#[cfg(any(feature = "xz", feature = "gzip", feature = "streams"))]
#[derive(Default)]
struct Prefix {
    bytes: Vec<u8>,
}
#[cfg(any(feature = "xz", feature = "gzip", feature = "streams"))]
impl Write for Prefix {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let n = (512 - self.bytes.len()).min(bytes.len());
        self.bytes.extend_from_slice(&bytes[..n]);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
enum Backend<R> {
    #[cfg(feature = "zip")]
    Zip(R, Vec<zip_backend::Location>),
    #[cfg(feature = "tar")]
    Tar(R, Vec<u64>),
    #[cfg(feature = "gzip")]
    TarGzip(Vec<u8>, Vec<u64>),
    #[cfg(all(feature = "streams", feature = "tar"))]
    CompressedTar(Vec<u8>, Vec<u64>),
    #[cfg(feature = "cab")]
    Cab(cabinet::Cabinet<R>),
    #[cfg(feature = "iso")]
    Iso(R, Vec<Vec<libmkiso::iso9660::Extent>>),
    #[cfg(feature = "xz")]
    Xz(R),
    #[cfg(feature = "xz")]
    TarXz(Vec<u8>, Vec<u64>),
    #[cfg(feature = "sevenz")]
    SevenZip(Box<sevenz_backend::SevenZip<R>>),
    #[cfg(any(feature = "gzip", feature = "streams"))]
    Stream(R, Format),
    #[allow(dead_code)]
    Unavailable(std::marker::PhantomData<R>),
}
pub struct Archive<R> {
    backend: Backend<R>,
    entries: Vec<Entry>,
    limits: Limits,
    format: Format,
    detected_format: Option<Format>,
    #[cfg(any(feature = "gzip", feature = "streams"))]
    gzip_header: Option<GzipHeader>,
    #[cfg(feature = "crypto")]
    password: Option<zeroize::Zeroizing<Vec<u8>>>,
}
impl<R: Read + Seek> Archive<R> {
    /// Open with caller-owned seekable scratch storage for decoded compressed TAR.
    /// Supply an empty temporary file to avoid retaining the decoded TAR in RAM.
    /// Scratch is provisional, must start empty, and is owned by the returned
    /// archive when used. Output limits still bound the spool's disk consumption.
    /// Other formats use their normal backend (including its memory policy).
    pub fn open_with_scratch(
        mut reader: R,
        mut scratch: R,
        limits: Limits,
        requested: Option<Format>,
        password: Option<&[u8]>,
    ) -> Result<Self>
    where
        R: Write,
    {
        if scratch.seek(SeekFrom::End(0))? != 0 {
            return Err(Error::Malformed("scratch storage must be empty".into()));
        }
        let length = reader.seek(SeekFrom::End(0))?;
        if length > limits.max_input_bytes {
            return Err(Error::ResourceLimit("input bytes"));
        }
        reader.rewind()?;
        let mut signature = [0; 512];
        let count = length.min(signature.len() as u64) as usize;
        reader.read_exact(&mut signature[..count])?;
        reader.rewind()?;
        let detected = probe(&signature[..count]).ok();
        let format = requested.or(detected);
        #[cfg(any(
            feature = "gzip",
            feature = "xz",
            all(feature = "streams", feature = "tar")
        ))]
        {
            let tar_format = match format {
                #[cfg(feature = "gzip")]
                Some(Format::Gzip | Format::TarGzip) => Some(Format::TarGzip),
                #[cfg(feature = "xz")]
                Some(Format::Xz | Format::TarXz) => Some(Format::TarXz),
                #[cfg(all(feature = "streams", feature = "tar"))]
                Some(Format::Bzip2 | Format::TarBzip2) => Some(Format::TarBzip2),
                #[cfg(all(feature = "streams", feature = "tar"))]
                Some(Format::TarBrotli) => Some(Format::TarBrotli),
                _ => None,
            };
            if let Some(tar_format) = tar_format {
                let explicit_tar = requested == Some(tar_format);
                if explicit_tar || requested.is_none() {
                    fn decode<R: Read + Seek>(
                        reader: &mut R,
                        writer: &mut impl Write,
                        format: Format,
                        limits: Limits,
                    ) -> Result<u64> {
                        match format {
                            #[cfg(feature = "xz")]
                            Format::TarXz => xz_backend::decode(reader, writer, limits),
                            #[cfg(feature = "gzip")]
                            Format::TarGzip => {
                                stream_backend::decode(reader, writer, Format::Gzip, limits)
                            }
                            #[cfg(all(feature = "streams", feature = "tar"))]
                            Format::TarBzip2 => {
                                stream_backend::decode(reader, writer, Format::Bzip2, limits)
                            }
                            #[cfg(all(feature = "streams", feature = "tar"))]
                            Format::TarBrotli => {
                                stream_backend::decode(reader, writer, Format::Brotli, limits)
                            }
                            _ => Err(Error::Unsupported("compressed TAR codec".into())),
                        }
                    }
                    let mut prefix = Prefix::default();
                    if explicit_tar || {
                        decode(&mut reader, &mut prefix, tar_format, limits)?;
                        probe(&prefix.bytes).ok() == Some(Format::Tar)
                    } {
                        if password.is_some() {
                            return Err(Error::Unsupported("compressed TAR encryption".into()));
                        }
                        let decoded = decode(&mut reader, &mut scratch, tar_format, limits)?;
                        scratch.flush()?;
                        scratch.rewind()?;
                        let mut archive = Self::open_inner(
                            scratch,
                            Limits {
                                max_input_bytes: decoded,
                                ..limits
                            },
                            None,
                            Some(Format::Tar),
                        )?;
                        archive.limits = limits;
                        archive.format = tar_format;
                        archive.detected_format = detected;
                        #[cfg(any(feature = "gzip", feature = "streams"))]
                        if tar_format == Format::TarGzip {
                            archive.gzip_header =
                                Some(stream_backend::gzip_header(&mut reader, limits)?);
                        }
                        return Ok(archive);
                    }
                }
            }
        }
        let _ = format;
        if let Some(password) = password {
            if requested.is_some() {
                return Err(Error::Unsupported(
                    "explicit interpretation with password".into(),
                ));
            }
            Self::open_with_password(reader, limits, password)
        } else {
            Self::open_inner(reader, limits, None, requested)
        }
    }
    pub fn open(reader: R, limits: Limits) -> Result<Self> {
        Self::open_inner(reader, limits, None, None)
    }
    pub fn open_as(reader: R, format: Format, limits: Limits) -> Result<Self> {
        Self::open_inner(reader, limits, None, Some(format))
    }
    fn open_inner(
        mut reader: R,
        limits: Limits,
        password: Option<&[u8]>,
        requested: Option<Format>,
    ) -> Result<Self> {
        let _ = password;
        let length = reader.seek(SeekFrom::End(0))?;
        if length > limits.max_input_bytes {
            return Err(Error::ResourceLimit("input bytes"));
        }
        reader.rewind()?;
        let mut sig = [0u8; 512];
        let count = length.min(sig.len() as u64) as usize;
        reader.read_exact(&mut sig[..count])?;
        reader.rewind()?;
        let mut detected = probe(&sig[..count]);
        if length >= 32775 && (detected.is_err() || sig[..count].iter().all(|b| *b == 0)) {
            reader.seek(SeekFrom::Start(32768))?;
            let mut descriptor = [0u8; 7];
            reader.read_exact(&mut descriptor)?;
            if descriptor[1..6] == *b"CD001" {
                detected = Ok(Format::Iso);
            }
            reader.rewind()?;
        }
        let detected_format = detected.as_ref().ok().copied();
        #[allow(unused_mut)]
        let mut format = match requested {
            Some(format) => format,
            None => detected?,
        };
        #[cfg(any(feature = "gzip", feature = "streams"))]
        let mut gzip_header = None;
        #[allow(unreachable_code, unused_variables)]
        let (backend, entries): (Backend<R>, Vec<Entry>) = match format {
            #[cfg(feature = "zip")]
            Format::Zip => {
                let (r, e, l) = zip_backend::index(reader, limits)?;
                (Backend::Zip(r, l), e)
            }
            #[cfg(feature = "tar")]
            Format::Tar => {
                let (e, l) = tar_backend::index(&mut reader, limits)?;
                (Backend::Tar(reader, l), e)
            }
            #[cfg(feature = "iso")]
            Format::Iso => {
                let (e, l) = iso_backend::index(&mut reader, limits)?;
                (Backend::Iso(reader, l), e)
            }
            #[cfg(any(feature = "gzip", feature = "streams"))]
            Format::Gzip | Format::TarGzip => {
                let header = stream_backend::gzip_header(&mut reader, limits)?;
                let mut prefix = Prefix::default();
                let size = stream_backend::decode(&mut reader, &mut prefix, Format::Gzip, limits)?;
                gzip_header = Some(header.clone());
                #[cfg(feature = "gzip")]
                if requested != Some(Format::Gzip)
                    && (requested == Some(Format::TarGzip)
                        || probe(&prefix.bytes).ok() == Some(Format::Tar))
                {
                    let mut bytes = crate::range::BoundedBuffer::new(limits.max_buffered_bytes);
                    stream_backend::decode(&mut reader, &mut bytes, Format::Gzip, limits)?;
                    let bytes = bytes.into_inner();
                    let (e, l) = tar_backend::index(&mut io::Cursor::new(&bytes), limits)?;
                    format = Format::TarGzip;
                    validate_index(&e, limits)?;
                    return Ok(Self {
                        backend: Backend::TarGzip(bytes, l),
                        entries: e,
                        limits,
                        format,
                        detected_format,
                        gzip_header,
                        #[cfg(feature = "crypto")]
                        password: None,
                    });
                }
                format = Format::Gzip;
                let name = if header.original_name.is_empty() {
                    b"data".to_vec()
                } else {
                    header.original_name
                };
                (
                    Backend::Stream(reader, Format::Gzip),
                    vec![Entry {
                        id: EntryId(0),
                        name: String::from_utf8_lossy(&name).into_owned(),
                        raw_name: name,
                        kind: EntryKind::File,
                        size,
                        compressed_size: Some(length),
                        compression: "DEFLATE".into(),
                        encrypted: false,
                    }],
                )
            }
            #[cfg(feature = "streams")]
            Format::Zlib
            | Format::Lzma
            | Format::Deflate
            | Format::Bzip2
            | Format::Brotli
            | Format::TarBzip2
            | Format::TarBrotli => {
                #[cfg(feature = "tar")]
                {
                    let mut detected_size = None;
                    let detected_tar = if requested.is_none() && format == Format::Bzip2 {
                        let mut prefix = Prefix::default();
                        detected_size = Some(stream_backend::decode(
                            &mut reader,
                            &mut prefix,
                            format,
                            limits,
                        )?);
                        probe(&prefix.bytes).ok() == Some(Format::Tar)
                    } else {
                        false
                    };
                    if detected_tar || matches!(format, Format::TarBzip2 | Format::TarBrotli) {
                        if detected_tar {
                            format = Format::TarBzip2;
                        }
                        let raw = if format == Format::TarBzip2 {
                            Format::Bzip2
                        } else {
                            Format::Brotli
                        };
                        let mut bytes = crate::range::BoundedBuffer::new(limits.max_buffered_bytes);
                        stream_backend::decode(&mut reader, &mut bytes, raw, limits)?;
                        let bytes = bytes.into_inner();
                        let (entries, locations) =
                            tar_backend::index(&mut io::Cursor::new(&bytes), limits)?;
                        (Backend::CompressedTar(bytes, locations), entries)
                    } else {
                        let size = match detected_size {
                            Some(size) => size,
                            None => stream_backend::decode(
                                &mut reader,
                                &mut io::sink(),
                                format,
                                limits,
                            )?,
                        };
                        (
                            Backend::Stream(reader, format),
                            vec![Entry {
                                id: EntryId(0),
                                name: "data".into(),
                                raw_name: b"data".to_vec(),
                                kind: EntryKind::File,
                                size,
                                compressed_size: Some(length),
                                compression: format!("{format:?}"),
                                encrypted: false,
                            }],
                        )
                    }
                }
                #[cfg(not(feature = "tar"))]
                {
                    let size =
                        stream_backend::decode(&mut reader, &mut io::sink(), format, limits)?;
                    (
                        Backend::Stream(reader, format),
                        vec![Entry {
                            id: EntryId(0),
                            name: "data".into(),
                            raw_name: b"data".to_vec(),
                            kind: EntryKind::File,
                            size,
                            compressed_size: Some(length),
                            compression: format!("{format:?}"),
                            encrypted: false,
                        }],
                    )
                }
            }
            #[cfg(feature = "xz")]
            Format::Xz | Format::TarXz => {
                let mut prefix = Prefix::default();
                let size = xz_backend::decode(&mut reader, &mut prefix, limits)?;
                if requested != Some(Format::Xz)
                    && (requested == Some(Format::TarXz)
                        || probe(&prefix.bytes).ok() == Some(Format::Tar))
                {
                    let mut decoded = crate::range::BoundedBuffer::new(limits.max_buffered_bytes);
                    xz_backend::decode(&mut reader, &mut decoded, limits)?;
                    let decoded = decoded.into_inner();
                    let (e, l) = tar_backend::index(&mut io::Cursor::new(&decoded), limits)?;
                    format = Format::TarXz;
                    (Backend::TarXz(decoded, l), e)
                } else {
                    (
                        Backend::Xz(reader),
                        vec![Entry {
                            id: EntryId(0),
                            name: "data".into(),
                            raw_name: b"data".to_vec(),
                            kind: EntryKind::File,
                            size,
                            compressed_size: Some(length),
                            compression: "LZMA2".into(),
                            encrypted: false,
                        }],
                    )
                }
            }
            #[cfg(feature = "sevenz")]
            Format::SevenZip => {
                let (backend, entries) = sevenz_backend::index(reader, limits, password)?;
                (Backend::SevenZip(Box::new(backend)), entries)
            }
            #[cfg(feature = "cab")]
            Format::Cab => {
                cab_preflight(&mut reader, limits)?;
                let cabinet = cabinet::Cabinet::new(reader)?;
                let e = cabinet
                    .entries()
                    .iter()
                    .enumerate()
                    .map(|(i, e)| Entry {
                        id: EntryId(i),
                        raw_name: e.name.as_bytes().to_vec(),
                        name: e.name.clone(),
                        kind: EntryKind::File,
                        size: u64::from(e.size),
                        compressed_size: None,
                        compression: format!("{:?}", e.compression),
                        encrypted: false,
                    })
                    .collect();
                (Backend::Cab(cabinet), e)
            }
            _ => Err::<(Backend<R>, Vec<Entry>), _>(Error::Unsupported(format!(
                "{format:?} backend"
            )))?,
        };
        validate_index(&entries, limits)?;
        Ok(Self {
            backend,
            entries,
            limits,
            format,
            detected_format,
            #[cfg(any(feature = "gzip", feature = "streams"))]
            gzip_header,
            #[cfg(feature = "crypto")]
            password: None,
        })
    }
    pub fn format(&self) -> Format {
        self.format
    }
    pub fn detected_format(&self) -> Option<Format> {
        self.detected_format
    }
    #[cfg(any(feature = "gzip", feature = "streams"))]
    pub fn gzip_header(&self) -> Option<&GzipHeader> {
        self.gzip_header.as_ref()
    }
    pub fn open_with_password(reader: R, limits: Limits, password: &[u8]) -> Result<Self> {
        if password.len() > 1 << 20 {
            return Err(Error::ResourceLimit("password bytes"));
        }
        let archive = Self::open_inner(reader, limits, Some(password), None)?;
        #[cfg(feature = "crypto")]
        {
            let mut archive = archive;
            if password.len() > 1 << 20 {
                return Err(Error::ResourceLimit("password bytes"));
            }
            archive.password = Some(zeroize::Zeroizing::new(password.to_vec()));
            Ok(archive)
        }
        #[cfg(not(feature = "crypto"))]
        {
            let _ = (archive, password);
            Err(Error::Unsupported("crypto feature unavailable".into()))
        }
    }
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    /// Read bounded source metadata without decoding entry payloads.
    /// This may reposition the seekable reader; extraction remains independent.
    pub fn entry_metadata(&mut self, id: EntryId) -> Result<EntryMetadata> {
        if id.0 >= self.entries.len() {
            return Err(Error::Malformed("unknown entry ID".into()));
        }
        match &mut self.backend {
            #[cfg(feature = "sevenz")]
            Backend::SevenZip(backend) => sevenz_backend::metadata(backend, id),
            #[cfg(any(feature = "gzip", feature = "streams"))]
            Backend::Stream(_, Format::Gzip) => Ok(EntryMetadata {
                modified: self
                    .gzip_header
                    .as_ref()
                    .filter(|header| header.modified_unix_seconds != 0)
                    .map(|header| {
                        StoredTimestamp::UnixSeconds(u64::from(header.modified_unix_seconds))
                    }),
                ..Default::default()
            }),
            #[cfg(feature = "cab")]
            Backend::Cab(cabinet) => {
                let entry = cabinet
                    .entries()
                    .get(id.0)
                    .ok_or_else(|| Error::Malformed("unknown CAB entry ID".into()))?;
                Ok(EntryMetadata {
                    modified: if entry.dos_date & 31 != 0 && (entry.dos_date >> 5) & 15 != 0 {
                        Some(StoredTimestamp::DosLocal {
                            year: 1980 + (entry.dos_date >> 9),
                            month: ((entry.dos_date >> 5) & 15) as u8,
                            day: (entry.dos_date & 31) as u8,
                            hour: (entry.dos_time >> 11) as u8,
                            minute: ((entry.dos_time >> 5) & 63) as u8,
                            second: ((entry.dos_time & 31) * 2) as u8,
                        })
                    } else {
                        None
                    },
                    unix_mode: Some(if entry.attributes & 1 != 0 {
                        0o444
                    } else {
                        0o644
                    }),
                    ..Default::default()
                })
            }
            #[cfg(feature = "zip")]
            Backend::Zip(_, locations) => Ok(locations[id.0].metadata.clone()),
            #[cfg(feature = "tar")]
            Backend::Tar(reader, locations) => tar_backend::metadata(reader, locations[id.0]),
            #[cfg(feature = "gzip")]
            Backend::TarGzip(data, locations) => {
                tar_backend::metadata(&mut io::Cursor::new(data), locations[id.0])
            }
            #[cfg(feature = "xz")]
            Backend::TarXz(data, locations) => {
                tar_backend::metadata(&mut io::Cursor::new(data), locations[id.0])
            }
            #[cfg(all(feature = "streams", feature = "tar"))]
            Backend::CompressedTar(data, locations) => {
                tar_backend::metadata(&mut io::Cursor::new(data), locations[id.0])
            }
            _ => Ok(EntryMetadata::default()),
        }
    }
    /// Routes provisional payload chunks by ID, decoding shared 7z folders once.
    pub fn extract_selected(
        &mut self,
        ids: &[EntryId],
        sink: &mut impl FnMut(EntryId, &[u8]) -> Result<()>,
    ) -> Result<ExtractReport> {
        let mut selected = std::collections::BTreeSet::new();
        let mut total = 0u64;
        for id in ids {
            let entry = self
                .entries
                .get(id.0)
                .ok_or_else(|| Error::Malformed("unknown entry ID".into()))?;
            if !selected.insert(id.0) {
                return Err(Error::Malformed("duplicate selected entry ID".into()));
            }
            total = total
                .checked_add(entry.size)
                .ok_or(Error::ResourceLimit("selected bytes"))?;
            if total > self.limits.max_total_bytes {
                return Err(Error::ResourceLimit("selected bytes"));
            }
        }
        #[cfg(feature = "sevenz")]
        if let Backend::SevenZip(backend) = &mut self.backend {
            return sevenz_backend::extract_selected(backend, ids, sink);
        }
        let mut report = ExtractReport {
            verified: true,
            ..Default::default()
        };
        for id in ids {
            let mut writer = FnWriter {
                sink,
                id: *id,
                error: None,
            };
            let result = self.extract(*id, &mut writer);
            if let Some(error) = writer.error {
                return Err(error);
            }
            let result = result?;
            report.bytes = report
                .bytes
                .checked_add(result.bytes)
                .ok_or(Error::ResourceLimit("selected bytes"))?;
            report.entries += result.entries;
        }
        Ok(report)
    }
    /// Runs independent folder jobs with one operation budget and bounded pending chunks.
    pub fn extract_selected_parallel(
        &mut self,
        ids: &[EntryId],
        workers: usize,
        cancelled: &(impl Fn() -> bool + Sync),
        sink: &mut impl FnMut(EntryId, &[u8]) -> Result<()>,
    ) -> Result<BatchReport>
    where
        R: Send,
    {
        if workers == 0 {
            return Err(Error::Malformed("worker count must be positive".into()));
        }
        if cancelled() {
            return Err(Error::Cancelled);
        }
        #[cfg(all(feature = "sevenz", feature = "parallel"))]
        if let Backend::SevenZip(backend) = &mut self.backend {
            return sevenz_backend::extract_selected_parallel(
                backend,
                ids,
                workers,
                self.limits.max_active_workspace_bytes,
                cancelled,
                sink,
            );
        }
        let report = self.extract_selected(ids, &mut |id, bytes| {
            if cancelled() {
                return Err(Error::Cancelled);
            }
            sink(id, bytes)
        })?;
        Ok(BatchReport {
            decoded_bytes: report.bytes,
            report,
            workers_used: 1,
            folder_tasks: 0,
            fallback_reason: Some(
                "selected backend or build has no independent folder workers".into(),
            ),
        })
    }
    /// Bytes delivered before a failure are provisional. Publish only after success.
    pub fn extract(&mut self, id: EntryId, output: &mut impl Write) -> Result<ExtractReport> {
        let entry = self
            .entries
            .get(id.0)
            .ok_or_else(|| Error::Malformed("unknown entry ID".into()))?;
        if entry.size > self.limits.max_entry_bytes {
            return Err(Error::ResourceLimit("entry decoded bytes"));
        }
        if entry.encrypted && !cfg!(feature = "crypto") {
            return Err(Error::PasswordRequired);
        }
        if entry.kind == EntryKind::Directory {
            return Ok(ExtractReport {
                bytes: 0,
                entries: 1,
                verified: true,
            });
        }
        if entry.kind != EntryKind::File {
            return Err(Error::Unsupported("links and special entries".into()));
        }
        let limit = entry.size.min(self.limits.max_entry_bytes);
        let _ = (&output, limit);
        let bytes: u64 = match &mut self.backend {
            #[cfg(feature = "zip")]
            Backend::Zip(reader, loc) => {
                zip_backend::extract(reader, &loc[id.0], output, limit, {
                    #[cfg(feature = "crypto")]
                    {
                        self.password.as_deref().map(|p| p.as_slice())
                    }
                    #[cfg(not(feature = "crypto"))]
                    {
                        None
                    }
                })?
            }
            #[cfg(feature = "tar")]
            Backend::Tar(reader, loc) => {
                reader.seek(SeekFrom::Start(loc[id.0]))?;
                copy_bounded(&mut reader.take(entry.size), output, limit)?
            }
            #[cfg(feature = "gzip")]
            Backend::TarGzip(data, loc) => {
                let mut reader = io::Cursor::new(data);
                reader.seek(SeekFrom::Start(loc[id.0]))?;
                copy_bounded(&mut reader.take(entry.size), output, limit)?
            }
            #[cfg(all(feature = "streams", feature = "tar"))]
            Backend::CompressedTar(data, loc) => {
                let mut reader = io::Cursor::new(data);
                reader.seek(SeekFrom::Start(loc[id.0]))?;
                copy_bounded(&mut reader.take(entry.size), output, limit)?
            }
            #[cfg(feature = "cab")]
            Backend::Cab(cab) => copy_bounded(&mut cab.read_file(&entry.name)?, output, limit)?,
            #[cfg(feature = "iso")]
            Backend::Iso(reader, loc) => {
                let mut bytes = 0;
                for extent in &loc[id.0] {
                    reader.seek(SeekFrom::Start(extent.offset))?;
                    bytes += copy_bounded(
                        &mut reader.take(extent.size),
                        output,
                        limit.saturating_sub(bytes),
                    )?;
                }
                bytes
            }
            #[cfg(feature = "xz")]
            Backend::Xz(reader) => xz_backend::decode(reader, output, self.limits)?,
            #[cfg(feature = "xz")]
            Backend::TarXz(data, loc) => {
                let mut reader = io::Cursor::new(data);
                reader.seek(SeekFrom::Start(loc[id.0]))?;
                copy_bounded(&mut reader.take(entry.size), output, limit)?
            }
            #[cfg(any(feature = "gzip", feature = "streams"))]
            Backend::Stream(reader, format) => {
                stream_backend::decode(reader, output, *format, self.limits)?
            }
            #[cfg(feature = "sevenz")]
            Backend::SevenZip(backend) => sevenz_backend::extract(backend, id, output)?,
            Backend::Unavailable(_) => Err::<u64, _>(Error::Unsupported("backend".into()))?,
        };
        if bytes != entry.size {
            return Err(Error::Integrity("decoded size mismatch".into()));
        }
        Ok(ExtractReport {
            bytes,
            entries: 1,
            verified: true,
        })
    }
    pub fn read_entry(&mut self, id: EntryId, max: u64) -> Result<Vec<u8>> {
        let size = self
            .entries
            .get(id.0)
            .ok_or_else(|| Error::Malformed("unknown entry ID".into()))?
            .size;
        if size > max {
            return Err(Error::ResourceLimit("buffered entry bytes"));
        }
        let mut data = Vec::new();
        self.extract(id, &mut data)?;
        Ok(data)
    }
    /// Extract with coarse selected-output progress. Physical reads and shared
    /// decoder work remain unknown in this adapter rather than inferred from headers.
    pub fn extract_observed<O: progress::Observer>(
        &mut self,
        id: EntryId,
        output: &mut impl Write,
        observer: &mut O,
    ) -> Result<ExtractReport> {
        let total = self.entries.get(id.0).map(|entry| entry.size);
        let mut reporter = progress::Reporter::new(observer, total);
        reporter.stage(progress::Stage::Decoding);
        struct Sink<'a, 'b, W, O: progress::Observer> {
            output: &'a mut W,
            reporter: &'a mut progress::Reporter<'b, O>,
        }
        impl<W: Write, O: progress::Observer> Write for Sink<'_, '_, W, O> {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                let count = self.output.write(bytes)?;
                if O::ENABLED {
                    self.reporter.written(count as u64);
                    self.reporter.publish();
                }
                Ok(count)
            }
            fn flush(&mut self) -> io::Result<()> {
                self.output.flush()
            }
        }
        let result = self.extract(
            id,
            &mut Sink {
                output,
                reporter: &mut reporter,
            },
        );
        match &result {
            Ok(_) => {
                reporter.verified_entry();
                reporter.finish(progress::Stage::Complete);
            }
            Err(Error::Cancelled) => reporter.finish(progress::Stage::Cancelled),
            Err(_) => reporter.finish(progress::Stage::Failed),
        }
        result
    }

    /// Verify an archive while delivering cumulative entry-level snapshots.
    pub fn test_observed<O: progress::Observer>(
        &mut self,
        observer: &mut O,
    ) -> Result<ExtractReport> {
        let selected = self
            .entries
            .iter()
            .try_fold(0u64, |total, entry| total.checked_add(entry.size));
        let mut reporter = progress::Reporter::new(observer, selected);
        reporter.stage(progress::Stage::Verifying);
        let ids: Vec<_> = self.entries.iter().map(|entry| entry.id).collect();
        let report = match self.extract_selected(&ids, &mut |_, _| Ok(())) {
            Ok(report) => report,
            Err(error) => {
                reporter.finish(if matches!(error, Error::Cancelled) {
                    progress::Stage::Cancelled
                } else {
                    progress::Stage::Failed
                });
                return Err(error);
            }
        };
        for _ in 0..report.entries {
            reporter.verified_entry();
        }
        reporter.publish();
        reporter.finish(progress::Stage::Complete);
        Ok(report)
    }

    pub fn test(&mut self) -> Result<ExtractReport> {
        let ids: Vec<_> = self.entries.iter().map(|entry| entry.id).collect();
        self.extract_selected(&ids, &mut |_, _| Ok(()))
    }
}
/// Inflate raw DEFLATE, zlib or gzip from forward-only input into provisional output.
#[cfg(feature = "streams")]
pub fn inflate_stream(
    reader: &mut impl Read,
    writer: &mut impl Write,
    format: Format,
    limits: Limits,
) -> Result<u64> {
    let window = match format {
        Format::Deflate => 0,
        Format::Zlib => 15,
        Format::Gzip => 31,
        _ => return Err(Error::Unsupported("inflate format".into())),
    };
    if limits.max_dictionary_bytes < 32768 || limits.max_active_workspace_bytes < 1 << 20 {
        return Err(Error::ResourceLimit("DEFLATE workspace"));
    }
    let maximum = limits
        .max_input_bytes
        .checked_add(1)
        .ok_or(Error::ResourceLimit("input bytes"))?;
    let mut input = reader.take(maximum);
    let bytes = codec::inflate_window(
        &mut input,
        writer,
        window,
        limits.max_total_bytes,
        limits.max_entries,
    )?;
    if maximum - input.limit() > limits.max_input_bytes {
        return Err(Error::ResourceLimit("input bytes"));
    }
    Ok(bytes)
}
/// Compress forward-only input as raw DEFLATE, zlib or gzip.
#[cfg(feature = "streams")]
pub fn deflate_stream(
    reader: &mut impl Read,
    writer: &mut impl Write,
    format: Format,
    limits: Limits,
) -> Result<u64> {
    let window = match format {
        Format::Deflate => -15,
        Format::Zlib => 15,
        Format::Gzip => 31,
        _ => return Err(Error::Unsupported("deflate format".into())),
    };
    if limits.max_dictionary_bytes < 32768 || limits.max_active_workspace_bytes < 1 << 20 {
        return Err(Error::ResourceLimit("DEFLATE workspace"));
    }
    let mut compressor = codec::DeflateWriter::new(writer, window);
    let maximum = limits.max_input_bytes.min(limits.max_total_bytes);
    let bytes = copy_bounded(reader, &mut compressor, maximum)?;
    compressor.finish()?;
    Ok(bytes)
}
pub fn probe(bytes: &[u8]) -> Result<Format> {
    if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") {
        Ok(Format::Zip)
    } else if bytes.starts_with(b"MSCF") {
        Ok(Format::Cab)
    } else if bytes.starts_with(b"BZh") {
        Ok(Format::Bzip2)
    } else if bytes.starts_with(&[0x1f, 0x8b]) {
        Ok(Format::Gzip)
    } else if bytes.len() >= 2
        && bytes[0] & 15 == 8
        && bytes[0] >> 4 <= 7
        && (u16::from(bytes[0]) * 256 + u16::from(bytes[1])) % 31 == 0
    {
        Ok(Format::Zlib)
    } else if bytes.starts_with(b"7z\xbc\xaf\x27\x1c") {
        Ok(Format::SevenZip)
    } else if bytes.starts_with(b"\xfd7zXZ\0") {
        Ok(Format::Xz)
    } else if bytes.len() >= 512
        && (bytes[257..].starts_with(b"ustar") || bytes[..512].iter().all(|b| *b == 0))
    {
        Ok(Format::Tar)
    } else {
        Err(Error::Unsupported("unrecognized archive signature".into()))
    }
}
pub fn create<W: Write + Seek>(
    format: Format,
    entries: &[CreateEntry],
    writer: &mut W,
    limits: Limits,
) -> Result<()> {
    if matches!(
        format,
        Format::Zip | Format::Cab | Format::TarGzip | Format::Gzip | Format::Zlib
    ) && (limits.max_dictionary_bytes < 32768 || limits.max_active_workspace_bytes < 1 << 20)
    {
        return Err(Error::ResourceLimit("DEFLATE encoder workspace bytes"));
    }
    let _ = &writer;
    let metadata: Vec<_> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| Entry {
            id: EntryId(i),
            raw_name: e.name.as_bytes().to_vec(),
            name: e.name.clone(),
            kind: e.kind,
            size: e.data.len() as u64,
            compressed_size: None,
            compression: String::new(),
            encrypted: false,
        })
        .collect();
    validate_index(&metadata, limits)?;
    match format {
        #[cfg(feature = "zip")]
        Format::Zip => zip_backend::create(entries, writer),
        #[cfg(feature = "tar")]
        Format::Tar => tar_backend::create(entries, writer),
        #[cfg(any(feature = "bzip2", feature = "brotli"))]
        Format::TarBzip2 | Format::TarBrotli => {
            stream_backend::encode_tar(entries, writer, format, limits)
        }
        #[cfg(feature = "gzip")]
        Format::TarGzip => create_stream(format, entries, writer, limits),
        #[cfg(feature = "xz")]
        Format::TarXz => create_stream(format, entries, writer, limits),
        #[cfg(feature = "xz")]
        Format::Xz => {
            if entries.len() != 1 || entries[0].kind != EntryKind::File {
                return Err(Error::Unsupported(
                    "XZ requires exactly one regular entry".into(),
                ));
            }
            xz_backend::encode(&entries[0].data, writer, limits)
        }
        #[cfg(feature = "sevenz")]
        Format::SevenZip => sevenz_backend::create(entries, writer, &mut CreateOptions::default()),
        #[cfg(any(feature = "gzip", feature = "streams"))]
        Format::Gzip => {
            if entries.len() != 1 || entries[0].kind != EntryKind::File {
                return Err(Error::Unsupported("gzip requires one regular entry".into()));
            }
            stream_backend::encode_named_gzip(&entries[0].data, &entries[0].name, writer, limits)
        }
        #[cfg(feature = "streams")]
        Format::Zlib | Format::Lzma | Format::Deflate | Format::Bzip2 | Format::Brotli => {
            if entries.len() != 1 || entries[0].kind != EntryKind::File {
                return Err(Error::Unsupported(
                    "single-stream format requires one regular entry".into(),
                ));
            }
            stream_backend::encode(&entries[0].data, writer, format, limits)
        }
        #[cfg(feature = "cab")]
        Format::Cab => {
            let mut builder = cabinet::CabinetBuilder::new(cabinet::WriteCompression::MsZip);
            for e in entries {
                if e.kind != EntryKind::File {
                    return Err(Error::Unsupported("CAB directories and links".into()));
                }
                builder.add_file(&e.name, &e.data)?;
            }
            builder.write(writer)?;
            Ok(())
        }
        _ => Err(Error::Unsupported(format!("creation of {format:?}"))),
    }
}
/// Writes profiles that do not need output seeking, including TAR to stdout.
pub fn create_stream(
    format: Format,
    entries: &[CreateEntry],
    writer: &mut impl Write,
    limits: Limits,
) -> Result<()> {
    create_stream_inner(format, entries, writer, limits, None)
}
/// Create a forward-only archive while retaining supported source metadata.
pub fn create_stream_with_metadata(
    format: Format,
    entries: &[CreateEntry],
    writer: &mut impl Write,
    limits: Limits,
    metadata: &[EntryMetadata],
) -> Result<()> {
    if metadata.len() != entries.len() {
        return Err(Error::Malformed("creation metadata count mismatch".into()));
    }
    create_stream_inner(format, entries, writer, limits, Some(metadata))
}
fn create_stream_inner(
    format: Format,
    entries: &[CreateEntry],
    writer: &mut impl Write,
    limits: Limits,
    metadata_values: Option<&[EntryMetadata]>,
) -> Result<()> {
    let _ = metadata_values;
    if matches!(format, Format::TarGzip | Format::Gzip | Format::Zlib)
        && (limits.max_dictionary_bytes < 32768 || limits.max_active_workspace_bytes < 1 << 20)
    {
        return Err(Error::ResourceLimit("DEFLATE encoder workspace bytes"));
    }
    let _ = &writer;
    let mut total = 0u64;
    let mut metadata = 0u64;
    if entries.len() as u64 > limits.max_entries {
        return Err(Error::ResourceLimit("entries"));
    }
    for entry in entries {
        if entry.kind == EntryKind::Directory && !entry.data.is_empty() {
            return Err(Error::Malformed(
                "directory entry has a nonzero payload size".into(),
            ));
        }
        if entry
            .name
            .as_bytes()
            .split(|byte| matches!(byte, b'/' | b'\\'))
            .filter(|part| !part.is_empty())
            .count()
            > limits.max_nesting_depth
        {
            return Err(Error::ResourceLimit("entry path depth"));
        }

        let size = entry.data.len() as u64;
        if size > limits.max_entry_bytes {
            return Err(Error::ResourceLimit("entry decoded bytes"));
        }
        total = total
            .checked_add(size)
            .ok_or(Error::ResourceLimit("total decoded bytes"))?;
        metadata = metadata
            .checked_add(entry.name.len() as u64)
            .ok_or(Error::ResourceLimit("metadata bytes"))?;
    }
    if total > limits.max_total_bytes {
        return Err(Error::ResourceLimit("total decoded bytes"));
    }
    if metadata > limits.max_metadata_bytes {
        return Err(Error::ResourceLimit("metadata bytes"));
    }
    match format {
        #[cfg(feature = "tar")]
        Format::Tar => tar_backend::create_with_metadata(entries, writer, metadata_values),
        #[cfg(any(feature = "bzip2", feature = "brotli"))]
        Format::TarBzip2 | Format::TarBrotli => stream_backend::encode_tar_with_metadata(
            entries,
            writer,
            format,
            limits,
            metadata_values,
        ),
        #[cfg(feature = "gzip")]
        Format::TarGzip => {
            let mut compressor = codec::DeflateWriter::new(writer, 31);
            tar_backend::create_with_metadata(entries, &mut compressor, metadata_values)?;
            compressor.finish()?;
            Ok(())
        }
        #[cfg(any(feature = "gzip", feature = "streams"))]
        Format::Gzip => {
            if entries.len() != 1 || entries[0].kind != EntryKind::File {
                return Err(Error::Unsupported("gzip requires one regular entry".into()));
            }
            stream_backend::encode_named_gzip_with_mtime(
                &entries[0].data,
                &entries[0].name,
                writer,
                limits,
                metadata_values
                    .and_then(|values| values.first())
                    .and_then(|value| value.modified),
            )
        }
        #[cfg(feature = "streams")]
        Format::Zlib | Format::Lzma | Format::Deflate | Format::Bzip2 | Format::Brotli => {
            if entries.len() != 1 || entries[0].kind != EntryKind::File {
                return Err(Error::Unsupported(
                    "single-stream format requires one regular entry".into(),
                ));
            }
            stream_backend::encode(&entries[0].data, writer, format, limits)
        }
        #[cfg(feature = "xz")]
        Format::Xz => {
            if entries.len() != 1 || entries[0].kind != EntryKind::File {
                return Err(Error::Unsupported("XZ requires one regular entry".into()));
            }
            xz_backend::encode(&entries[0].data, writer, limits)
        }
        #[cfg(feature = "xz")]
        Format::TarXz => {
            let mut compressor = xz_backend::XzWriter::new(writer, limits, None)?;
            tar_backend::create_with_metadata(entries, &mut compressor, metadata_values)?;
            compressor.finish()
        }
        _ => Err(Error::Unsupported(format!(
            "{format:?} creation requires output seeking or an unsupported pipeline"
        ))),
    }
}
pub fn create_with_options<W: Write + Seek>(
    format: Format,
    entries: &[CreateEntry],
    writer: &mut W,
    limits: Limits,
    mut options: CreateOptions<'_>,
) -> Result<()> {
    let mut metadata = 0u64;
    let mut total = 0u64;
    if entries.len() as u64 > limits.max_entries {
        return Err(Error::ResourceLimit("entries"));
    }
    for entry in entries {
        if entry.kind == EntryKind::Directory && !entry.data.is_empty() {
            return Err(Error::Malformed(
                "directory entry has a nonzero payload size".into(),
            ));
        }
        if entry
            .name
            .as_bytes()
            .split(|byte| matches!(byte, b'/' | b'\\'))
            .filter(|part| !part.is_empty())
            .count()
            > limits.max_nesting_depth
        {
            return Err(Error::ResourceLimit("entry path depth"));
        }

        let size = entry.data.len() as u64;
        if size > limits.max_entry_bytes {
            return Err(Error::ResourceLimit("entry decoded bytes"));
        }
        total = total
            .checked_add(size)
            .ok_or(Error::ResourceLimit("total decoded bytes"))?;
        metadata = metadata
            .checked_add(entry.name.len() as u64)
            .ok_or(Error::ResourceLimit("metadata bytes"))?;
    }
    if total > limits.max_total_bytes {
        return Err(Error::ResourceLimit("total decoded bytes"));
    }
    if metadata > limits.max_metadata_bytes {
        return Err(Error::ResourceLimit("metadata bytes"));
    }
    if options
        .entry_metadata
        .is_some_and(|values| values.len() != entries.len())
    {
        return Err(Error::Malformed("creation metadata count mismatch".into()));
    }
    #[cfg(feature = "sevenz")]
    if format == Format::SevenZip {
        if options.sevenz_compression == SevenZipCompression::Deflate
            && (limits.max_dictionary_bytes < 32768 || limits.max_active_workspace_bytes < 1 << 20)
        {
            return Err(Error::ResourceLimit("7z DEFLATE workspace"));
        }
        let index: Vec<_> = entries
            .iter()
            .enumerate()
            .map(|(i, entry)| Entry {
                id: EntryId(i),
                raw_name: entry.name.as_bytes().to_vec(),
                name: entry.name.clone(),
                kind: entry.kind,
                size: entry.data.len() as u64,
                compressed_size: None,
                compression: String::new(),
                encrypted: false,
            })
            .collect();
        validate_index(&index, limits)?;
        return sevenz_backend::create(entries, writer, &mut options);
    }
    #[cfg(feature = "cab")]
    if options.password.is_none() && format == Format::Cab {
        let defaults;
        let values = if let Some(values) = options.entry_metadata {
            values
        } else {
            defaults = vec![EntryMetadata::default(); entries.len()];
            &defaults
        };
        if (options.cab_compression == CabCompression::MsZip
            && (limits.max_dictionary_bytes < 32768 || limits.max_active_workspace_bytes < 1 << 20))
            || (matches!(
                options.cab_compression,
                CabCompression::Lzx | CabCompression::Quantum
            ) && (limits.max_dictionary_bytes < 2 << 20
                || limits.max_active_workspace_bytes < 64 << 20))
        {
            return Err(Error::ResourceLimit("CAB encoder workspace bytes"));
        }
        let mut builder = cabinet::CabinetBuilder::new(match options.cab_compression {
            CabCompression::Copy => cabinet::WriteCompression::None,
            CabCompression::MsZip => cabinet::WriteCompression::MsZip,
            CabCompression::Lzx => cabinet::WriteCompression::Lzx { window_order: 21 },
            CabCompression::Quantum => cabinet::WriteCompression::Quantum {
                level: 6,
                window_order: 21,
            },
        });
        for (entry, metadata) in entries.iter().zip(values) {
            if entry.kind != EntryKind::File {
                return Err(Error::Unsupported("CAB directories and links".into()));
            }
            if let Some(StoredTimestamp::DosLocal {
                year,
                month,
                day,
                hour,
                minute,
                second,
            }) = metadata.modified
            {
                if !(1980..=2107).contains(&year) {
                    return Err(Error::Unsupported(
                        "CAB timestamp year must be 1980 through 2107".into(),
                    ));
                }
                let date = ((year - 1980) << 9) | (u16::from(month) << 5) | u16::from(day);
                let time =
                    (u16::from(hour) << 11) | (u16::from(minute) << 5) | u16::from(second / 2);
                let attributes = 0x20
                    | if metadata.unix_mode.is_some_and(|mode| mode & 0o222 == 0) {
                        1
                    } else {
                        0
                    };
                builder.add_file_with_metadata(&entry.name, &entry.data, date, time, attributes)?;
            } else {
                builder.add_file(&entry.name, &entry.data)?;
            }
        }
        builder.write(writer)?;
        return Ok(());
    }
    #[cfg(feature = "zip")]
    if format == Format::Zip && options.password.is_none() {
        if options.zip_compression == ZipCompression::Deflate
            && (limits.max_dictionary_bytes < 32768 || limits.max_active_workspace_bytes < 1 << 20)
        {
            return Err(Error::ResourceLimit("DEFLATE encoder workspace bytes"));
        }
        return zip_backend::create_with_compression(
            entries,
            writer,
            options.entry_metadata,
            options.zip_compression,
        );
    }
    if let Some(values) = options.entry_metadata {
        if values.len() != entries.len() {
            return Err(Error::Malformed("creation metadata count mismatch".into()));
        }
        if options.password.is_none()
            && matches!(
                format,
                Format::Tar
                    | Format::TarGzip
                    | Format::TarXz
                    | Format::TarBzip2
                    | Format::TarBrotli
                    | Format::Gzip
            )
        {
            return create_stream_with_metadata(format, entries, writer, limits, values);
        }
    }
    if options.password.is_none() {
        return create(format, entries, writer, limits);
    }
    if format != Format::Zip {
        return Err(Error::Unsupported(
            "encrypted creation for this format".into(),
        ));
    }
    if options.encrypt_headers {
        return Err(Error::Unsupported("ZIP filename encryption".into()));
    }
    #[cfg(all(feature = "zip", feature = "crypto"))]
    {
        let password = options.password.ok_or(Error::PasswordRequired)?;
        let random = options.randomness.as_deref_mut().ok_or_else(|| {
            Error::Unsupported("encrypted creation requires secure randomness".into())
        })?;
        let total = entries.iter().try_fold(0u64, |total, e| {
            total
                .checked_add(e.data.len() as u64)
                .ok_or(Error::ResourceLimit("total decoded bytes"))
        })?;
        if total > limits.max_total_bytes {
            return Err(Error::ResourceLimit("total decoded bytes"));
        }
        if entries.len() as u64 > limits.max_entries {
            return Err(Error::ResourceLimit("entries"));
        }
        zip_backend::create_encrypted(
            entries,
            writer,
            password,
            random,
            options.zip_encryption,
            options.entry_metadata,
            options.zip_compression,
        )
    }
    #[cfg(not(all(feature = "zip", feature = "crypto")))]
    {
        let _ = (&mut options, writer, entries, limits);
        Err(Error::Unsupported("ZIP crypto feature unavailable".into()))
    }
}
fn validate_index(entries: &[Entry], limits: Limits) -> Result<()> {
    if entries.len() as u64 > limits.max_entries {
        return Err(Error::ResourceLimit("entries"));
    }
    let mut metadata = 0u64;
    let mut total = 0u64;
    for e in entries {
        if e.kind == EntryKind::Directory && e.size != 0 {
            return Err(Error::Malformed(
                "directory entry has a nonzero payload size".into(),
            ));
        }
        if e.raw_name
            .split(|byte| matches!(byte, b'/' | b'\\'))
            .filter(|part| !part.is_empty())
            .count()
            > limits.max_nesting_depth
        {
            return Err(Error::ResourceLimit("entry path depth"));
        }
        metadata = metadata
            .checked_add(e.raw_name.len() as u64)
            .ok_or(Error::ResourceLimit("metadata bytes"))?;
        total = total
            .checked_add(e.size)
            .ok_or(Error::ResourceLimit("total decoded bytes"))?;
        if e.size > limits.max_entry_bytes {
            return Err(Error::ResourceLimit("entry decoded bytes"));
        }
    }
    if metadata > limits.max_metadata_bytes {
        return Err(Error::ResourceLimit("metadata bytes"));
    }
    if total > limits.max_total_bytes {
        return Err(Error::ResourceLimit("total decoded bytes"));
    }
    Ok(())
}
#[allow(dead_code)]
pub(crate) fn copy_bounded(
    reader: &mut impl Read,
    writer: &mut impl Write,
    limit: u64,
) -> Result<u64> {
    let mut total = 0u64;
    let mut buf = [0u8; 65536];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total = total
            .checked_add(n as u64)
            .ok_or(Error::ResourceLimit("decoded bytes"))?;
        if total > limit {
            return Err(Error::ResourceLimit("decoded bytes"));
        }
        writer.write_all(&buf[..n])?;
    }
    Ok(total)
}
#[cfg(feature = "cab")]
fn cab_preflight(reader: &mut (impl Read + Seek), limits: Limits) -> Result<()> {
    let mut header = [0u8; 36];
    reader.read_exact(&mut header)?;
    let count = u16::from_le_bytes([header[28], header[29]]) as u64;
    if count > limits.max_entries {
        return Err(Error::ResourceLimit("entries"));
    }
    let flags = u16::from_le_bytes([header[30], header[31]]);
    if flags & 3 != 0 {
        return Err(Error::Unsupported(
            "CAB spanning requires explicit volume resolver".into(),
        ));
    }
    let mut reserve = 0u8;
    if flags & 4 != 0 {
        let mut r = [0u8; 4];
        reader.read_exact(&mut r)?;
        reserve = r[2];
        reader.seek(SeekFrom::Current(i64::from(u16::from_le_bytes([
            r[0], r[1],
        ]))))?;
    }
    let folders = u16::from_le_bytes([header[26], header[27]]);
    for _ in 0..folders {
        let mut folder = [0u8; 8];
        reader.read_exact(&mut folder)?;
        let method = u16::from_le_bytes([folder[6], folder[7]]);
        let order = (method >> 8) & 31;
        let dictionary = match method & 15 {
            0 => 0,
            1 => 32768,
            2 | 3 => 1u64
                .checked_shl(u32::from(order))
                .ok_or(Error::ResourceLimit("dictionary bytes"))?,
            _ => return Err(Error::Unsupported("CAB codec".into())),
        };
        if dictionary > limits.max_dictionary_bytes {
            return Err(Error::ResourceLimit("dictionary bytes"));
        }
        if dictionary != 0 && dictionary.saturating_add(2 << 20) > limits.max_active_workspace_bytes
        {
            return Err(Error::ResourceLimit("CAB decoder workspace bytes"));
        }
        reader.seek(SeekFrom::Current(i64::from(reserve)))?;
    }
    reader.seek(SeekFrom::Start(u64::from(u32::from_le_bytes(
        header[16..20]
            .try_into()
            .map_err(|_| Error::Malformed("CAB header".into()))?,
    ))))?;
    let mut metadata = 0u64;
    for _ in 0..count {
        let mut fixed = [0u8; 16];
        reader.read_exact(&mut fixed)?;
        loop {
            let mut byte = [0u8; 1];
            reader.read_exact(&mut byte)?;
            metadata = metadata
                .checked_add(1)
                .ok_or(Error::ResourceLimit("metadata bytes"))?;
            if metadata > limits.max_metadata_bytes {
                return Err(Error::ResourceLimit("metadata bytes"));
            }
            if byte[0] == 0 {
                break;
            }
        }
    }
    reader.rewind()?;
    Ok(())
}
