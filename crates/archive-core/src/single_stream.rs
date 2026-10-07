//! Single-file codecs. Raw codecs carry no archive metadata or framing.
use crate::{Error, Format, Limits, Result, copy_bounded};
use std::io::{Cursor, Read, Write};

/// A stream or independent Windows compression block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Deflate,
    Gzip,
    Zlib,
    Lzma,
    Lzma2,
    Xz,
    Bzip2,
    Brotli,
    /// XPRESS LZ77 with Huffman coding (the WIM variant).
    Xpress,
    /// Plain XPRESS LZ77 (the Windows buffer variant).
    XpressPlain,
    Lzx,
    Lzms,
    Lznt1,
    Quantum,
}
impl Codec {
    /// Parse a codec name or common filename suffix.
    pub fn parse(name: &str) -> Result<Self> {
        Ok(match name.to_ascii_lowercase().as_str() {
            "deflate" => Self::Deflate,
            "gz" | "gzip" => Self::Gzip,
            "zlib" => Self::Zlib,
            "lzma" => Self::Lzma,
            "lzma2" => Self::Lzma2,
            "xz" => Self::Xz,
            "bz2" | "bzip2" => Self::Bzip2,
            "br" | "brotli" => Self::Brotli,
            "xpress" | "xpress-huffman" => Self::Xpress,
            "xpress-plain" => Self::XpressPlain,
            "lzx" => Self::Lzx,
            "lzms" => Self::Lzms,
            "lznt1" => Self::Lznt1,
            "quantum" => Self::Quantum,
            _ => return Err(Error::Unsupported(format!("single-file codec {name:?}"))),
        })
    }
    fn format(self) -> Option<Format> {
        Some(match self {
            Self::Deflate => Format::Deflate,
            Self::Gzip => Format::Gzip,
            Self::Zlib => Format::Zlib,
            Self::Lzma => Format::Lzma,
            Self::Xz => Format::Xz,
            Self::Bzip2 => Format::Bzip2,
            Self::Brotli => Format::Brotli,
            _ => return None,
        })
    }
}
/// Settings that raw streams cannot describe themselves.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Required exact decoded length for Windows blocks.
    pub output_size: Option<u64>,
    /// LZMA2 dictionary size; must agree with the encoder.
    pub dictionary_bytes: u32,
    /// LZX/Quantum window order; must agree with the encoder.
    pub window_order: u8,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            output_size: None,
            dictionary_bytes: 8 << 20,
            window_order: 15,
        }
    }
}
fn malformed(error: impl std::fmt::Display) -> Error {
    Error::Malformed(error.to_string())
}
fn workspace(bytes: u64, limits: Limits) -> Result<()> {
    if bytes > limits.max_active_workspace_bytes {
        return Err(Error::ResourceLimit("single-file workspace bytes"));
    }
    Ok(())
}
fn buffer(reader: &mut impl Read, maximum: u64) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    let mut chunk = [0; 65536];
    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            return Ok(data);
        }
        if data.len() as u64 + count as u64 > maximum {
            return Err(Error::ResourceLimit("single-file input bytes"));
        }
        data.try_reserve_exact(count)
            .map_err(|_| Error::ResourceLimit("single-file allocation"))?;
        data.extend_from_slice(&chunk[..count]);
    }
}
fn window(codec: Codec, options: Options, limits: Limits) -> Result<usize> {
    let range = if codec == Codec::Quantum {
        10..=21
    } else {
        15..=21
    };
    if !range.contains(&options.window_order) {
        return Err(Error::Unsupported("invalid window order".into()));
    }
    let size = 1usize << options.window_order;
    if size as u64 > limits.max_dictionary_bytes {
        return Err(Error::ResourceLimit("dictionary bytes"));
    }
    Ok(size)
}
/// Compress one file. Windows codecs emit a single raw block, not a WIM/CAB.
/// Input buffering and encoder workspaces are bounded by `limits`.
pub fn compress(
    reader: &mut impl Read,
    writer: &mut impl Write,
    codec: Codec,
    options: Options,
    limits: Limits,
) -> Result<u64> {
    let mut limits = limits;
    limits.max_total_bytes = limits.max_total_bytes.min(limits.max_entry_bytes);
    if options.output_size.is_some() {
        return Err(Error::Unsupported(
            "--output-size is only for decompression".into(),
        ));
    }
    if matches!(codec, Codec::Deflate | Codec::Gzip | Codec::Zlib) {
        return crate::deflate_stream(
            reader,
            writer,
            codec
                .format()
                .ok_or_else(|| Error::Unsupported("DEFLATE format".into()))?,
            limits,
        );
    }
    let block_maximum = match codec {
        Codec::Xpress => 65536,
        Codec::Lzx => window(codec, options, limits)? as u64,
        Codec::Quantum => 32768,
        _ => u64::MAX,
    };
    let maximum = limits
        .max_input_bytes
        .min(limits.max_total_bytes)
        .min(limits.max_entry_bytes)
        .min(limits.max_active_workspace_bytes / 4)
        .min(block_maximum);
    let data = buffer(reader, maximum)?;
    let size = data.len() as u64;
    let mut remaining = limits;
    remaining.max_active_workspace_bytes =
        remaining.max_active_workspace_bytes.saturating_sub(size);
    if let Some(format) = codec.format() {
        if format == Format::Xz {
            #[cfg(feature = "xz")]
            crate::xz_backend::encode(&data, writer, remaining)?;
            #[cfg(not(feature = "xz"))]
            return Err(Error::Unsupported("XZ feature unavailable".into()));
        } else {
            crate::stream_backend::encode(&data, writer, format, remaining)?;
        }
        return Ok(size);
    }
    if codec == Codec::Lzma2 {
        let mut settings = ms_compress::lzma::Lzma2Options::with_preset(6);
        settings.lzma_options.dict_size = options.dictionary_bytes;
        if !(4096..=64 << 20).contains(&options.dictionary_bytes)
            || u64::from(options.dictionary_bytes) > limits.max_dictionary_bytes
        {
            return Err(Error::ResourceLimit("LZMA2 dictionary bytes"));
        }
        workspace(
            u64::from(settings.lzma_options.get_memory_usage()) * 1024,
            remaining,
        )?;
        let mut encoder = ms_compress::lzma::Lzma2Writer::new(writer, settings);
        encoder.write_all(&data)?;
        encoder.finish()?;
        return Ok(size);
    }
    // Conservative bound covers token arrays, match tables, and both output buffers.
    let encoder_extent = if codec == Codec::Lzx {
        size.max(window(codec, options, limits)? as u64)
    } else {
        size
    };
    workspace(
        32 * 1024 * 1024 + encoder_extent * if codec == Codec::Lzx { 64 } else { 128 },
        remaining,
    )?;
    let capacity = data
        .len()
        .checked_mul(8)
        .and_then(|n| n.checked_add(4096))
        .ok_or(Error::ResourceLimit("compressed block bytes"))?;
    let encoded = match codec {
        Codec::Xpress => {
            let mut encoder = ms_compress::xpress_encode::XpressCompressor::new(data.len().max(1))
                .map_err(malformed)?;
            encoder
                .compress_block(&data, capacity)
                .map_err(malformed)?
                .ok_or_else(|| Error::Unsupported("XPRESS requires a nonempty block".into()))?
                .to_vec()
        }
        Codec::Lzx => {
            ms_compress::lzx_encode::compress_lzx(&data, capacity, window(codec, options, limits)?)
                .map_err(malformed)?
                .ok_or_else(|| Error::Unsupported("LZX requires a nonempty block".into()))?
        }
        Codec::Lzms => ms_compress::lzms::encode::compress_lzms(&data, capacity)
            .map_err(malformed)?
            .ok_or_else(|| Error::Unsupported("LZMS requires a nonempty block".into()))?,
        Codec::XpressPlain => ms_compress::xpress_plain::compress(&data).map_err(malformed)?,
        Codec::Lznt1 => ms_compress::lznt1::compress(&data).map_err(malformed)?,
        Codec::Quantum => {
            window(codec, options, limits)?;
            ms_compress::quantum::QuantumEncoder::new(options.window_order, 6)
                .map_err(malformed)?
                .compress_frame(&data)
                .map_err(malformed)?
        }
        _ => unreachable!(),
    };
    writer.write_all(&encoded)?;
    Ok(size)
}
/// Decode a single file. Windows blocks require the exact `output_size`.
/// Raw codecs have no checksum and do not establish data integrity.
pub fn decompress(
    reader: &mut impl Read,
    writer: &mut impl Write,
    codec: Codec,
    options: Options,
    limits: Limits,
) -> Result<u64> {
    if matches!(codec, Codec::Deflate | Codec::Gzip | Codec::Zlib) {
        if options.output_size.is_some() {
            return Err(Error::Unsupported(
                "--output-size is only for raw Windows blocks".into(),
            ));
        }
        let mut limits = limits;
        limits.max_total_bytes = limits.max_total_bytes.min(limits.max_entry_bytes);
        return crate::inflate_stream(
            reader,
            writer,
            codec
                .format()
                .ok_or_else(|| Error::Unsupported("DEFLATE format".into()))?,
            limits,
        );
    }
    let data = buffer(
        reader,
        limits
            .max_input_bytes
            .min(limits.max_active_workspace_bytes / 4),
    )?;
    let mut remaining = limits;
    remaining.max_active_workspace_bytes = remaining
        .max_active_workspace_bytes
        .saturating_sub(data.len() as u64);
    remaining.max_total_bytes = limits.max_total_bytes.min(limits.max_entry_bytes);
    if let Some(format) = codec.format() {
        if options.output_size.is_some() {
            return Err(Error::Unsupported(
                "--output-size is only for raw Windows blocks".into(),
            ));
        }
        let mut input = Cursor::new(data);
        return if format == Format::Xz {
            #[cfg(feature = "xz")]
            {
                crate::xz_backend::decode(&mut input, writer, remaining)
            }
            #[cfg(not(feature = "xz"))]
            {
                Err(Error::Unsupported("XZ feature unavailable".into()))
            }
        } else {
            crate::stream_backend::decode(&mut input, writer, format, remaining)
        };
    }
    if codec == Codec::Lzma2 {
        if options.output_size.is_some() {
            return Err(Error::Unsupported(
                "LZMA2 does not require --output-size".into(),
            ));
        }
        if !(4096..=64 << 20).contains(&options.dictionary_bytes)
            || u64::from(options.dictionary_bytes) > limits.max_dictionary_bytes
        {
            return Err(Error::ResourceLimit("LZMA2 dictionary bytes"));
        }
        workspace(
            u64::from(ms_compress::lzma::lzma2_get_memory_usage(
                options.dictionary_bytes,
            )) * 1024,
            remaining,
        )?;
        let mut decoder =
            ms_compress::lzma::Lzma2Reader::new(Cursor::new(data), options.dictionary_bytes, None);
        return copy_bounded(&mut decoder, writer, remaining.max_total_bytes);
    }
    let size = options
        .output_size
        .ok_or_else(|| Error::Unsupported("raw Windows codecs require --output-size".into()))?;
    if size > remaining.max_total_bytes {
        return Err(Error::ResourceLimit("decoded block bytes"));
    }
    let block_maximum = match codec {
        Codec::Xpress => 65536,
        Codec::Lzx => window(codec, options, limits)? as u64,
        Codec::Quantum => 32768,
        _ => u64::MAX,
    };
    if size > block_maximum {
        return Err(Error::ResourceLimit("codec block bytes"));
    }
    workspace(size + 32 * 1024 * 1024, remaining)?;
    let count = usize::try_from(size).map_err(|_| Error::ResourceLimit("decoded block bytes"))?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(count)
        .map_err(|_| Error::ResourceLimit("decoded block allocation"))?;
    output.resize(count, 0);
    match codec {
        Codec::Xpress => ms_compress::decompress_xpress(&data, &mut output).map_err(malformed)?,
        Codec::Lzx => {
            ms_compress::lzx::decompress_lzx(&data, &mut output, window(codec, options, limits)?)
                .map_err(malformed)?
        }
        Codec::Lzms => ms_compress::lzms::decompress_lzms(&data, &mut output).map_err(malformed)?,
        Codec::XpressPlain | Codec::Lznt1 => {
            let written = if codec == Codec::Lznt1 {
                ms_compress::lznt1::decompress(&data, &mut output).map_err(malformed)?
            } else {
                ms_compress::xpress_plain::decompress(&data, &mut output).map_err(malformed)?
            };
            if written != count {
                return Err(Error::Malformed("decoded block size mismatch".into()));
            }
        }
        Codec::Quantum => {
            window(codec, options, limits)?;
            ms_compress::quantum::QuantumDecoder::new(options.window_order)
                .map_err(malformed)?
                .decompress_frame(&data, &mut output)
                .map_err(malformed)?;
        }
        _ => unreachable!(),
    }
    writer.write_all(&output)?;
    Ok(size)
}
