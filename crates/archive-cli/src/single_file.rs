use super::*;
use archive_core::single_stream::{Codec, Options};
use std::io::Read;

pub(crate) fn run(
    cli: &Cli,
    limits: Limits,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let (decode, format, input, output, output_size, dictionary_bytes, window_order) =
        match &cli.command {
            Command::Compress {
                format,
                input,
                output,
                output_size,
                dictionary_bytes,
                window_order,
            } => (
                false,
                format,
                input,
                output,
                output_size,
                dictionary_bytes,
                window_order,
            ),
            Command::Decompress {
                format,
                input,
                output,
                output_size,
                dictionary_bytes,
                window_order,
            } => (
                true,
                format,
                input,
                output,
                output_size,
                dictionary_bytes,
                window_order,
            ),
            _ => unreachable!(),
        };
    if cli.password_file.is_some() {
        return Err("single-file codecs do not support passwords".into());
    }
    if cli.json && output == Path::new("-") {
        return Err("binary stdout cannot combine with JSON".into());
    }
    let source: Box<dyn io::Read> = if input == Path::new("-") {
        Box::new(io::stdin())
    } else {
        Box::new(std::fs::File::open(input)?)
    };
    let mut source = CancellableSource(source);
    let codec = if let Some(name) = format {
        Codec::parse(name)?
    } else if decode {
        let mut prefix = Vec::new();
        (&mut source).take(13).read_to_end(&mut prefix)?;
        let detected = match archive_core::probe(&prefix).ok() {
            Some(Format::Gzip) => Some(Codec::Gzip),
            Some(Format::Zlib) => Some(Codec::Zlib),
            Some(Format::Xz) => Some(Codec::Xz),
            Some(Format::Bzip2) => Some(Codec::Bzip2),
            _ => None,
        };
        source = CancellableSource(Box::new(io::Cursor::new(prefix).chain(source)));
        detected
            .or_else(|| {
                input
                    .extension()
                    .and_then(|s| s.to_str())
                    .and_then(|s| Codec::parse(s).ok())
            })
            .ok_or("cannot identify single-file codec; specify --format")?
    } else {
        output
            .extension()
            .and_then(|s| s.to_str())
            .and_then(|s| Codec::parse(s).ok())
            .ok_or("cannot infer single-file codec from output extension; specify --format")?
    };
    let options = Options {
        output_size: *output_size,
        dictionary_bytes: *dictionary_bytes,
        window_order: *window_order,
    };
    let mut transform = |writer: &mut dyn io::Write| {
        let mut writer = writer;
        if decode {
            archive_core::single_stream::decompress(
                &mut source,
                &mut writer,
                codec,
                options,
                limits,
            )
        } else {
            archive_core::single_stream::compress(&mut source, &mut writer, codec, options, limits)
        }
    };
    let bytes = if output == Path::new("-") {
        transform(&mut io::stdout().lock())?
    } else {
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        let bytes = transform(&mut CancellableSink(temp.as_file_mut()))?;
        check_cancelled()?;
        temp.as_file().sync_all()?;
        temp.persist_noclobber(output)?;
        bytes
    };
    Ok(
        serde_json::json!({"schema_version":1,"ok":true,"operation":if decode {"decompress"} else {"compress"},"bytes":bytes}),
    )
}
