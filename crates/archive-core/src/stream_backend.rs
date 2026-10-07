use crate::{Error, Format, Limits, Result, codec, copy_bounded};
use std::io::{Read, Seek, Write};
#[derive(Debug, Clone)]
pub struct GzipHeader {
    pub original_name: Vec<u8>,
    pub comment: Vec<u8>,
    pub extra: Vec<u8>,
    pub modified_unix_seconds: u32,
    pub operating_system: u8,
}
pub(crate) fn gzip_header(reader: &mut (impl Read + Seek), limits: Limits) -> Result<GzipHeader> {
    reader.rewind()?;
    let mut fixed = [0u8; 10];
    reader.read_exact(&mut fixed)?;
    if fixed[..3] != [31, 139, 8] || fixed[3] & 0xe0 != 0 {
        return Err(Error::Malformed("gzip header".into()));
    }
    let mut budget = 10u64;
    let mut extra = Vec::new();
    if fixed[3] & 4 != 0 {
        let mut size = [0u8; 2];
        reader.read_exact(&mut size)?;
        let size = usize::from(u16::from_le_bytes(size));
        budget = budget
            .checked_add(size as u64 + 2)
            .ok_or(Error::ResourceLimit("gzip metadata bytes"))?;
        if budget > limits.max_metadata_bytes {
            return Err(Error::ResourceLimit("gzip metadata bytes"));
        }
        extra.resize(size, 0);
        reader.read_exact(&mut extra)?;
    }
    fn string(reader: &mut impl Read, budget: &mut u64, limit: u64) -> Result<Vec<u8>> {
        let mut value = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            reader.read_exact(&mut byte)?;
            *budget = budget
                .checked_add(1)
                .ok_or(Error::ResourceLimit("gzip metadata bytes"))?;
            if *budget > limit {
                return Err(Error::ResourceLimit("gzip metadata bytes"));
            }
            if byte[0] == 0 {
                break;
            }
            value.push(byte[0]);
        }
        Ok(value)
    }
    let name = if fixed[3] & 8 != 0 {
        string(reader, &mut budget, limits.max_metadata_bytes)?
    } else {
        Vec::new()
    };
    let comment = if fixed[3] & 16 != 0 {
        string(reader, &mut budget, limits.max_metadata_bytes)?
    } else {
        Vec::new()
    };
    reader.rewind()?;
    Ok(GzipHeader {
        original_name: name,
        comment,
        extra,
        modified_unix_seconds: u32::from_le_bytes(
            fixed[4..8]
                .try_into()
                .map_err(|_| Error::Malformed("gzip timestamp".into()))?,
        ),
        operating_system: fixed[9],
    })
}
pub(crate) fn decode(
    reader: &mut (impl Read + Seek),
    writer: &mut impl Write,
    format: Format,
    limits: Limits,
) -> Result<u64> {
    if matches!(format, Format::Gzip | Format::Zlib | Format::Deflate)
        && (limits.max_dictionary_bytes < 32768 || limits.max_active_workspace_bytes < 1 << 20)
    {
        return Err(Error::ResourceLimit("DEFLATE decoder workspace bytes"));
    }
    reader.rewind()?;
    match format {
        #[cfg(feature = "bzip2")]
        Format::Bzip2 => {
            bzip_budget(reader, limits)?;
            let mut decoder = bzip2::read::MultiBzDecoder::new(reader);
            copy_bounded(&mut decoder, writer, limits.max_total_bytes)
        }
        #[cfg(feature = "brotli")]
        Format::Brotli => {
            brotli_budget(reader, limits)?;
            use brotli::CustomRead;
            let (mut decoder, failed) = crate::brotli_budget::brotli_decoder(reader);
            let mut bytes = 0u64;
            let mut output = [0u8; 65536];
            loop {
                let result = decoder.read(&mut output);
                if failed.get() {
                    return Err(Error::ResourceLimit("Brotli allocation bytes"));
                }
                let count = result?;
                if count == 0 {
                    break;
                }
                bytes = bytes
                    .checked_add(count as u64)
                    .ok_or(Error::ResourceLimit("decoded bytes"))?;
                if bytes > limits.max_total_bytes {
                    return Err(Error::ResourceLimit("decoded bytes"));
                }
                writer.write_all(&output[..count])?;
            }
            Ok(bytes)
        }
        Format::Gzip => codec::inflate_window(
            reader,
            writer,
            31,
            limits.max_total_bytes,
            limits.max_entries,
        ),
        Format::Zlib => codec::inflate_window(reader, writer, 15, limits.max_total_bytes, 1),
        Format::Deflate => codec::inflate_window(reader, writer, 0, limits.max_total_bytes, 1),
        Format::Lzma => {
            let mut header = [0u8; 13];
            reader.read_exact(&mut header)?;
            let dictionary = u32::from_le_bytes(
                header[1..5]
                    .try_into()
                    .map_err(|_| Error::Malformed("LZMA dictionary".into()))?,
            );
            if u64::from(dictionary.max(4096)) > limits.max_dictionary_bytes {
                return Err(Error::ResourceLimit("dictionary bytes"));
            }
            reader.rewind()?;
            let memory =
                u32::try_from((limits.max_active_workspace_bytes / 1024).min(u64::from(u32::MAX)))
                    .map_err(|_| Error::ResourceLimit("LZMA workspace"))?;
            let mut decoder = ms_compress::lzma::LzmaReader::new_mem_limit(reader, memory, None)?;
            copy_bounded(&mut decoder, writer, limits.max_total_bytes)
        }
        _ => Err(Error::Unsupported("single-stream format".into())),
    }
}
pub(crate) fn encode(
    data: &[u8],
    writer: &mut impl Write,
    format: Format,
    limits: Limits,
) -> Result<()> {
    if matches!(format, Format::Gzip | Format::Zlib | Format::Deflate)
        && (limits.max_dictionary_bytes < 32768 || limits.max_active_workspace_bytes < 1 << 20)
    {
        return Err(Error::ResourceLimit("DEFLATE encoder workspace bytes"));
    }
    match format {
        #[cfg(feature = "bzip2")]
        Format::Bzip2 => {
            encoder_budget(limits, false)?;
            let mut encoder = bzip2::write::BzEncoder::new(writer, bzip2::Compression::new(9));
            encoder.write_all(data)?;
            encoder.finish()?;
            Ok(())
        }
        #[cfg(feature = "brotli")]
        Format::Brotli => {
            encoder_budget(limits, true)?;
            let mut encoder = brotli::CompressorWriter::new(writer, 65536, 5, 22);
            encoder.write_all(data)?;
            encoder.flush()?;
            Ok(())
        }
        Format::Gzip => codec::deflate_window(data, writer, 31),
        Format::Zlib => codec::deflate_window(data, writer, 15),
        Format::Deflate => codec::deflate_window(data, writer, -15),
        Format::Lzma => {
            let mut options = ms_compress::lzma::LzmaOptions::with_preset(6);
            options.dict_size = options
                .dict_size
                .min(data.len().max(4096).min(u32::MAX as usize) as u32);
            options.dict_size = (0u8..40)
                .map(|property| (2u64 | u64::from(property & 1)) << (property / 2 + 11))
                .find(|size| *size >= u64::from(options.dict_size))
                .and_then(|size| u32::try_from(size).ok())
                .ok_or(Error::ResourceLimit("dictionary bytes"))?;
            if u64::from(options.dict_size) > limits.max_dictionary_bytes
                || u64::from(options.get_memory_usage()) * 1024 > limits.max_active_workspace_bytes
            {
                return Err(Error::ResourceLimit("LZMA encoder workspace"));
            }
            let mut encoder = ms_compress::lzma::LzmaWriter::new_use_header(
                writer,
                &options,
                Some(data.len() as u64),
            )?;
            encoder.write_all(data)?;
            encoder.finish()?;
            Ok(())
        }
        _ => Err(Error::Unsupported("single-stream format".into())),
    }
}
#[cfg(feature = "bzip2")]
fn bzip_budget(reader: &mut (impl Read + Seek), limits: Limits) -> Result<()> {
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    reader.rewind()?;
    if &header[..3] != b"BZh" || !(b'1'..=b'9').contains(&header[3]) {
        return Err(Error::Malformed("BZip2 header".into()));
    }
    if limits.max_dictionary_bytes < 900_000 || limits.max_active_workspace_bytes < 16 << 20 {
        return Err(Error::ResourceLimit("BZip2 workspace"));
    }
    Ok(())
}
#[cfg(feature = "brotli")]
fn brotli_budget(reader: &mut (impl Read + Seek), limits: Limits) -> Result<()> {
    let mut header = [0];
    reader.read_exact(&mut header)?;
    reader.rewind()?;
    let byte = header[0];
    let window = if byte & 1 == 0 {
        16
    } else if byte >> 1 & 7 != 0 {
        17 + (byte >> 1 & 7)
    } else {
        match byte >> 4 & 7 {
            1 => return Err(Error::Unsupported("Brotli large-window extension".into())),
            0 => 17,
            value => 8 + value,
        }
    };
    if (1u64 << window) > limits.max_dictionary_bytes
        || limits.max_active_workspace_bytes < 96 << 20
    {
        return Err(Error::ResourceLimit("Brotli workspace"));
    }
    Ok(())
}
#[cfg(any(feature = "bzip2", feature = "brotli"))]
fn encoder_budget(limits: Limits, brotli: bool) -> Result<()> {
    let (dictionary, workspace) = if brotli {
        (1 << 22, 96 << 20)
    } else {
        (900_000, 16 << 20)
    };
    if limits.max_dictionary_bytes < dictionary || limits.max_active_workspace_bytes < workspace {
        return Err(Error::ResourceLimit("compression workspace"));
    }
    Ok(())
}
#[cfg(any(feature = "bzip2", feature = "brotli"))]
pub(crate) fn encode_tar(
    entries: &[crate::CreateEntry],
    writer: &mut impl Write,
    format: Format,
    limits: Limits,
) -> Result<()> {
    encode_tar_with_metadata(entries, writer, format, limits, None)
}
#[cfg(any(feature = "bzip2", feature = "brotli"))]
pub(crate) fn encode_tar_with_metadata(
    entries: &[crate::CreateEntry],
    writer: &mut impl Write,
    format: Format,
    limits: Limits,
    metadata: Option<&[crate::EntryMetadata]>,
) -> Result<()> {
    match format {
        #[cfg(feature = "bzip2")]
        Format::TarBzip2 => {
            encoder_budget(limits, false)?;
            let mut encoder = bzip2::write::BzEncoder::new(writer, bzip2::Compression::new(9));
            crate::tar_backend::create_with_metadata(entries, &mut encoder, metadata)?;
            encoder.finish()?;
            Ok(())
        }
        #[cfg(feature = "brotli")]
        Format::TarBrotli => {
            encoder_budget(limits, true)?;
            let mut encoder = brotli::CompressorWriter::new(writer, 65536, 5, 22);
            crate::tar_backend::create_with_metadata(entries, &mut encoder, metadata)?;
            encoder.flush()?;
            Ok(())
        }
        _ => Err(Error::Unsupported("compressed TAR format".into())),
    }
}
pub(crate) fn encode_named_gzip(
    data: &[u8],
    name: &str,
    writer: &mut impl Write,
    limits: Limits,
) -> Result<()> {
    encode_named_gzip_with_mtime(data, name, writer, limits, None)
}
pub(crate) fn encode_named_gzip_with_mtime(
    data: &[u8],
    name: &str,
    writer: &mut impl Write,
    limits: Limits,
    modified: Option<crate::StoredTimestamp>,
) -> Result<()> {
    if name.as_bytes().contains(&0) {
        return Err(Error::Malformed("gzip filename contains NUL".into()));
    }
    if (name.len() as u64)
        .checked_add(11)
        .is_none_or(|size| size > limits.max_metadata_bytes)
    {
        return Err(Error::ResourceLimit("gzip metadata bytes"));
    }
    if limits.max_dictionary_bytes < 32768 || limits.max_active_workspace_bytes < 1 << 20 {
        return Err(Error::ResourceLimit("DEFLATE encoder workspace bytes"));
    }
    let modified = match modified {
        Some(crate::StoredTimestamp::UnixSeconds(seconds)) => u32::try_from(seconds)
            .map_err(|_| Error::Unsupported("gzip timestamp exceeds 32 bits".into()))?,
        _ => 0,
    };
    writer.write_all(&[31, 139, 8, 8])?;
    writer.write_all(&modified.to_le_bytes())?;
    writer.write_all(&[0, 255])?;
    writer.write_all(name.as_bytes())?;
    writer.write_all(&[0])?;
    codec::deflate(data, writer, false)?;
    writer.write_all(&ms_compress::zlib::crc32::crc32(0, data).to_le_bytes())?;
    writer.write_all(&(data.len() as u32).to_le_bytes())?;
    Ok(())
}
