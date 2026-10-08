use archive_core::{Archive, CreateEntry, EntryKind, Format, Limits};
mod formats;
mod optical;
mod package_compat;
#[cfg(feature = "progress")]
mod render_progress;
mod single_file;
mod staging;
mod streams;
mod tar_args;
use clap::{Parser, Subcommand, ValueEnum};
#[cfg(feature = "progress")]
use render_progress::RenderProgress;
use std::{
    io::{self, IsTerminal},
    path::{Path, PathBuf},
};

static CANCELLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn check_cancelled() -> io::Result<()> {
    if CANCELLED.load(std::sync::atomic::Ordering::Relaxed) {
        // Interrupted is retried by read_exact/write_all; cancellation is terminal.
        Err(io::Error::other("operation cancelled"))
    } else {
        Ok(())
    }
}
#[cfg(unix)]
extern "C" fn interrupt(_: libc::c_int) {
    CANCELLED.store(true, std::sync::atomic::Ordering::Relaxed);
}
#[cfg(windows)]
unsafe extern "system" fn console_interrupt(event: u32) -> i32 {
    if event <= 1 {
        CANCELLED.store(true, std::sync::atomic::Ordering::Relaxed);
        1
    } else {
        0
    }
}
fn install_interrupt_handler() {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGINT, interrupt as *const () as libc::sighandler_t);
    }
    #[cfg(windows)]
    unsafe {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn SetConsoleCtrlHandler(
                handler: Option<unsafe extern "system" fn(u32) -> i32>,
                add: i32,
            ) -> i32;
        }
        SetConsoleCtrlHandler(Some(console_interrupt), 1);
    }
}
struct CancellableSink<'a>(&'a mut std::fs::File);
struct CancellableSource<R>(R);
impl<R: io::Read> io::Read for CancellableSource<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        check_cancelled()?;
        self.0.read(bytes)
    }
}
impl io::Write for CancellableSink<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        check_cancelled()?;
        io::Write::write(self.0, bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        check_cancelled()?;
        io::Write::flush(self.0)
    }
}

#[derive(Parser)]
#[command(name = "arc", version, about = "Portable archive operations")]
struct Cli {
    /// Print processed entry names to stderr.
    #[arg(long, global = true)]
    verbose: bool,
    #[arg(short = 'j', long, global = true)]
    json: bool,
    #[arg(short = 'p', long, global = true)]
    password_file: Option<PathBuf>,
    #[arg(short = 'b', long, global = true)]
    bundle_entry: Option<String>,
    #[arg(short = 'm', long, global = true, value_name = "NAME=PATH")]
    media: Vec<MediaMapping>,
    #[arg(short = 'I', long, global = true)]
    image: Option<u32>,
    #[arg(short = 'n', long, global = true, conflicts_with = "image")]
    image_name: Option<String>,
    #[arg(short = 'v', long, global = true, value_enum)]
    view: Option<OpticalView>,
    #[arg(short = 'M', long, global = true, default_value_t = 1073741824)]
    max_input_bytes: u64,
    /// Maximum decoded bytes per entry.
    #[arg(long, global = true, default_value_t = 8u64 << 30)]
    max_entry_bytes: u64,
    /// Maximum total decoded bytes per archive operation.
    #[arg(long, global = true, default_value_t = 32u64 << 30)]
    max_total_bytes: u64,
    #[arg(long, global = true, default_value_t = 100_000)]
    max_entries: u64,
    #[arg(short = 'P', long, global = true, value_enum, default_value = "auto")]
    progress: Progress,
    #[command(subcommand)]
    command: Command,
}
#[derive(Clone, Copy, ValueEnum)]
enum Progress {
    Auto,
    Always,
    Never,
}
#[derive(Clone, Copy, ValueEnum)]
enum OpticalView {
    Iso,
    Udf,
}
#[derive(Clone, Copy, ValueEnum)]
enum ZipEncryption {
    Aes256,
    Zipcrypto,
}
#[derive(Clone, Copy, ValueEnum)]
enum ArchiveCompression {
    Copy,
    Deflate,
    #[value(name = "mszip", alias = "ms-zip")]
    MsZip,
    Lzx,
    Quantum,
    Lzma,
    Lzma2,
    Bzip2,
    Brotli,
}
#[derive(Clone, Copy, ValueEnum)]
enum DeflateWrapper {
    Deflate,
    Gzip,
    Zlib,
}
#[derive(Clone)]
struct MediaMapping {
    name: String,
    path: PathBuf,
}
impl std::str::FromStr for MediaMapping {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (name, path) = value
            .split_once('=')
            .ok_or("media mapping requires NAME=PATH")?;
        if name.is_empty() || path.is_empty() {
            return Err("media mapping requires nonempty NAME and PATH".into());
        }
        Ok(Self {
            name: name.into(),
            path: path.into(),
        })
    }
}
#[derive(Subcommand)]
enum Command {
    /// Compress a single file using a stream or raw Windows codec.
    Compress {
        /// Codec name; defaults to the output suffix for compression and detection for decompression.
        #[arg(short = 'f', long)]
        format: Option<String>,
        #[arg(short = 'i', long)]
        input: PathBuf,
        #[arg(short = 'o', long)]
        output: PathBuf,
        /// Exact decoded byte count for raw Windows codecs.
        #[arg(long)]
        output_size: Option<u64>,
        /// Raw LZMA2 dictionary size in bytes (must match when decoding).
        #[arg(long, default_value_t = 8 << 20)]
        dictionary_bytes: u32,
        /// Raw LZX/Quantum window order (must match when decoding).
        #[arg(long, default_value_t = 15)]
        window_order: u8,
    },
    /// Decompress a single file; raw Windows codecs require --output-size.
    Decompress {
        /// Codec name; defaults to the output suffix for compression and detection for decompression.
        #[arg(short = 'f', long)]
        format: Option<String>,
        #[arg(short = 'i', long)]
        input: PathBuf,
        #[arg(short = 'o', long)]
        output: PathBuf,
        /// Exact decoded byte count for raw Windows codecs.
        #[arg(long)]
        output_size: Option<u64>,
        /// Raw LZMA2 dictionary size in bytes (must match when decoding).
        #[arg(long, default_value_t = 8 << 20)]
        dictionary_bytes: u32,
        /// Raw LZX/Quantum window order (must match when decoding).
        #[arg(long, default_value_t = 15)]
        window_order: u8,
    },
    Deflate {
        #[arg(short = 'f', long, value_enum)]
        format: Option<DeflateWrapper>,
        #[arg(short = 'i', long)]
        input: PathBuf,
        #[arg(short = 'o', long)]
        output: PathBuf,
    },
    Inflate {
        #[arg(short = 'f', long, value_enum)]
        format: Option<DeflateWrapper>,
        #[arg(short = 'i', long)]
        input: PathBuf,
        #[arg(short = 'o', long)]
        output: PathBuf,
    },
    #[command(visible_alias = "l")]
    List {
        archive: PathBuf,
        #[arg(short = 'f', long)]
        format: Option<String>,
    },
    #[command(visible_alias = "t")]
    Test {
        archive: PathBuf,
        #[arg(short = 'f', long)]
        format: Option<String>,
    },
    #[command(visible_alias = "x")]
    Extract {
        archive: PathBuf,
        #[arg(short = 'f', long)]
        format: Option<String>,
        /// Extraction directory; '*' path components expand to the archive name.
        #[arg(short = 'o', visible_short_alias = 'd', long, default_value = ".")]
        output: PathBuf,
        /// Extract into a subfolder named after the archive, without its extension.
        #[arg(short = 's', long)]
        archive_folder: bool,
        #[arg(short = 't', long, default_value_t = 1)]
        threads: usize,
    },
    #[command(visible_alias = "a")]
    Create {
        #[arg(short = 'c', long, value_enum)]
        compression: Option<ArchiveCompression>,
        #[arg(short = 'z', long, value_enum, default_value = "aes256")]
        zip_encryption: ZipEncryption,
        #[arg(short = 'e', long)]
        encrypt: bool,
        #[arg(short = 'H', long)]
        encrypt_headers: bool,
        #[arg(short = 'f', long)]
        format: Option<String>,
        #[arg(short = 'i', long)]
        input: PathBuf,
        #[arg(short = 'o', long)]
        output: PathBuf,
    },
}

