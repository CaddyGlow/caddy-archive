use crate::{Error, Result};
use ms_compress::zlib::{Deflate, DeflateConfig, DeflateFlush, Inflate, InflateFlush, Status};
use std::io::{Read, Write};
#[cfg(feature = "zip")]
pub(crate) fn inflate(
    reader: &mut impl Read,
    writer: &mut impl Write,
    gzip: bool,
    limit: u64,
) -> Result<u64> {
    inflate_window(reader, writer, if gzip { 31 } else { 0 }, limit, 1)
}
pub(crate) fn inflate_window(
    reader: &mut impl Read,
    writer: &mut impl Write,
    window: u8,
    limit: u64,
    max_members: u64,
) -> Result<u64> {
    let mut decoder = Inflate::new(window != 0, if window == 0 { 15 } else { window });
    let mut completed = 0u64;
    let mut members = 1u64;
    let mut input = [0u8; 65536];
    let mut output = [0u8; 65536];
    let mut start = 0;
    let mut end = 0;
    let mut eof = false;
    loop {
        if start == end && !eof {
            end = reader.read(&mut input)?;
            start = 0;
            eof = end == 0;
        }
        let before_in = decoder.total_in();
        let before_out = decoder.total_out();
        let status = decoder
            .decompress(&input[start..end], &mut output, InflateFlush::NoFlush)
            .map_err(|e| Error::Integrity(e.as_str().into()))?;
        start += usize::try_from(decoder.total_in() - before_in)
            .map_err(|_| Error::ResourceLimit("input offset"))?;
        let produced = usize::try_from(decoder.total_out() - before_out)
            .map_err(|_| Error::ResourceLimit("output offset"))?;
        if completed
            .checked_add(decoder.total_out())
            .is_none_or(|total| total > limit)
        {
            return Err(Error::ResourceLimit("decoded bytes"));
        }
        writer.write_all(&output[..produced])?;
        if status == Status::StreamEnd {
            if start == end {
                end = reader.read(&mut input)?;
                start = 0;
            }
            if start == end {
                return completed
                    .checked_add(decoder.total_out())
                    .ok_or(Error::ResourceLimit("decoded bytes"));
            }
            if window != 31 || max_members == 1 {
                return Err(Error::Unsupported(
                    "trailing or concatenated compressed streams".into(),
                ));
            }
            completed = completed
                .checked_add(decoder.total_out())
                .ok_or(Error::ResourceLimit("decoded bytes"))?;
            members += 1;
            if members > max_members {
                return Err(Error::ResourceLimit("gzip members"));
            }
            decoder = Inflate::new(true, 31);
            continue;
        }
        if decoder.total_in() == before_in && produced == 0 {
            return Err(Error::Integrity("truncated compressed stream".into()));
        }
    }
}
pub(crate) fn deflate(data: &[u8], writer: &mut impl Write, gzip: bool) -> Result<()> {
    deflate_window(data, writer, if gzip { 31 } else { -15 })
}
pub(crate) fn deflate_window(data: &[u8], writer: &mut impl Write, window: i32) -> Result<()> {
    let mut decoder = Deflate::new_with_config(DeflateConfig {
        window_bits: window,
        ..Default::default()
    });
    let mut output = [0u8; 65536];
    let mut start = 0;
    loop {
        let before_in = decoder.total_in();
        let before_out = decoder.total_out();
        let status = decoder
            .compress(&data[start..], &mut output, DeflateFlush::Finish)
            .map_err(|e| Error::Malformed(e.as_str().into()))?;
        start += usize::try_from(decoder.total_in() - before_in)
            .map_err(|_| Error::ResourceLimit("input offset"))?;
        let produced = usize::try_from(decoder.total_out() - before_out)
            .map_err(|_| Error::ResourceLimit("output offset"))?;
        writer.write_all(&output[..produced])?;
        if status == Status::StreamEnd {
            return Ok(());
        }
        if produced == 0 && decoder.total_in() == before_in {
            return Err(Error::Malformed("compression stalled".into()));
        }
    }
}
#[cfg(any(feature = "gzip", feature = "streams"))]
pub(crate) struct DeflateWriter<W> {
    writer: W,
    encoder: Deflate,
}
#[cfg(any(feature = "gzip", feature = "streams"))]
impl<W: Write> DeflateWriter<W> {
    pub(crate) fn new(writer: W, window: i32) -> Self {
        Self {
            writer,
            encoder: Deflate::new_with_config(DeflateConfig {
                window_bits: window,
                ..Default::default()
            }),
        }
    }
    pub(crate) fn finish(mut self) -> Result<W> {
        let mut bytes = [0u8; 65536];
        loop {
            let before = self.encoder.total_out();
            let status = self
                .encoder
                .compress(&[], &mut bytes, DeflateFlush::Finish)
                .map_err(|error| Error::Malformed(error.as_str().into()))?;
            let size = (self.encoder.total_out() - before) as usize;
            self.writer.write_all(&bytes[..size])?;
            if status == Status::StreamEnd {
                return Ok(self.writer);
            }
            if size == 0 {
                return Err(Error::Malformed("compressor failed to finish".into()));
            }
        }
    }
}
#[cfg(any(feature = "gzip", feature = "streams"))]
impl<W: Write> Write for DeflateWriter<W> {
    fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
        let mut remaining = input;
        let mut bytes = [0u8; 65536];
        while !remaining.is_empty() {
            let before_in = self.encoder.total_in();
            let before_out = self.encoder.total_out();
            self.encoder
                .compress(remaining, &mut bytes, DeflateFlush::NoFlush)
                .map_err(|error| std::io::Error::other(error.as_str()))?;
            let consumed = (self.encoder.total_in() - before_in) as usize;
            let produced = (self.encoder.total_out() - before_out) as usize;
            self.writer.write_all(&bytes[..produced])?;
            if consumed == 0 && produced == 0 {
                return Err(std::io::Error::other("compression stalled"));
            }
            remaining = &remaining[consumed..];
        }
        Ok(input.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}
