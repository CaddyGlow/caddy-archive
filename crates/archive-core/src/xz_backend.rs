//! XZ 1.2.1 framing over ms-compress LZMA2. CRC32/64, multiple blocks and streams.
use crate::{Error, Limits, Result};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, Write};
fn crc(bytes: &[u8]) -> u32 {
    ms_compress::zlib::crc32::crc32(0, bytes)
}
fn integer(reader: &mut impl Read) -> Result<u64> {
    let mut result = 0u64;
    for i in 0..9 {
        let mut b = [0];
        reader.read_exact(&mut b)?;
        if i > 0 && b[0] == 0 {
            return Err(Error::Malformed("noncanonical XZ integer".into()));
        }
        result |= u64::from(b[0] & 127) << (i * 7);
        if b[0] & 128 == 0 {
            return Ok(result);
        }
    }
    Err(Error::Malformed("XZ integer overflow".into()))
}
fn put_integer(bytes: &mut Vec<u8>, mut value: u64) {
    while value >= 128 {
        bytes.push(value as u8 | 128);
        value >>= 7;
    }
    bytes.push(value as u8);
}
fn check_crc(bytes: &[u8]) -> Result<()> {
    if bytes.len() < 4 {
        return Err(Error::Malformed("truncated XZ checksum".into()));
    }
    let end = bytes.len() - 4;
    if crc(&bytes[..end]).to_le_bytes() != bytes[end..] {
        return Err(Error::Integrity("XZ header/index CRC32".into()));
    }
    Ok(())
}
struct CheckWriter<'a, W> {
    writer: &'a mut W,
    check: Check,
}
enum Check {
    None,
    Crc32(u32),
    Crc64(crc64fast::Digest),
    Sha256(Sha256),
}
impl Check {
    fn new(kind: u8) -> Self {
        match kind {
            1 => Self::Crc32(0),
            4 => Self::Crc64(crc64fast::Digest::new()),
            10 => Self::Sha256(Sha256::new()),
            _ => Self::None,
        }
    }
    fn update(&mut self, data: &[u8]) {
        match self {
            Self::None => {}
            Self::Crc32(crc) => *crc = ms_compress::zlib::crc32::crc32(*crc, data),
            Self::Crc64(crc) => crc.write(data),
            Self::Sha256(sha) => sha.update(data),
        }
    }
    fn verify(self, checksum: &[u8]) -> bool {
        match self {
            Self::None => checksum.is_empty(),
            Self::Crc32(crc) => checksum == crc.to_le_bytes(),
            Self::Crc64(crc) => checksum == crc.sum64().to_le_bytes(),
            Self::Sha256(sha) => checksum == &sha.finalize()[..],
        }
    }
}
impl<W: Write> Write for CheckWriter<'_, W> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let n = self.writer.write(data)?;
        self.check.update(&data[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}
pub(crate) fn decode(
    reader: &mut (impl Read + Seek),
    writer: &mut impl Write,
    limits: Limits,
) -> Result<u64> {
    reader.rewind()?;
    let mut total = 0u64;
    let mut stream_count = 0u64;
    loop {
        let mut magic = [0u8; 6];
        let n = reader.read(&mut magic[..1])?;
        if n == 0 {
            if stream_count == 0 {
                return Err(Error::Malformed("empty XZ stream".into()));
            }
            return Ok(total);
        }
        if magic[0] == 0 {
            let mut padding = [0u8; 3];
            reader.read_exact(&mut padding)?;
            if padding != [0; 3] {
                return Err(Error::Malformed("XZ stream padding".into()));
            }
            continue;
        }
        reader.read_exact(&mut magic[1..])?;
        if magic != *b"\xfd7zXZ\0" {
            return Err(Error::Malformed("XZ stream magic".into()));
        }
        stream_count += 1;
        if stream_count > limits.max_entries {
            return Err(Error::ResourceLimit("XZ streams"));
        }
        let mut stream_header = [0u8; 6];
        reader.read_exact(&mut stream_header)?;
        check_crc(&stream_header)?;
        if stream_header[0] != 0 {
            return Err(Error::Unsupported("XZ stream flags".into()));
        }
        let check = stream_header[1];
        let check_size = match check {
            0 => 0,
            1 => 4,
            4 => 8,
            10 => 32,
            _ => return Err(Error::Unsupported("XZ integrity check".into())),
        };
        let mut records = Vec::new();
        loop {
            let mut first = [0u8; 1];
            reader.read_exact(&mut first)?;
            if first[0] == 0 {
                break;
            }
            let header_size = (usize::from(first[0]) + 1) * 4;
            let mut header = vec![0u8; header_size];
            header[0] = first[0];
            reader.read_exact(&mut header[1..])?;
            check_crc(&header)?;
            let flags = header[1];
            if flags & 0x3c != 0 {
                return Err(Error::Unsupported("XZ filter chain or block flags".into()));
            }
            let mut fields = &header[2..header_size - 4];
            let compressed = if flags & 64 != 0 {
                Some(integer(&mut fields)?)
            } else {
                None
            };
            let decoded = if flags & 128 != 0 {
                Some(integer(&mut fields)?)
            } else {
                None
            };
            let filter_count = usize::from(flags & 3) + 1;
            let mut filters = Vec::new();
            let mut prop = 0;
            for i in 0..filter_count {
                let id = integer(&mut fields)?;
                let length = usize::try_from(integer(&mut fields)?)
                    .map_err(|_| Error::ResourceLimit("XZ filter properties"))?;
                if length > fields.len() {
                    return Err(Error::Malformed("XZ filter property length".into()));
                }
                let properties = &fields[..length];
                fields = &fields[length..];
                if i == filter_count - 1 {
                    if id != 0x21 {
                        return Err(Error::Unsupported("XZ final filter must be LZMA2".into()));
                    }
                    if properties.len() != 1 {
                        return Err(Error::Malformed("XZ LZMA2 properties".into()));
                    }
                    prop = properties[0];
                } else {
                    if !(3..=11).contains(&id) {
                        return Err(Error::Unsupported(format!("XZ filter {id}")));
                    }
                    let property = if id == 3 {
                        if properties.len() != 1 {
                            return Err(Error::Malformed("XZ delta properties".into()));
                        }
                        u32::from(properties[0]) + 1
                    } else {
                        match properties.len() {
                            0 => 0,
                            4 => u32::from_le_bytes(
                                properties
                                    .try_into()
                                    .map_err(|_| Error::Malformed("XZ BCJ properties".into()))?,
                            ),
                            _ => return Err(Error::Malformed("XZ BCJ properties".into())),
                        }
                    };
                    let alignment = match id {
                        3 | 4 => 1,
                        5 | 7 | 9 | 10 => 4,
                        6 => 16,
                        8 | 11 => 2,
                        _ => 1,
                    };
                    if property % alignment != 0 {
                        return Err(Error::Malformed("XZ BCJ start offset alignment".into()));
                    }
                    filters.push((id, property));
                }
            }
            if prop > 40 {
                return Err(Error::Malformed("XZ dictionary property".into()));
            }
            if fields.iter().any(|b| *b != 0) {
                return Err(Error::Malformed("XZ block header padding".into()));
            }
            let dictionary = if prop == 40 {
                u32::MAX
            } else {
                (2u32 | u32::from(prop & 1)) << (prop / 2 + 11)
            };
            if u64::from(dictionary) > limits.max_dictionary_bytes {
                return Err(Error::ResourceLimit("dictionary bytes"));
            }
            if u64::from(ms_compress::lzma::lzma2_get_memory_usage(dictionary)) * 1024
                > limits.max_active_workspace_bytes
            {
                return Err(Error::ResourceLimit("active decoder workspace bytes"));
            }
            let start = reader.stream_position()?;
            let mut checked = CheckWriter {
                writer,
                check: Check::new(check),
            };
            let mut decoder: Box<dyn Read + '_> = Box::new(ms_compress::lzma::Lzma2Reader::new(
                &mut *reader,
                dictionary,
                None,
            ));
            for (id, property) in filters.into_iter().rev() {
                use ms_compress::lzma::filter::{bcj::BcjReader, delta::DeltaReader};
                let pos = property as usize;
                decoder = match id {
                    3 => Box::new(DeltaReader::new(decoder, pos)),
                    4 => Box::new(BcjReader::new_x86(decoder, pos)),
                    5 => Box::new(BcjReader::new_ppc(decoder, pos)),
                    6 => Box::new(BcjReader::new_ia64(decoder, pos)),
                    7 => Box::new(BcjReader::new_arm(decoder, pos)),
                    8 => Box::new(BcjReader::new_arm_thumb(decoder, pos)),
                    9 => Box::new(BcjReader::new_sparc(decoder, pos)),
                    10 => Box::new(BcjReader::new_arm64(decoder, pos)),
                    11 => Box::new(BcjReader::new_riscv(decoder, pos)),
                    _ => return Err(Error::Unsupported("XZ filter".into())),
                };
            }
            let bytes =
                crate::copy_bounded(&mut decoder, &mut checked, limits.max_total_bytes - total)?;
            drop(decoder);
            let compressed_actual = reader
                .stream_position()?
                .checked_sub(start)
                .ok_or(Error::Malformed("XZ compressed offset".into()))?;
            if compressed.is_some_and(|size| size != compressed_actual)
                || decoded.is_some_and(|size| size != bytes)
            {
                return Err(Error::Integrity("XZ declared block size".into()));
            }
            total = total
                .checked_add(bytes)
                .ok_or(Error::ResourceLimit("decoded bytes"))?;
            let padding = (4 - compressed_actual % 4) % 4;
            let mut pad = [0u8; 3];
            reader.read_exact(&mut pad[..padding as usize])?;
            if pad[..padding as usize].iter().any(|b| *b != 0) {
                return Err(Error::Malformed("XZ block padding".into()));
            }
            let mut checksum = [0u8; 32];
            reader.read_exact(&mut checksum[..check_size])?;
            let valid = checked.check.verify(&checksum[..check_size]);
            if !valid {
                return Err(Error::Integrity("XZ block checksum".into()));
            }
            records.push((
                header_size as u64 + compressed_actual + check_size as u64,
                bytes,
            ));
            if records.len() as u64 > limits.max_entries {
                return Err(Error::ResourceLimit("XZ blocks"));
            }
        }
        let index_start = reader
            .stream_position()?
            .checked_sub(1)
            .ok_or(Error::Malformed("XZ index position".into()))?;
        let count = integer(reader)?;
        if count != records.len() as u64 {
            return Err(Error::Integrity("XZ index block count".into()));
        }
        for (unpadded, decoded) in records {
            if integer(reader)? != unpadded || integer(reader)? != decoded {
                return Err(Error::Integrity("XZ index record".into()));
            }
        }
        let bytes = reader
            .stream_position()?
            .checked_sub(index_start)
            .ok_or(Error::Malformed("XZ index offset".into()))?;
        let padding = (4 - bytes % 4) % 4;
        let mut pad = [0u8; 3];
        reader.read_exact(&mut pad[..padding as usize])?;
        if pad[..padding as usize].iter().any(|b| *b != 0) {
            return Err(Error::Malformed("XZ index padding".into()));
        }
        let index_end = reader.stream_position()?;
        let index_size = index_end - index_start + 4;
        if index_size > limits.max_metadata_bytes {
            return Err(Error::ResourceLimit("XZ index bytes"));
        }
        let mut index = vec![
            0u8;
            usize::try_from(index_size)
                .map_err(|_| Error::ResourceLimit("XZ index bytes"))?
        ];
        reader.seek(std::io::SeekFrom::Start(index_start))?;
        reader.read_exact(&mut index)?;
        check_crc(&index)?;
        let mut footer = [0u8; 12];
        reader.read_exact(&mut footer)?;
        if footer[10..] != *b"YZ"
            || footer[8..10] != stream_header[..2]
            || crc(&footer[4..10]).to_le_bytes() != footer[..4]
        {
            return Err(Error::Integrity("XZ stream footer".into()));
        }
        let backward = u32::from_le_bytes(
            footer[4..8]
                .try_into()
                .map_err(|_| Error::Malformed("XZ footer".into()))?,
        );
        if (u64::from(backward) + 1) * 4 != index_size {
            return Err(Error::Integrity("XZ backward index size".into()));
        }
    }
}
pub(crate) fn encode(data: &[u8], writer: &mut impl Write, limits: Limits) -> Result<()> {
    let mut stream = XzWriter::new(writer, limits, Some(data.len()))?;
    stream.write_all(data)?;
    stream.finish()
}
pub(crate) struct XzWriter<'a, W: Write> {
    encoder: ms_compress::lzma::Lzma2Writer<CountingWriter<'a, W>>,
    digest: crc64fast::Digest,
    decoded: u64,
    header_size: u64,
    limits: Limits,
}
impl<'a, W: Write> XzWriter<'a, W> {
    pub(crate) fn new(
        writer: &'a mut W,
        limits: Limits,
        input_size: Option<usize>,
    ) -> Result<Self> {
        let mut options = ms_compress::lzma::LzmaOptions::with_preset(6);
        options.dict_size = options.dict_size.min(
            input_size
                .unwrap_or(options.dict_size as usize)
                .max(4096)
                .min(u32::MAX as usize) as u32,
        );
        let dictionary = options.dict_size;
        if u64::from(dictionary) > limits.max_dictionary_bytes {
            return Err(Error::ResourceLimit("dictionary bytes"));
        }
        let prop = (0u8..40)
            .find(|p| (2u64 | u64::from(p & 1)) << (p / 2 + 11) >= u64::from(dictionary))
            .ok_or(Error::ResourceLimit("dictionary bytes"))?;
        let declared_dictionary = (2u64 | u64::from(prop & 1)) << (prop / 2 + 11);
        if declared_dictionary > limits.max_dictionary_bytes {
            return Err(Error::ResourceLimit("dictionary bytes"));
        }
        if u64::from(options.get_memory_usage()) * 1024 > limits.max_active_workspace_bytes {
            return Err(Error::ResourceLimit("active encoder workspace bytes"));
        }
        writer.write_all(b"\xfd7zXZ\0")?;
        writer.write_all(&[0, 4])?;
        writer.write_all(&crc(&[0, 4]).to_le_bytes())?;
        let mut block = vec![0, 0];
        put_integer(&mut block, 0x21);
        put_integer(&mut block, 1);
        block.push(prop);
        while (block.len() + 4) % 4 != 0 {
            block.push(0);
        }
        block[0] = ((block.len() + 4) / 4 - 1) as u8;
        let block_crc = crc(&block);
        block.extend_from_slice(&block_crc.to_le_bytes());
        writer.write_all(&block)?;
        let counted = CountingWriter {
            writer: &mut *writer,
            count: 0,
        };
        let encoder = ms_compress::lzma::Lzma2Writer::new(
            counted,
            ms_compress::lzma::Lzma2Options {
                lzma_options: options,
                chunk_size: None,
            },
        );
        Ok(Self {
            encoder,
            digest: crc64fast::Digest::new(),
            decoded: 0,
            header_size: block.len() as u64,
            limits,
        })
    }
    pub(crate) fn finish(self) -> Result<()> {
        let counted = self.encoder.finish()?;
        let compressed_size = counted.count;
        let writer = counted.writer;
        let padding = (4 - compressed_size % 4) % 4;
        writer.write_all(&[0u8; 3][..padding as usize])?;
        writer.write_all(&self.digest.sum64().to_le_bytes())?;
        let mut index = vec![0, 1];
        put_integer(&mut index, self.header_size + compressed_size + 8);
        put_integer(&mut index, self.decoded);
        while index.len() % 4 != 0 {
            index.push(0);
        }
        let index_crc = crc(&index);
        index.extend_from_slice(&index_crc.to_le_bytes());
        writer.write_all(&index)?;
        let mut footer = Vec::new();
        footer.extend_from_slice(
            &u32::try_from(index.len() / 4 - 1)
                .map_err(|_| Error::ResourceLimit("XZ index bytes"))?
                .to_le_bytes(),
        );
        footer.extend_from_slice(&[0, 4]);
        writer.write_all(&crc(&footer).to_le_bytes())?;
        writer.write_all(&footer)?;
        writer.write_all(b"YZ")?;
        Ok(())
    }
}
impl<W: Write> Write for XzWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self
            .decoded
            .checked_add(bytes.len() as u64)
            .is_none_or(|size| size > self.limits.max_total_bytes)
        {
            return Err(std::io::Error::other(Error::ResourceLimit("decoded bytes")));
        }
        let n = self.encoder.write(bytes)?;
        self.decoded += n as u64;
        self.digest.write(&bytes[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.encoder.flush()
    }
}
struct CountingWriter<'a, W> {
    writer: &'a mut W,
    count: u64,
}
impl<W: Write> Write for CountingWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let n = self.writer.write(bytes)?;
        self.count = self
            .count
            .checked_add(n as u64)
            .ok_or_else(|| std::io::Error::other("compressed size overflow"))?;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}