fn main() {
    install_interrupt_handler();
    let args = tar_args::expand(std::env::args_os().collect());
    let mut cli = match args.and_then(Cli::try_parse_from) {
        Ok(cli) => cli,
        Err(error) => {
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                let _ = error.print();
                std::process::exit(0);
            }
            if std::env::args_os().any(|argument| argument == "--json" || argument == "-j") {
                println!(
                    "{}",
                    serde_json::json!({"schema_version":1,"ok":false,"error":{"code":2,"message":error.to_string()}})
                );
            } else {
                let _ = error.print();
            }
            std::process::exit(2);
        }
    };
    let result = prepare_output(&mut cli).and_then(|()| run(&cli));
    if let Err(error) = result {
        let code = if CANCELLED.load(std::sync::atomic::Ordering::Relaxed) {
            130
        } else {
            error.downcast_ref::<archive_core::Error>().map_or_else(
                || {
                    error
                        .downcast_ref::<ms_package::Error>()
                        .map_or(1, |e| match e {
                            ms_package::Error::Malformed(_) | ms_package::Error::Integrity(_) => 4,
                            ms_package::Error::Unsupported(_) => 3,
                            ms_package::Error::Limit(_) => 5,
                            _ => 1,
                        })
                },
                |e| match e {
                    archive_core::Error::Unsupported(_) => 3,
                    archive_core::Error::Integrity(_) | archive_core::Error::Malformed(_) => 4,
                    archive_core::Error::ResourceLimit(_) => 5,
                    archive_core::Error::PasswordRequired => 6,
                    archive_core::Error::Cancelled => 130,
                    archive_core::Error::Io(_) => 1,
                },
            )
        };
        if cli.json {
            println!(
                "{}",
                serde_json::json!({"schema_version":1,"ok":false,"error":{"message":error.to_string(),"code":code}})
            );
        } else {
            eprintln!("arc: {error}");
        }
        std::process::exit(code);
    }
}

fn prepare_output(cli: &mut Cli) -> Result<(), Box<dyn std::error::Error>> {
    if let Command::Extract {
        archive,
        output,
        archive_folder,
        ..
    } = &mut cli.command
    {
        let wildcard = output.components().any(|part| part.as_os_str() == "*");
        if *archive_folder || wildcard {
            if archive == Path::new("-") {
                return Err(
                    "archive-named extraction requires a filename; use -o DIR for stdin".into(),
                );
            }
            let stem = formats::archive_stem(archive)?;
            if wildcard {
                let mut expanded = PathBuf::new();
                for part in output.components() {
                    if part.as_os_str() == "*" {
                        expanded.push(&stem);
                    } else {
                        expanded.push(part.as_os_str());
                    }
                }
                *output = expanded;
            } else {
                output.push(stem);
            }
        }
    }
    Ok(())
}

