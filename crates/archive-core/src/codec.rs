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
#[cfg(any(feature = "zip", feature = "gzip", feature = "streams"))]
pub(crate) fn inflate_window(
    reader: &mut impl Read,
    writer: &mut impl Write,
    window: u8,
    limit: u64,
    max_members: u64,
) -> Result<u64> {
    let mut decoder = InflateReader::new(reader, window, limit, max_members);
    let mut output = [0; 65536];
    let mut total = 0;
    loop {
        let count = decoder.read_inner(&mut output)?;
        if count == 0 {
            return Ok(total);
        }
        writer.write_all(&output[..count])?;
        total += count as u64;
    }
}
/// Incremental inflater; EOF verifies the checksum and rejects trailing data.
pub(crate) struct InflateReader<R> {
    source: R,
    decoder: Inflate,
    input: Box<[u8; 65536]>,
    start: usize,
    end: usize,
    window: u8,
    limit: u64,
    max_members: u64,
    members: u64,
    completed: u64,
    ended: bool,
    finished: bool,
}
impl<R: Read> InflateReader<R> {
    pub(crate) fn new(source: R, window: u8, limit: u64, max_members: u64) -> Self {
        Self {
            source,
            decoder: Inflate::new(window != 0, if window == 0 { 15 } else { window }),
            input: Box::new([0; 65536]),
            start: 0,
            end: 0,
            window,
            limit,
            max_members,
            members: 1,
            completed: 0,
            ended: false,
            finished: false,
        }
    }
    fn read_inner(&mut self, output: &mut [u8]) -> Result<usize> {
        if output.is_empty() || self.finished {
            return Ok(0);
        }
        loop {
            if self.start == self.end {
                self.end = self.source.read(&mut self.input[..])?;
                self.start = 0;
            }
            if self.ended {
                if self.start == self.end {
                    self.finished = true;
                    return Ok(0);
                }
                if self.window != 31 || self.max_members == 1 {
                    return Err(Error::Unsupported(
                        "trailing or concatenated compressed streams".into(),
                    ));
                }
                self.members += 1;
                if self.members > self.max_members {
                    return Err(Error::ResourceLimit("gzip members"));
                }
                self.completed = self
                    .completed
                    .checked_add(self.decoder.total_out())
                    .ok_or(Error::ResourceLimit("decoded bytes"))?;
                self.decoder = Inflate::new(true, 31);
                self.ended = false;
            }
            let before_in = self.decoder.total_in();
            let before_out = self.decoder.total_out();
            let status = self
                .decoder
                .decompress(
                    &self.input[self.start..self.end],
                    output,
                    InflateFlush::NoFlush,
                )
                .map_err(|e| Error::Integrity(e.as_str().into()))?;
            self.start += (self.decoder.total_in() - before_in) as usize;
            let produced = (self.decoder.total_out() - before_out) as usize;
            if self
                .completed
                .checked_add(self.decoder.total_out())
                .is_none_or(|total| total > self.limit)
            {
                return Err(Error::ResourceLimit("decoded bytes"));
            }
            self.ended = status == Status::StreamEnd;
            if produced != 0 {
                return Ok(produced);
            }
            if !self.ended && self.decoder.total_in() == before_in {
                return Err(Error::Integrity("truncated compressed stream".into()));
            }
        }
    }
}
impl<R: Read> Read for InflateReader<R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        self.read_inner(output).map_err(std::io::Error::other)
    }
}

#[cfg(any(feature = "gzip", feature = "streams", all(test, feature = "zip")))]
pub(crate) fn deflate(data: &[u8], writer: &mut impl Write, gzip: bool) -> Result<()> {
    deflate_window(data, writer, if gzip { 31 } else { -15 })
}
#[cfg(any(feature = "gzip", feature = "streams", all(test, feature = "zip")))]
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
#[cfg(any(
    feature = "gzip",
    feature = "streams",
    feature = "zip",
    feature = "sevenz"
))]
pub(crate) struct DeflateWriter<W> {
    writer: W,
    encoder: Deflate,
}
#[cfg(any(
    feature = "gzip",
    feature = "streams",
    feature = "zip",
    feature = "sevenz"
))]
impl<W: Write> DeflateWriter<W> {
    pub(crate) fn new(writer: W, window: i32) -> Self {
        Self::with_level(writer, window, 6)
    }
    pub(crate) fn with_level(writer: W, window: i32, level: u8) -> Self {
        Self {
            writer,
            encoder: Deflate::new_with_config(DeflateConfig {
                level: i32::from(level),
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
#[cfg(any(
    feature = "gzip",
    feature = "streams",
    feature = "zip",
    feature = "sevenz"
))]
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
