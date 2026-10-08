//! Single-pass compressed-stream detection and provisional TAR retention.
use crate::{Entry, EntryId, EntryKind, Format, Limits, Result};
use std::io::{self, Read, Seek, Write};

pub(crate) struct Profile {
    pub raw: Format,
    #[cfg(feature = "tar")]
    pub tar: Option<Format>,
    retain: Option<bool>,
}

pub(crate) fn profile(format: Format, requested: Option<Format>) -> Option<Profile> {
    let (raw, tar) = match format {
        #[cfg(any(feature = "gzip", feature = "streams"))]
        Format::Gzip | Format::TarGzip => (
            Format::Gzip,
            cfg!(feature = "gzip").then_some(Format::TarGzip),
        ),
        #[cfg(feature = "xz")]
        Format::Xz | Format::TarXz => (Format::Xz, Some(Format::TarXz)),
        #[cfg(feature = "streams")]
        Format::Bzip2 | Format::TarBzip2 => (
            Format::Bzip2,
            cfg!(feature = "tar").then_some(Format::TarBzip2),
        ),
        #[cfg(feature = "streams")]
        Format::Brotli | Format::TarBrotli => (
            Format::Brotli,
            cfg!(feature = "tar").then_some(Format::TarBrotli),
        ),
        #[cfg(feature = "streams")]
        Format::Zlib | Format::Deflate | Format::Lzma => (format, None),
        _ => return None,
    };
    let retain = if tar.is_some_and(|tar| format == tar) {
        Some(true)
    } else if tar.is_none() || requested.is_some() || raw == Format::Brotli {
        Some(false)
    } else {
        None
    };
    Some(Profile {
        raw,
        #[cfg(feature = "tar")]
        tar,
        retain,
    })
}

pub(crate) fn decode_raw(
    reader: &mut (impl Read + Seek),
    writer: &mut impl Write,
    format: Format,
    limits: Limits,
) -> Result<u64> {
    #[cfg(feature = "xz")]
    if format == Format::Xz {
        return crate::xz_backend::decode(reader, writer, limits);
    }
    #[cfg(any(feature = "gzip", feature = "streams"))]
    {
        crate::stream_backend::decode(reader, writer, format, limits)
    }
    #[cfg(not(any(feature = "gzip", feature = "streams")))]
    {
        let _ = (reader, writer, format, limits);
        Err(crate::Error::Unsupported("compressed stream codec".into()))
    }
}

impl Profile {
    pub(crate) fn decode(
        &self,
        reader: &mut (impl Read + Seek),
        output: &mut impl Write,
        limits: Limits,
    ) -> Result<(u64, bool)> {
        let mut sink = TarSink {
            output,
            prefix: [0; 512],
            used: 0,
            retain: self.retain,
        };
        let size = decode_raw(reader, &mut sink, self.raw, limits)?;
        Ok((size, sink.retain == Some(true)))
    }
}

struct TarSink<'a, W> {
    output: &'a mut W,
    prefix: [u8; 512],
    used: usize,
    retain: Option<bool>,
}
impl<W: Write> Write for TarSink<'_, W> {
    fn write(&mut self, mut bytes: &[u8]) -> io::Result<usize> {
        let count = bytes.len();
        if self.retain.is_none() {
            let take = (512 - self.used).min(bytes.len());
            self.prefix[self.used..self.used + take].copy_from_slice(&bytes[..take]);
            self.used += take;
            bytes = &bytes[take..];
            if self.used == 512 {
                self.retain = Some(crate::probe(&self.prefix).ok() == Some(Format::Tar));
                if self.retain == Some(true) {
                    self.output.write_all(&self.prefix)?;
                }
            }
        }
        if self.retain == Some(true) {
            self.output.write_all(bytes)?;
        }
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}

pub(crate) fn entry(format: Format, size: u64, length: u64, name: Option<&[u8]>) -> Entry {
    let name = name.filter(|name| !name.is_empty()).unwrap_or(b"data");
    Entry {
        id: EntryId(0),
        name: String::from_utf8_lossy(name).into_owned(),
        raw_name: name.to_vec(),
        kind: EntryKind::File,
        size,
        compressed_size: Some(length),
        compression: match format {
            Format::Gzip => "DEFLATE".into(),
            Format::Xz => "LZMA2".into(),
            _ => format!("{format:?}"),
        },
        encrypted: false,
    }
}