fn run(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    let enabled = matches!(cli.progress, Progress::Always)
        || (matches!(cli.progress, Progress::Auto) && !cli.json && io::stderr().is_terminal());
    #[cfg(not(feature = "progress"))]
    if matches!(cli.progress, Progress::Always) {
        return Err("progress rendering unavailable; rebuild with --features progress".into());
    }
    #[cfg(feature = "progress")]
    let mut bar = if enabled {
        Some(RenderProgress::new())
    } else {
        None
    };
    let _ = enabled;
    let limits = Limits {
        max_input_bytes: cli.max_input_bytes,
        max_entry_bytes: cli.max_entry_bytes,
        max_total_bytes: cli.max_total_bytes,
        max_entries: cli.max_entries,
        ..Limits::default()
    };
    let result = match &cli.command {
        Command::Compress { output, .. } | Command::Decompress { output, .. } => {
            let result = single_file::run(cli, limits)?;
            if output == Path::new("-") {
                return Ok(());
            }
            result
        }
        Command::Deflate {
            format,
            input,
            output,
        }
        | Command::Inflate {
            format,
            input,
            output,
        } => {
            if output == Path::new("-") && cli.json {
                return Err(archive_core::Error::Unsupported(
                    "binary stdout cannot combine with JSON".into(),
                )
                .into());
            }
            if cli.password_file.is_some() {
                return Err(
                    archive_core::Error::Unsupported("stream password encryption".into()).into(),
                );
            }
            let operation = if matches!(cli.command, Command::Inflate { .. }) {
                "inflate"
            } else {
                "deflate"
            };
            let source: Box<dyn io::Read> = if input == Path::new("-") {
                Box::new(io::stdin())
            } else {
                Box::new(std::fs::File::open(input)?)
            };
            let mut source = CancellableSource(source);
            let format = if let Some(format) = format {
                match format {
                    DeflateWrapper::Deflate => Format::Deflate,
                    DeflateWrapper::Gzip => Format::Gzip,
                    DeflateWrapper::Zlib => Format::Zlib,
                }
            } else if operation == "deflate" {
                formats::creation(None, output)?
            } else {
                use io::Read;
                let mut prefix = Vec::new();
                (&mut source).take(2).read_to_end(&mut prefix)?;
                let detected = archive_core::probe(&prefix).unwrap_or(Format::Deflate);
                source = CancellableSource(Box::new(io::Cursor::new(prefix).chain(source)));
                detected
            };
            if !matches!(format, Format::Deflate | Format::Gzip | Format::Zlib) {
                return Err(archive_core::Error::Unsupported("deflate/inflate supports only deflate, gzip, and zlib; use create/extract for other formats".into()).into());
            }

            let transform = |sink: &mut dyn io::Write| -> archive_core::Result<u64> {
                let mut sink = sink;
                if operation == "inflate" {
                    archive_core::inflate_stream(&mut source, &mut sink, format, limits)
                } else {
                    archive_core::deflate_stream(&mut source, &mut sink, format, limits)
                }
            };
            let mut transform = transform;
            if output == Path::new("-") {
                transform(&mut io::stdout().lock())?;
                return Ok(());
            }
            let parent = output
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let mut temp = tempfile::NamedTempFile::new_in(parent)?;
            let bytes = transform(&mut CancellableSink(temp.as_file_mut()))?;
            check_cancelled()?;
            temp.as_file().sync_all()?;
            temp.persist_noclobber(output)?;
            serde_json::json!({"schema_version":1,"ok":true,"operation":operation,"bytes":bytes})
        }
        Command::Create {
            compression,
            format,
            input,
            output,
            encrypt,
            encrypt_headers,
            zip_encryption,
        } => {
            if format
                .as_deref()
                .is_some_and(|name| matches!(name, "appx" | "msix" | "msi"))
            {
                return Err(archive_core::Error::Unsupported(
                    "Windows packages are read-only".into(),
                )
                .into());
            }
            let format = formats::creation(format.as_deref(), output)?;
            let valid_codec = matches!(
                (format, compression),
                (_, None)
                    | (
                        Format::SevenZip,
                        Some(
                            ArchiveCompression::Copy
                                | ArchiveCompression::Deflate
                                | ArchiveCompression::Lzma
                                | ArchiveCompression::Lzma2
                                | ArchiveCompression::Bzip2
                                | ArchiveCompression::Brotli
                        )
                    )
                    | (
                        Format::Zip,
                        Some(ArchiveCompression::Copy | ArchiveCompression::Deflate)
                    )
                    | (
                        Format::Cab,
                        Some(
                            ArchiveCompression::Copy
                                | ArchiveCompression::MsZip
                                | ArchiveCompression::Lzx
                                | ArchiveCompression::Quantum
                        )
                    )
            );
            if !valid_codec {
                return Err(archive_core::Error::Unsupported(
                    "codec is incompatible with output archive format".into(),
                )
                .into());
            }
            if compression.is_some() && output == Path::new("-") {
                return Err("archive codec selection requires an output file".into());
            }
            if output == Path::new("-") && (cli.json || *encrypt || *encrypt_headers) {
                return Err(archive_core::Error::Unsupported(
                    "binary stdout cannot combine with JSON or encrypted seek containers".into(),
                )
                .into());
            }
            let mut entries = Vec::new();
            let mut total = 0;
            enumerate(input, input, &mut entries, &mut total, &limits)?;
            if cli.verbose {
                for entry in &entries {
                    eprintln!("{}", entry.name);
                }
            }
            if matches!(
                format,
                Format::Cab
                    | Format::Gzip
                    | Format::Zlib
                    | Format::Lzma
                    | Format::Xz
                    | Format::Bzip2
                    | Format::Brotli
                    | Format::Deflate
            ) {
                entries.retain(|entry| entry.kind == EntryKind::File);
            }
            let metadata: Vec<_> = entries
                .iter()
                .map(|entry| {
                    if format == Format::Cab {
                        archive_fs::source_dos_metadata(&input.join(&entry.name))
                    } else {
                        archive_fs::source_metadata(&input.join(&entry.name))
                    }
                })
                .collect::<io::Result<_>>()?;
            if output == Path::new("-") {
                archive_core::create_stream_with_metadata(
                    format,
                    &entries,
                    &mut io::stdout().lock(),
                    limits,
                    &metadata,
                )?;
                return Ok(());
            }
            let parent = output
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let mut temp = tempfile::NamedTempFile::new_in(parent)?;
            let password = read_password(cli)?;
            if *encrypt && password.is_none() {
                return Err("--encrypt requires an explicit --password-file source".into());
            }
            let mut randomness = NativeRandom;
            archive_core::create_with_options(
                format,
                &entries,
                temp.as_file_mut(),
                limits,
                archive_core::CreateOptions {
                    sevenz_compression: match compression.unwrap_or(ArchiveCompression::Lzma2) {
                        ArchiveCompression::Copy => archive_core::SevenZipCompression::Copy,
                        ArchiveCompression::Deflate => archive_core::SevenZipCompression::Deflate,
                        ArchiveCompression::MsZip
                        | ArchiveCompression::Lzx
                        | ArchiveCompression::Quantum => {
                            archive_core::SevenZipCompression::default()
                        }
                        ArchiveCompression::Lzma => archive_core::SevenZipCompression::Lzma,
                        ArchiveCompression::Lzma2 => archive_core::SevenZipCompression::Lzma2,
                        ArchiveCompression::Bzip2 => archive_core::SevenZipCompression::Bzip2,
                        ArchiveCompression::Brotli => archive_core::SevenZipCompression::Brotli,
                    },
                    zip_compression: if matches!(compression, Some(ArchiveCompression::Copy)) {
                        archive_core::ZipCompression::Copy
                    } else {
                        archive_core::ZipCompression::Deflate
                    },
                    cab_compression: match compression {
                        Some(ArchiveCompression::Copy) => archive_core::CabCompression::Copy,
                        Some(ArchiveCompression::Lzx) => archive_core::CabCompression::Lzx,
                        Some(ArchiveCompression::Quantum) => archive_core::CabCompression::Quantum,
                        _ => archive_core::CabCompression::MsZip,
                    },
                    zip_encryption: match zip_encryption {
                        ZipEncryption::Aes256 => archive_core::ZipEncryption::Aes256,
                        ZipEncryption::Zipcrypto => archive_core::ZipEncryption::ZipCrypto,
                    },
                    encrypt_headers: *encrypt_headers,
                    password: if *encrypt {
                        password.as_ref().map(|p| p.0.as_slice())
                    } else {
                        None
                    },
                    randomness: Some(&mut randomness),
                    entry_metadata: Some(&metadata),
                },
            )?;
            check_cancelled()?;
            temp.as_file().sync_all()?;
            temp.persist_noclobber(output)?;
            serde_json::json!({"schema_version":1,"ok":true,"operation":"create","entries":entries.len()})
        }
        command => {
            let path = match command {
                Command::List { archive, .. }
                | Command::Test { archive, .. }
                | Command::Extract { archive, .. } => archive,
                _ => unreachable!(),
            };
            if path == Path::new("-") {
                let result = streams::stdin_operation(cli, limits)?;
                if cli.json {
                    println!("{result}");
                } else if !matches!(cli.command, Command::List { .. }) {
                    println!("{}", result["operation"].as_str().unwrap_or("operation"));
                }
                return Ok(());
            }
            if let Some(result) = package_operation(cli, path)? {
                #[cfg(feature = "progress")]
                if let Some(bar) = bar {
                    bar.finish_and_clear();
                }
                if cli.json {
                    println!("{result}");
                } else if !matches!(cli.command, Command::List { .. }) {
                    println!("{}", result["operation"].as_str().unwrap_or("operation"));
                }
                return Ok(());
            }
            let password = read_password(cli)?;
            let interpretation = match command {
                Command::List { format, .. }
                | Command::Test { format, .. }
                | Command::Extract { format, .. } => format.as_deref(),
                _ => None,
            };
            let mut archive = if let Some(format) = interpretation {
                let format = formats::parse(format)?;
                if password.is_some() {
                    return Err(archive_core::Error::Unsupported(
                        "explicit interpretation with password".into(),
                    )
                    .into());
                }
                Archive::open_with_scratch(
                    std::fs::File::open(path)?,
                    tempfile::tempfile()?,
                    limits,
                    Some(format),
                    None,
                )?
            } else if let Some(password) = &password {
                Archive::open_with_scratch(
                    std::fs::File::open(path)?,
                    tempfile::tempfile()?,
                    limits,
                    None,
                    Some(&password.0),
                )?
            } else {
                match Archive::open_with_scratch(
                    std::fs::File::open(path)?,
                    tempfile::tempfile()?,
                    limits,
                    None,
                    None,
                ) {
                    Err(archive_core::Error::Unsupported(message))
                        if message == "unrecognized archive signature" =>
                    {
                        let hint = formats::extension(path).filter(|format| {
                            matches!(
                                format,
                                Format::Lzma | Format::Brotli | Format::TarBrotli | Format::Deflate
                            )
                        });
                        if let Some(format) = hint {
                            Archive::open_with_scratch(
                                std::fs::File::open(path)?,
                                tempfile::tempfile()?,
                                limits,
                                Some(format),
                                None,
                            )?
                        } else {
                            return Err(archive_core::Error::Unsupported(
                                "unrecognized archive signature; specify --format for raw streams"
                                    .into(),
                            )
                            .into());
                        }
                    }
                    result => result?,
                }
            };
            if cli.verbose && !matches!(command, Command::List { .. }) {
                for entry in archive.entries() {
                    eprintln!("{}", entry.name);
                }
            }
            match command {
                Command::List { .. } => {
                    let entries: Vec<_> = archive.entries().iter().map(|e| serde_json::json!({"id":e.id.0,"name":e.name,"raw_name":e.raw_name,"kind":format!("{:?}",e.kind).to_lowercase(),"size":e.size,"compressed_size":e.compressed_size,"compression":e.compression,"encrypted":e.encrypted})).collect();
                    if !cli.json {
                        for e in archive.entries() {
                            println!("{:>12} {}", e.size, e.name);
                        }
                    }
                    serde_json::json!({"schema_version":1,"ok":true,"operation":"list","format":format!("{:?}",archive.format()).to_lowercase(),"entries":entries})
                }
                Command::Test { .. } => {
                    #[cfg(feature = "progress")]
                    let report = if let Some(renderer) = &mut bar {
                        archive.test_observed(&mut renderer.observer)?
                    } else {
                        archive.test()?
                    };
                    #[cfg(not(feature = "progress"))]
                    let report = archive.test()?;
                    serde_json::json!({"schema_version":1,"ok":true,"operation":"test","verified":report.verified,"bytes":report.bytes,"entries":report.entries})
                }
                Command::Extract {
                    output, threads, ..
                } => {
                    if *threads == 0 {
                        return Err("worker count must be positive".into());
                    }
                    std::fs::create_dir_all(output)?;
                    let mut destination = archive_fs::Destination::open(output)?;
                    let mut selected = archive.entries().to_vec();
                    if matches!(archive.format(), Format::Iso | Format::SevenZip) {
                        for entry in &mut selected {
                            entry.raw_name = entry.name.as_bytes().to_vec();
                        }
                    }
                    // Preflight all names before creating any outputs.
                    let mut names = std::collections::BTreeSet::new();
                    for entry in &selected {
                        let key = archive_fs::validate_name(&entry.raw_name)?
                            .join("/")
                            .to_lowercase();
                        if !names.insert(key) {
                            return Err("duplicate destination path".into());
                        }
                        if !matches!(entry.kind, EntryKind::File | EntryKind::Directory) {
                            return Err("links and special files are unsupported".into());
                        }
                    }
                    #[cfg(feature = "progress")]
                    let result = if let Some(renderer) = &mut bar {
                        extract_batch(
                            &mut archive,
                            &selected,
                            &mut destination,
                            *threads,
                            &mut renderer.observer,
                        )?
                    } else {
                        extract_batch(
                            &mut archive,
                            &selected,
                            &mut destination,
                            *threads,
                            &mut archive_core::progress::NoProgress,
                        )?
                    };
                    #[cfg(not(feature = "progress"))]
                    let result = extract_batch(
                        &mut archive,
                        &selected,
                        &mut destination,
                        *threads,
                        &mut archive_core::progress::NoProgress,
                    )?;
                    result
                }
                _ => unreachable!(),
            }
        }
    };
    #[cfg(feature = "progress")]
    if let Some(bar) = bar {
        bar.finish_and_clear();
    }
    if cli.json {
        println!("{result}");
    } else if !matches!(cli.command, Command::List { .. }) {
        println!("{}", result["operation"].as_str().unwrap_or("operation"));
    }
    Ok(())
}

struct Password(Vec<u8>);
impl Drop for Password {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.0.zeroize();
    }
}
fn read_password(cli: &Cli) -> Result<Option<Password>, Box<dyn std::error::Error>> {
    use std::io::Read;
    let Some(path) = &cli.password_file else {
        return Ok(None);
    };
    let mut password = Password(Vec::with_capacity(16_386));
    std::fs::File::open(path)?
        .take(16385)
        .read_to_end(&mut password.0)?;
    let bytes = &mut password.0;
    if bytes.len() > 16384 {
        use zeroize::Zeroize;
        bytes.zeroize();
        return Err("password source exceeds length limit".into());
    }
    if bytes.ends_with(b"\n") {
        bytes.pop();
        if bytes.ends_with(b"\r") {
            bytes.pop();
        }
    }
    Ok(Some(password))
}
struct NativeRandom;
impl archive_core::RandomSource for NativeRandom {
    fn fill(&mut self, bytes: &mut [u8]) -> archive_core::Result<()> {
        #[cfg(unix)]
        {
            use std::io::Read;
            std::fs::File::open("/dev/urandom")?.read_exact(bytes)?;
            Ok(())
        }
        #[cfg(windows)]
        {
            unsafe {
                #[link(name = "bcrypt")]
                unsafe extern "system" {
                    fn BCryptGenRandom(
                        algorithm: *mut std::ffi::c_void,
                        buffer: *mut u8,
                        length: u32,
                        flags: u32,
                    ) -> i32;
                }
                let length = u32::try_from(bytes.len())
                    .map_err(|_| archive_core::Error::ResourceLimit("randomness bytes"))?;
                if BCryptGenRandom(std::ptr::null_mut(), bytes.as_mut_ptr(), length, 2) < 0 {
                    return Err(archive_core::Error::Io(io::Error::other(
                        "OS randomness failed",
                    )));
                }
                Ok(())
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = bytes;
            Err(archive_core::Error::Unsupported(
                "OS randomness unavailable".into(),
            ))
        }
    }
}

fn enumerate(
    root: &Path,
    path: &Path,
    entries: &mut Vec<CreateEntry>,
    total: &mut u64,
    limits: &Limits,
) -> Result<(), Box<dyn std::error::Error>> {
    check_cancelled()?;
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Err("input symlinks are unsupported".into());
    }
    if path != root {
        let name = path
            .strip_prefix(root)?
            .to_str()
            .ok_or("non-UTF-8 input path")?
            .replace('\\', "/");
        archive_fs::validate_name(name.as_bytes())?;
        if entries.len() as u64 >= limits.max_entries {
            return Err("entry count budget exceeded".into());
        }
        if meta.is_dir() {
            entries.push(CreateEntry {
                name,
                data: Vec::new(),
                kind: EntryKind::Directory,
            });
        } else if meta.is_file() {
            *total = total.checked_add(meta.len()).ok_or("size overflow")?;
            if meta.len() > limits.max_entry_bytes || *total > limits.max_total_bytes {
                return Err("decoded byte budget exceeded".into());
            }
            let mut options = std::fs::OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc_no_follow());
            }
            let file = options.open(path)?;
            let mut data = Vec::new();
            use std::io::Read;
            file.take(meta.len().checked_add(1).ok_or("size overflow")?)
                .read_to_end(&mut data)?;
            if data.len() as u64 != meta.len() {
                return Err("input changed during enumeration".into());
            }
            entries.push(CreateEntry {
                name,
                data,
                kind: EntryKind::File,
            });
        } else {
            return Err("special input files are unsupported".into());
        }
    }
    if meta.is_dir() {
        let mut children = std::fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
        children.sort_by_key(|e| e.file_name());
        for child in children {
            enumerate(root, &child.path(), entries, total, limits)?;
        }
    } else if path == root {
        return Err("--input must be a directory".into());
    }
    Ok(())
}

#[cfg(unix)]
fn libc_no_follow() -> i32 {
    libc::O_NOFOLLOW
}

struct NativeMedia {
    files: std::collections::BTreeMap<String, PathBuf>,
    max_input: u64,
}
fn extract_batch<O: archive_core::progress::Observer>(
    archive: &mut Archive<std::fs::File>,
    entries: &[archive_core::Entry],
    destination: &mut archive_fs::Destination,
    workers: usize,
    observer: &mut O,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    use archive_core::progress::{Reporter, Stage};
    let total = entries
        .iter()
        .try_fold(0u64, |sum, entry| sum.checked_add(entry.size))
        .ok_or("selected size overflow")?;
    let mut reporter = Reporter::new(observer, Some(total));
    reporter.stage(Stage::Decoding);
    let mut outputs = staging::BatchSpool::new(destination)?;
    let mut ids = Vec::new();
    for entry in entries {
        check_cancelled()?;
        match entry.kind {
            EntryKind::Directory => {
                destination.directory(&entry.raw_name)?;
                outputs.directory_metadata(&entry.raw_name, archive.entry_metadata(entry.id)?);
            }
            EntryKind::File => {
                outputs.stage(entry.id.0, &entry.raw_name, entry.size)?;
                outputs.metadata(entry.id.0, archive.entry_metadata(entry.id)?)?;
                ids.push(entry.id);
            }
            _ => return Err("special archive entry".into()),
        }
    }
    let result = archive.extract_selected_parallel(
        &ids,
        workers,
        &|| CANCELLED.load(std::sync::atomic::Ordering::Relaxed),
        &mut |id, bytes| {
            if CANCELLED.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(archive_core::Error::Cancelled);
            }
            outputs.write(id.0, bytes)?;
            reporter.written(bytes.len() as u64);
            reporter.publish();
            Ok(())
        },
    );
    let batch = match result {
        Ok(report) => report,
        Err(error) => {
            reporter.finish(if matches!(error, archive_core::Error::Cancelled) {
                Stage::Cancelled
            } else {
                Stage::Failed
            });
            return Err(error.into());
        }
    };
    if !batch.report.verified || !outputs.complete() {
        reporter.finish(Stage::Failed);
        return Err(archive_core::Error::Integrity(
            "batch output verification or size mismatch".into(),
        )
        .into());
    }
    if let Err(error) = check_cancelled() {
        reporter.finish(Stage::Cancelled);
        return Err(error.into());
    }
    if archive.format() != Format::Cab {
        reporter.decoded(batch.decoded_bytes);
    }
    reporter.stage(Stage::Verifying);
    if let Err(error) = outputs.publish(destination, || reporter.verified_entry()) {
        reporter.finish(Stage::Failed);
        return Err(error.into());
    }
    reporter.finish(Stage::Complete);
    Ok(
        serde_json::json!({"schema_version":1,"ok":true,"operation":"extract","verified":batch.report.verified,"bytes":batch.report.bytes,"entries":entries.len(),"workers_used":batch.workers_used,"folder_tasks":batch.folder_tasks,"decoded_bytes":batch.decoded_bytes,"sequential_reason":batch.fallback_reason}),
    )
}
impl ms_package::MediaResolver for NativeMedia {
    fn resolve(&mut self, name: &str, max: u64) -> ms_package::Result<Vec<u8>> {
        use io::Read;
        let path = self
            .files
            .get(name)
            .ok_or_else(|| ms_package::Error::MissingMedia(name.into()))?;
        let max = max.min(self.max_input);
        let mut bytes = Vec::new();
        CancellableSource(std::fs::File::open(path)?)
            .take(
                max.checked_add(1)
                    .ok_or(ms_package::Error::Limit("media input bytes"))?,
            )
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > max {
            return Err(ms_package::Error::Limit("media input bytes"));
        }
        Ok(bytes)
    }
}

fn package_operation(
    cli: &Cli,
    path: &Path,
) -> Result<Option<serde_json::Value>, Box<dyn std::error::Error>> {
    let mut extension = path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    // An explicit archive interpretation bypasses extension-based package routing.
    if matches!(
        &cli.command,
        Command::List {
            format: Some(_),
            ..
        } | Command::Test {
            format: Some(_),
            ..
        } | Command::Extract {
            format: Some(_),
            ..
        }
    ) {
        return optical::operation(
            cli,
            path,
            Limits {
                max_input_bytes: cli.max_input_bytes,
                max_entry_bytes: cli.max_entry_bytes,
                max_total_bytes: cli.max_total_bytes,
                max_entries: cli.max_entries,
                ..Limits::default()
            },
        );
    }
    use io::Read;
    let mut signature = Vec::new();
    CancellableSource(std::fs::File::open(path)?)
        .take(8)
        .read_to_end(&mut signature)?;
    if signature == b"MSWIM\0\0\0" {
        extension = "wim".into();
    } else if signature == b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1" {
        extension = "msi".into();
    } else if matches!(extension.as_str(), "wim" | "esd" | "msi") {
        extension.clear();
    }
    if !cli.media.is_empty() && extension != "msi" {
        return Err(
            archive_core::Error::Unsupported("--media requires an MSI package".into()).into(),
        );
    }
    if cli.bundle_entry.is_some() && !matches!(extension.as_str(), "appxbundle" | "msixbundle") {
        return Err(archive_core::Error::Unsupported(
            "--bundle-entry requires an APPX/MSIX bundle".into(),
        )
        .into());
    }
    let limits = Limits {
        max_input_bytes: cli.max_input_bytes,
        max_entry_bytes: cli.max_entry_bytes,
        max_total_bytes: cli.max_total_bytes,
        max_entries: cli.max_entries,
        ..Limits::default()
    };
    if let Some(result) = optical::operation(cli, path, limits)? {
        return Ok(Some(result));
    }
    if matches!(extension.as_str(), "wim" | "esd") {
        let source = std::fs::File::open(path)?;
        if cli.image.is_none()
            && cli.image_name.is_none()
            && matches!(cli.command, Command::List { .. })
        {
            let images = archive_core::wim::images_reader(source, limits)?;
            if !cli.json {
                for image in &images {
                    println!("{} {}", image.index, image.name.as_deref().unwrap_or(""));
                }
            }
            let images: Vec<_> = images.iter().map(|image| serde_json::json!({"index":image.index,"name":image.name,"description":image.description})).collect();
            return Ok(Some(
                serde_json::json!({"schema_version":1,"ok":true,"operation":"list","format":"wim","images":images}),
            ));
        }
        let archive = if let Some(name) = &cli.image_name {
            archive_core::wim::FileWimArchive::open_reader_by_name(source, name, limits)?
        } else {
            archive_core::wim::FileWimArchive::open_reader(
                source,
                cli.image
                    .ok_or("WIM/ESD requires --image or --image-name")?,
                limits,
            )?
        };
        let image = archive.image();
        let result = match &cli.command {
            Command::List { .. } => {
                if !cli.json {
                    for entry in archive.entries() {
                        println!("{:>12} {}", entry.size, entry.name);
                    }
                }
                serde_json::json!({"schema_version":1,"ok":true,"operation":"list","format":"wim","image":archive.image(),"image_count":archive.image_count(),"entries":archive.entries()})
            }
            Command::Test { .. } => {
                let report = archive.test()?;
                serde_json::json!({"schema_version":1,"ok":true,"operation":"test","image":image,"verified":report.verified,"bytes":report.bytes,"entries":report.entries})
            }
            Command::Extract {
                output, threads, ..
            } => {
                if *threads == 0 {
                    return Err("worker count must be positive".into());
                }
                preflight(archive.entries().iter().map(|e| e.name.as_bytes()))?;
                if archive
                    .entries()
                    .iter()
                    .any(|e| !matches!(e.kind, EntryKind::File | EntryKind::Directory))
                {
                    return Err("WIM links and special files are unsupported".into());
                }
                std::fs::create_dir_all(output)?;
                let mut destination = archive_fs::Destination::open(output)?;
                let mut directories = Vec::new();
                for entry in archive.entries() {
                    if cli.verbose {
                        eprintln!("{}", entry.name);
                    }
                    let metadata = archive.entry_metadata(entry.id)?;
                    check_cancelled()?;
                    match entry.kind {
                        EntryKind::Directory => {
                            destination.directory(entry.name.as_bytes())?;
                            directories.push((entry.name.clone(), metadata));
                        }
                        EntryKind::File => {
                            destination.file_with_metadata(
                                entry.name.as_bytes(),
                                &metadata,
                                |sink| {
                                    let report = archive
                                        .extract(entry.id, &mut CancellableSink(sink))
                                        .map_err(io::Error::other)?;
                                    check_cancelled()?;
                                    Ok(report.bytes)
                                },
                            )?;
                        }
                        _ => return Err("WIM special file".into()),
                    }
                }
                directories.sort_by_key(|(name, _)| std::cmp::Reverse(name.split('/').count()));
                for (name, metadata) in directories {
                    destination.directory_metadata(name.as_bytes(), &metadata)?;
                }
                serde_json::json!({"schema_version":1,"ok":true,"operation":"extract","image":image,"verified":true,"workers_used":1})
            }
            _ => return Ok(None),
        };
        return Ok(Some(result));
    }
    if matches!(extension.as_str(), "appxbundle" | "msixbundle") {
        let mut bundle = ms_package::AppxBundle::open(
            std::fs::File::open(path)?,
            package_compat::limits(limits),
            limits.max_metadata_bytes,
        )?;
        if let Some(selected) = &cli.bundle_entry {
            if !matches!(cli.command, Command::List { .. }) {
                bundle.validate(limits.max_total_bytes)?;
            }
            let mut package = bundle.select(
                selected,
                package_compat::limits(limits),
                limits.max_entry_bytes,
                limits.max_metadata_bytes,
            )?;
            let mut result = appx_operation(cli, &mut package, &extension, limits)?;
            result["bundle_entry"] = serde_json::json!(selected);
            return Ok(Some(result));
        }
        if !matches!(cli.command, Command::List { .. }) {
            return Err(archive_core::Error::Unsupported(
                "bundle test/extraction requires exact --bundle-entry selection".into(),
            )
            .into());
        }
        let packages: Vec<_> = bundle.packages().iter().map(|package| serde_json::json!({
            "file_name":package.file_name,"architecture":package.architecture,
            "resource_id":package.resource_id,"package_type":package.package_type,"size":package.size,"version":package.version,
        })).collect();
        if !cli.json {
            for package in bundle.packages() {
                println!(
                    "{:>12} {} {} {}",
                    package.size, package.architecture, package.package_type, package.file_name
                );
            }
        }
        return Ok(Some(
            serde_json::json!({"schema_version":1,"ok":true,"operation":"list","format":extension,"packages":packages}),
        ));
    }
    if matches!(extension.as_str(), "appx" | "msix") {
        let mut package = ms_package::AppxPackage::open(
            std::fs::File::open(path)?,
            package_compat::limits(limits),
            limits.max_metadata_bytes,
        )?;
        return Ok(Some(appx_operation(cli, &mut package, &extension, limits)?));
    }
    if extension == "msi" {
        let mut package = ms_package::InstallerPackage::open(
            std::fs::File::open(path)?,
            limits.max_entries as usize,
        )?;
        let files = package.files()?;
        let result = match &cli.command {
            Command::List { .. } => {
                if !cli.json {
                    for file in &files {
                        println!("{:>12} {}", file.size, file.path);
                    }
                }
                let entries:Vec<_>=files.iter().map(|f|serde_json::json!({"id":f.id,"name":f.path,"size":f.size,"sequence":f.sequence,"cabinet":f.cabinet})).collect();
                serde_json::json!({"schema_version":1,"ok":true,"operation":"list","format":"msi","entries":entries,"tables":package.tables(),"streams":package.streams()})
            }
            Command::Test { .. } | Command::Extract { .. } => {
                preflight(files.iter().map(|f| f.path.as_bytes()))?;
                let mut resolver = NativeMedia {
                    files: std::collections::BTreeMap::new(),
                    max_input: limits.max_input_bytes,
                };
                for media in &cli.media {
                    if resolver
                        .files
                        .insert(media.name.clone(), media.path.clone())
                        .is_some()
                    {
                        return Err("duplicate explicit media mapping".into());
                    }
                }
                let mut total = 0u64;
                let mut destination = if let Command::Extract {
                    output, threads, ..
                } = &cli.command
                {
                    if *threads == 0 {
                        return Err("worker count must be positive".into());
                    }
                    std::fs::create_dir_all(output)?;
                    Some(archive_fs::Destination::open(output)?)
                } else {
                    None
                };
                let mut pending = destination
                    .as_ref()
                    .map(staging::BatchSpool::new)
                    .transpose()?;
                for (id, file) in files.iter().enumerate() {
                    check_cancelled()?;
                    if cli.verbose {
                        eprintln!("{}", file.path);
                    }
                    let bytes = package.read_file(file, &mut resolver, limits.max_entry_bytes)?;
                    total = total
                        .checked_add(bytes.len() as u64)
                        .ok_or("decoded size overflow")?;
                    if total > limits.max_total_bytes {
                        return Err("total decoded bytes limit exceeded".into());
                    }
                    if let Some(spool) = &mut pending {
                        spool.stage(id, file.path.as_bytes(), bytes.len() as u64)?;
                        spool.write(id, &bytes)?;
                    }
                }
                check_cancelled()?;
                if let (Some(spool), Some(destination)) = (pending, &mut destination) {
                    spool.publish(destination, || {})?;
                }
                serde_json::json!({"schema_version":1,"ok":true,"operation":if destination.is_some(){"extract"}else{"test"},"files_verified":files.len(),"bytes":total})
            }
            _ => return Ok(None),
        };
        return Ok(Some(result));
    }
    Ok(None)
}

fn appx_operation<R: io::Read + io::Seek>(
    cli: &Cli,
    package: &mut ms_package::AppxPackage<R>,
    format: &str,
    limits: Limits,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    match &cli.command {
        Command::List { .. } => {
            if !cli.json {
                for entry in package.entries() {
                    println!("{:>12} {}", entry.size, entry.name);
                }
            }
            Ok(
                serde_json::json!({"schema_version":1,"ok":true,"operation":"list","format":format,"entries":package.entries()}),
            )
        }
        Command::Test { .. } => {
            let integrity = package.validate(limits.max_total_bytes)?;
            Ok(
                serde_json::json!({"schema_version":1,"ok":true,"operation":"test","files_verified":integrity.files_verified,"bytes":integrity.bytes_verified,"signature_verified":integrity.signature_verified}),
            )
        }
        Command::Extract {
            output, threads, ..
        } => {
            if *threads == 0 {
                return Err("worker count must be positive".into());
            }
            package.validate(limits.max_total_bytes)?;
            let entries = package.entries().to_vec();
            preflight(entries.iter().map(|e| e.raw_name.as_slice()))?;
            std::fs::create_dir_all(output)?;
            let mut destination = archive_fs::Destination::open(output)?;
            let mut pending = staging::BatchSpool::new(&destination)?;
            for entry in entries {
                check_cancelled()?;
                if cli.verbose {
                    eprintln!("{}", entry.name);
                }
                match entry.kind {
                    package_core::EntryKind::Directory => {
                        destination.directory(&entry.raw_name)?;
                        pending.directory_metadata(
                            &entry.raw_name,
                            package_compat::metadata(package.entry_metadata(entry.id)?),
                        );
                    }
                    package_core::EntryKind::File => {
                        let bytes = package.read_entry(entry.id, limits.max_entry_bytes)?;
                        pending.stage(entry.id.0, &entry.raw_name, entry.size)?;
                        pending.metadata(
                            entry.id.0,
                            package_compat::metadata(package.entry_metadata(entry.id)?),
                        )?;
                        pending.write(entry.id.0, &bytes)?;
                    }
                    _ => return Err("package links are unsupported".into()),
                }
            }
            check_cancelled()?;
            pending.publish(&mut destination, || {})?;
            Ok(
                serde_json::json!({"schema_version":1,"ok":true,"operation":"extract","package_integrity_verified":true,"signature_verified":false}),
            )
        }
        _ => Err(archive_core::Error::Unsupported("package operation".into()).into()),
    }
}

fn preflight<'a>(names: impl Iterator<Item = &'a [u8]>) -> io::Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for name in names {
        if !seen.insert(archive_fs::validate_name(name)?.join("/").to_lowercase()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "duplicate destination path",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod argument_tests {
    use super::*;

    #[test]
    fn short_options_have_no_conflicts_and_parse_global_and_creation_values() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
        let cli = Cli::try_parse_from([
            "arc",
            "-j",
            "-p",
            "password",
            "-b",
            "bundle",
            "-m",
            "cab=data.cab",
            "-I",
            "1",
            "-v",
            "iso",
            "-M",
            "1024",
            "-P",
            "never",
            "create",
            "-f",
            "7z",
            "-i",
            "source",
            "-o",
            "output.7z",
            "-c",
            "copy",
            "-z",
            "zipcrypto",
            "-e",
            "-H",
        ])
        .unwrap();
        assert!(cli.json);
        assert_eq!(cli.password_file.as_deref(), Some(Path::new("password")));
        assert_eq!(cli.bundle_entry.as_deref(), Some("bundle"));
        assert_eq!(cli.media[0].name, "cab");
        assert_eq!(cli.image, Some(1));
        assert_eq!(cli.max_input_bytes, 1024);
        assert!(matches!(cli.view, Some(OpticalView::Iso)));
        assert!(matches!(cli.progress, Progress::Never));
        assert!(matches!(
            cli.command,
            Command::Create {
                compression: Some(ArchiveCompression::Copy),
                zip_encryption: ZipEncryption::Zipcrypto,
                encrypt: true,
                encrypt_headers: true,
                ..
            }
        ));
        let cli = Cli::try_parse_from(["arc", "-n", "Windows", "list", "install.wim"]).unwrap();
        assert_eq!(cli.image_name.as_deref(), Some("Windows"));
    }
}
