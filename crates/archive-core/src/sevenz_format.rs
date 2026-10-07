//! Direct 7z container parsing and serialization over workspace codecs.
//!
//! Field layouts follow the published 7-Zip `7zFormat.txt` and upstream
//! `7zIn.cpp`/`7zOut.cpp` references. This module is independently authored;
//! it does not contain the sevenz-rust2 container implementation.
use crate::{CreateEntry, CreateOptions, EntryKind, Error, Limits, Result};
use ms_compress::{
    lzma::{self, Lzma2Options, Lzma2Reader, Lzma2Writer, LzmaReader},
    zlib::crc32::crc32,
};
use std::{
    cell::RefCell,
    io::{self, Read, Seek, SeekFrom, Write},
    rc::Rc,
};

const SIGNATURE: [u8; 6] = [0x37, 0x7a, 0xbc, 0xaf, 0x27, 0x1c];
pub(crate) const COPY: &[u8] = &[0];
pub(crate) const DEFLATE: &[u8] = &[4, 1, 8];
pub(crate) const LZMA: &[u8] = &[3, 1, 1];
pub(crate) const LZMA2: &[u8] = &[0x21];
pub(crate) const AES: &[u8] = &[6, 0xf1, 7, 1];
pub(crate) const BZIP2: &[u8] = &[4, 2, 2];
pub(crate) const BROTLI: &[u8] = &[4, 0xf7, 0x11, 2];
pub(crate) const BROTLI_WORKSPACE: u64 = 64 * 1024 * 1024;
const BCJ2: &[u8] = &[3, 3, 1, 0x1b];

pub(crate) fn method_name(method: &[u8]) -> Result<&'static str> {
    Ok(match method {
        COPY => "Copy",
        DEFLATE => "Deflate",
        LZMA => "LZMA",
        LZMA2 => "LZMA2",
        AES => "AES256SHA256",
        #[cfg(feature = "bzip2")]
        BZIP2 => "BZip2",
        #[cfg(feature = "brotli")]
        BROTLI => "Brotli",
        [3] => "Delta",
        [3, 3, 1, 3] => "BCJ",
        [3, 3, 5, 1] => "ARM",
        [3, 3, 7, 1] => "ARMThumb",
        [3, 3, 2, 5] => "PPC",
        [3, 3, 4, 1] => "IA64",
        [3, 3, 8, 5] => "SPARC",
        [10] => "ARM64",
        [11] => "RISCV",
        BCJ2 => "BCJ2",
        _ => return Err(Error::Unsupported(format!("7z coder {method:?}"))),
    })
}

#[derive(Clone, Debug)]
pub(crate) struct Coder {
    pub method: Vec<u8>,
    pub props: Vec<u8>,
    pub inputs: usize,
}
impl Coder {
    pub fn encoder_method_id(&self) -> &[u8] {
        &self.method
    }
    pub fn properties(&self) -> &[u8] {
        &self.props
    }
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Folder {
    pub coders: Vec<Coder>,
    bindings: Vec<(usize, usize)>,
    packed_inputs: Vec<usize>,
    pub unpack_sizes: Vec<u64>,
    pub crc: Option<u32>,
    pub sizes: Vec<u64>,
    pub file_crcs: Vec<Option<u32>>,
    pub pack_indices: Vec<usize>,
    pub first_file: usize,
    pub end_file: usize,
}
impl Folder {
    pub fn get_unpack_size(&self) -> u64 {
        let bound: std::collections::BTreeSet<_> = self.bindings.iter().map(|(_, o)| *o).collect();
        self.unpack_sizes
            .iter()
            .enumerate()
            .find(|(i, _)| !bound.contains(i))
            .map_or(0, |(_, s)| *s)
    }
    pub fn get_unpack_size_at_index(&self, i: usize) -> u64 {
        self.unpack_sizes.get(i).copied().unwrap_or(0)
    }
}
#[derive(Clone, Debug, Default)]
pub(crate) struct FileRecord {
    pub name: String,
    pub size: u64,
    pub is_directory: bool,
    pub has_stream: bool,
    pub crc: Option<u32>,
    pub compressed_size: u64,
    pub metadata: crate::EntryMetadata,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct StreamMap {
    pub file_block_index: Vec<Option<usize>>,
    pub block_first_file_index: Vec<usize>,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct SevenArchive {
    pub files: Vec<FileRecord>,
    pub blocks: Vec<Folder>,
    pub stream_map: StreamMap,
    pub is_solid: bool,
    pack_offsets: Vec<u64>,
    pack_sizes: Vec<u64>,
    pack_crcs: Vec<Option<u32>>,
}

pub(crate) struct Password {
    pub bytes: Option<Vec<u8>>,
    pub limits: Limits,
}
impl Password {
    pub fn new(text: &str) -> Self {
        Self {
            bytes: Some(text.encode_utf16().flat_map(u16::to_le_bytes).collect()),
            limits: Limits::default(),
        }
    }
    pub fn empty() -> Self {
        Self {
            bytes: None,
            limits: Limits::default(),
        }
    }
}
impl Drop for Password {
    fn drop(&mut self) {
        #[cfg(feature = "crypto")]
        if let Some(bytes) = &mut self.bytes {
            zeroize::Zeroize::zeroize(bytes);
        }
    }
}

struct Header<'a> {
    bytes: &'a [u8],
    position: usize,
    limits: Limits,
}
impl<'a> Header<'a> {
    fn byte(&mut self) -> Result<u8> {
        let byte = *self
            .bytes
            .get(self.position)
            .ok_or_else(|| Error::Malformed("truncated 7z header".into()))?;
        self.position += 1;
        Ok(byte)
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(n)
            .ok_or(Error::ResourceLimit("7z metadata length"))?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| Error::Malformed("truncated 7z property".into()))?;
        self.position = end;
        Ok(bytes)
    }
    fn number(&mut self) -> Result<u64> {
        let first = self.byte()?;
        let mut value = 0u64;
        let mut mask = 0x80u8;
        for i in 0..8 {
            if first & mask == 0 {
                return Ok(value | u64::from(first & (mask - 1)) << (i * 8));
            }
            value |= u64::from(self.byte()?) << (i * 8);
            mask >>= 1;
        }
        Ok(value)
    }
    fn count(&mut self, max: u64) -> Result<usize> {
        let value = self.number()?;
        if value > max || value > self.bytes.len() as u64 {
            return Err(Error::ResourceLimit("7z header item count"));
        }
        usize::try_from(value).map_err(|_| Error::ResourceLimit("7z item count conversion"))
    }
    fn bits(&mut self, count: usize) -> Result<Vec<bool>> {
        let mut bits = Vec::with_capacity(count);
        let mut byte = 0;
        for i in 0..count {
            if i % 8 == 0 {
                byte = self.byte()?;
            }
            bits.push(byte & (0x80 >> (i % 8)) != 0);
        }
        Ok(bits)
    }
    fn digests(&mut self, count: usize) -> Result<Vec<Option<u32>>> {
        let defined = if self.byte()? != 0 {
            vec![true; count]
        } else {
            self.bits(count)?
        };
        defined
            .into_iter()
            .map(|set| {
                if set {
                    Ok(Some(u32::from_le_bytes(
                        self.take(4)?
                            .try_into()
                            .map_err(|_| Error::Malformed("7z CRC".into()))?,
                    )))
                } else {
                    Ok(None)
                }
            })
            .collect()
    }
}

fn malformed(message: &str) -> Error {
    Error::Malformed(message.into())
}
fn tag(h: &mut Header<'_>, expected: u8) -> Result<()> {
    if h.byte()? != expected {
        Err(malformed("unexpected 7z header property"))
    } else {
        Ok(())
    }
}

fn folder(h: &mut Header<'_>) -> Result<Folder> {
    let count = h.count(32)?;
    if count == 0 {
        return Err(malformed("empty 7z folder"));
    }
    let mut f = Folder::default();
    let (mut inputs, mut outputs) = (0usize, 0usize);
    for _ in 0..count {
        let flags = h.byte()?;
        let id = usize::from(flags & 15);
        if id == 0 || flags & 0xc0 != 0 {
            return Err(Error::Unsupported("7z alternative coder methods".into()));
        }
        let method = h.take(id)?.to_vec();
        let (num_in, num_out) = if flags & 0x10 != 0 {
            (h.count(32)?, h.count(32)?)
        } else {
            (1, 1)
        };
        if num_in == 0 || num_out != 1 {
            return Err(Error::Unsupported("7z multi-output coder".into()));
        }
        inputs = inputs
            .checked_add(num_in)
            .ok_or(Error::ResourceLimit("7z coder inputs"))?;
        outputs += num_out;
        if inputs > 32 || outputs > 32 {
            return Err(Error::ResourceLimit("7z coder graph"));
        }
        let props = if flags & 0x20 != 0 {
            let len = h.count(h.limits.max_metadata_bytes)?;
            h.take(len)?.to_vec()
        } else {
            Vec::new()
        };
        method_name(&method)?;
        if method == AES {
            validate_aes_properties(&props, h.limits)?;
        }
        f.coders.push(Coder {
            method,
            props,
            inputs: num_in,
        });
    }
    let bindings = outputs
        .checked_sub(1)
        .ok_or_else(|| malformed("7z folder outputs"))?;
    let (mut bound_in, mut bound_out) = (
        std::collections::BTreeSet::new(),
        std::collections::BTreeSet::new(),
    );
    for _ in 0..bindings {
        let i = h.count(32)?;
        let o = h.count(32)?;
        if i >= inputs || o >= outputs || !bound_in.insert(i) || !bound_out.insert(o) {
            return Err(malformed("invalid or duplicate 7z graph binding"));
        }
        f.bindings.push((i, o));
    }
    let packed = inputs
        .checked_sub(bindings)
        .ok_or_else(|| malformed("7z packed input count"))?;
    if packed == 0 {
        return Err(malformed("7z folder has no packed input"));
    }
    if packed == 1 {
        f.packed_inputs.push(
            (0..inputs)
                .find(|i| !bound_in.contains(i))
                .ok_or_else(|| malformed("7z missing packed input"))?,
        );
    } else {
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..packed {
            let p = h.count(32)?;
            if p >= inputs || bound_in.contains(&p) || !seen.insert(p) {
                return Err(malformed("7z invalid packed input"));
            }
            f.packed_inputs.push(p);
        }
    }
    // Each output belongs to one coder. Follow dependencies now, before decoder allocation.
    let mut states = vec![0u8; count];
    for coder in 0..count {
        graph_visit(&f, coder, &mut states, 0, h.limits.max_nesting_depth)?;
    }
    Ok(f)
}
fn graph_visit(
    f: &Folder,
    coder: usize,
    states: &mut [u8],
    depth: usize,
    max: usize,
) -> Result<()> {
    if depth > max {
        return Err(Error::ResourceLimit("7z graph depth"));
    }
    if states[coder] == 1 {
        return Err(malformed("cyclic 7z coder graph"));
    }
    if states[coder] == 2 {
        return Ok(());
    }
    states[coder] = 1;
    let first = f.coders[..coder].iter().map(|c| c.inputs).sum::<usize>();
    for input in first..first + f.coders[coder].inputs {
        if let Some((_, output)) = f.bindings.iter().find(|(i, _)| *i == input) {
            graph_visit(f, *output, states, depth + 1, max)?;
        }
    }
    states[coder] = 2;
    Ok(())
}

fn streams(h: &mut Header<'_>, base: u64) -> Result<SevenArchive> {
    let mut archive = SevenArchive::default();
    let mut current = h.byte()?;
    if current == 6 {
        let position = h.number()?;
        let count = h.count(h.limits.max_entries)?;
        tag(h, 9)?;
        archive.pack_sizes = (0..count).map(|_| h.number()).collect::<Result<_>>()?;
        current = h.byte()?;
        if current == 10 {
            archive.pack_crcs = h.digests(count)?;
            current = h.byte()?;
        } else {
            archive.pack_crcs = vec![None; count];
        }
        if current != 0 {
            return Err(malformed("7z pack info termination"));
        }
        let mut offset = base
            .checked_add(position)
            .ok_or(Error::ResourceLimit("7z packed offset"))?;
        for size in &archive.pack_sizes {
            archive.pack_offsets.push(offset);
            offset = offset
                .checked_add(*size)
                .ok_or(Error::ResourceLimit("7z packed extent"))?;
        }
        current = h.byte()?;
    }
    if current == 7 {
        tag(h, 11)?;
        let count = h.count(h.limits.max_entries)?;
        if h.byte()? != 0 {
            return Err(Error::Unsupported("external 7z folder metadata".into()));
        }
        for _ in 0..count {
            archive.blocks.push(folder(h)?);
        }
        tag(h, 12)?;
        let mut pack_index = 0;
        for f in &mut archive.blocks {
            for _ in 0..f.coders.len() {
                let size = h.number()?;
                if size > h.limits.max_total_bytes || usize::try_from(size).is_err() {
                    return Err(Error::ResourceLimit("7z coder decoded bytes"));
                }
                f.unpack_sizes.push(size);
            }
            f.pack_indices = (pack_index..pack_index + f.packed_inputs.len()).collect();
            pack_index += f.packed_inputs.len();
        }
        if pack_index != archive.pack_sizes.len() {
            return Err(malformed("7z packed stream/folder mapping"));
        }
        current = h.byte()?;
        if current == 10 {
            let crcs = h.digests(count)?;
            for (f, crc) in archive.blocks.iter_mut().zip(crcs) {
                f.crc = crc;
            }
            current = h.byte()?;
        }
        if current != 0 {
            return Err(malformed("7z unpack info termination"));
        }
        for f in &mut archive.blocks {
            f.sizes = vec![f.get_unpack_size()];
            f.file_crcs = vec![f.crc];
        }
        current = h.byte()?;
    }
    if current == 8 {
        let mut counts = vec![1usize; archive.blocks.len()];
        current = h.byte()?;
        if current == 13 {
            for count in &mut counts {
                *count = h.count(h.limits.max_entries)?;
                if *count == 0 {
                    return Err(malformed("7z zero folder substreams"));
                }
            }
            current = h.byte()?;
        }
        let substreams = counts.iter().try_fold(0u64, |total, count| {
            total
                .checked_add(*count as u64)
                .ok_or(Error::ResourceLimit("7z substreams"))
        })?;
        if substreams > h.limits.max_entries {
            return Err(Error::ResourceLimit("7z substreams"));
        }
        for (f, count) in archive.blocks.iter_mut().zip(&counts) {
            f.sizes.clear();
            let mut sum = 0u64;
            if *count > 1 && current != 9 {
                return Err(malformed("missing 7z substream sizes"));
            }
            for _ in 1..*count {
                let size = h.number()?;
                sum = sum
                    .checked_add(size)
                    .ok_or(Error::ResourceLimit("7z substream size"))?;
                f.sizes.push(size);
            }
            f.sizes.push(
                f.get_unpack_size()
                    .checked_sub(sum)
                    .ok_or_else(|| malformed("7z substreams exceed folder"))?,
            );
        }
        if current == 9 {
            current = h.byte()?;
        }
        let crc_count = archive
            .blocks
            .iter()
            .zip(&counts)
            .map(|(f, c)| if *c == 1 && f.crc.is_some() { 0 } else { *c })
            .sum();
        let crc_values = if current == 10 {
            let v = h.digests(crc_count)?;
            current = h.byte()?;
            v
        } else {
            vec![None; crc_count]
        };
        let mut values = crc_values.into_iter();
        for (f, count) in archive.blocks.iter_mut().zip(counts) {
            f.file_crcs = if count == 1 && f.crc.is_some() {
                vec![f.crc]
            } else {
                (0..count).map(|_| values.next().flatten()).collect()
            };
        }
        if current != 0 {
            return Err(malformed("7z substream info termination"));
        }
        current = h.byte()?;
    }
    if current != 0 {
        return Err(Error::Unsupported("7z streams metadata property".into()));
    }
    Ok(archive)
}

fn files(h: &mut Header<'_>, archive: &mut SevenArchive) -> Result<()> {
    let count = h.count(h.limits.max_entries)?;
    if (count as u64).saturating_mul(std::mem::size_of::<crate::EntryMetadata>() as u64)
        > h.limits.max_metadata_bytes
    {
        return Err(Error::ResourceLimit("7z file metadata bytes"));
    }
    let mut names = None;
    let mut empty = vec![false; count];
    let mut empty_files = Vec::new();
    let mut anti = Vec::new();
    let mut metadata = vec![crate::EntryMetadata::default(); count];
    loop {
        let property = h.byte()?;
        if property == 0 {
            break;
        }
        let length = h.count(h.limits.max_metadata_bytes)?;
        let bytes = h.take(length)?;
        let mut p = Header {
            bytes,
            position: 0,
            limits: h.limits,
        };
        match property {
            14 => empty = p.bits(count)?,
            15 => empty_files = p.bits(empty.iter().filter(|b| **b).count())?,
            16 => anti = p.bits(empty.iter().filter(|b| **b).count())?,
            17 => {
                if p.byte()? != 0 {
                    return Err(Error::Unsupported("external 7z filenames".into()));
                }
                let raw = p.take(p.bytes.len() - p.position)?;
                if raw.len() % 2 != 0 {
                    return Err(malformed("odd UTF-16 7z filename bytes"));
                }
                let words: Vec<_> = raw
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect();
                let mut parsed = Vec::new();
                let mut start = 0;
                for (i, w) in words.iter().enumerate() {
                    if *w == 0 {
                        parsed.push(
                            String::from_utf16(&words[start..i])
                                .map_err(|_| malformed("invalid UTF-16 7z filename"))?,
                        );
                        start = i + 1;
                    }
                }
                if start != words.len() || parsed.len() != count {
                    return Err(malformed("7z filename count or termination"));
                }
                if names.replace(parsed).is_some() {
                    return Err(malformed("duplicate 7z name property"));
                }
            }
            18..=21 => {
                let defined = if p.byte()? != 0 {
                    vec![true; count]
                } else {
                    p.bits(count)?
                };
                if p.byte()? != 0 {
                    return Err(Error::Unsupported("external 7z file metadata".into()));
                }
                for (index, defined) in defined.into_iter().enumerate() {
                    if !defined {
                        continue;
                    }
                    if property == 21 {
                        let attributes = u32::from_le_bytes(
                            p.take(4)?
                                .try_into()
                                .map_err(|_| malformed("7z attributes"))?,
                        );
                        metadata[index].unix_mode = if attributes & 0x8000 != 0 {
                            Some(attributes >> 16)
                        } else if attributes & 1 != 0 {
                            Some(0o444)
                        } else {
                            None
                        };
                    } else {
                        let ticks = u64::from_le_bytes(
                            p.take(8)?
                                .try_into()
                                .map_err(|_| malformed("7z timestamp"))?,
                        );
                        if property == 20 {
                            metadata[index].modified =
                                ticks.checked_sub(116_444_736_000_000_000).map(|ticks| {
                                    crate::StoredTimestamp::UnixSeconds(ticks / 10_000_000)
                                });
                        }
                    }
                }
                if p.position != p.bytes.len() {
                    return Err(malformed("7z metadata property length"));
                }
            }
            25 => {}
            _ => return Err(Error::Unsupported(format!("7z file property {property}"))),
        }
    }
    let names = names.ok_or_else(|| malformed("missing 7z names"))?;
    let mut empty_index = 0;
    for ((name, is_empty), metadata) in names.into_iter().zip(empty).zip(metadata) {
        let directory = is_empty && !empty_files.get(empty_index).copied().unwrap_or(false);
        let mut metadata = metadata;
        if directory && metadata.unix_mode == Some(0o444) {
            metadata.unix_mode = Some(0o555);
        }
        if is_empty {
            if anti.get(empty_index).copied().unwrap_or(false) {
                return Err(Error::Unsupported("7z anti-items".into()));
            }
            empty_index += 1;
        }
        archive.files.push(FileRecord {
            name,
            size: 0,
            is_directory: directory,
            has_stream: !is_empty,
            crc: None,
            compressed_size: 0,
            metadata,
        });
    }
    archive.stream_map.file_block_index = vec![None; count];
    let mut file_index = 0;
    for (folder_index, f) in archive.blocks.iter_mut().enumerate() {
        let mut first = None;
        for (size, crc) in f.sizes.iter().zip(&f.file_crcs) {
            while file_index < count && !archive.files[file_index].has_stream {
                file_index += 1;
            }
            let file = archive
                .files
                .get_mut(file_index)
                .ok_or_else(|| malformed("7z folder has too many file streams"))?;
            first.get_or_insert(file_index);
            file.size = *size;
            file.crc = *crc;
            if f.sizes.len() == 1 {
                file.compressed_size = f.pack_indices.iter().map(|i| archive.pack_sizes[*i]).sum();
            }
            archive.stream_map.file_block_index[file_index] = Some(folder_index);
            file_index += 1;
        }
        f.first_file = first.ok_or_else(|| malformed("empty folder file mapping"))?;
        f.end_file = file_index;
        archive.stream_map.block_first_file_index.push(f.first_file);
        archive.is_solid |= f.sizes.len() > 1;
    }
    if archive.files[file_index..].iter().any(|f| f.has_stream) {
        return Err(malformed("7z files have missing folder streams"));
    }
    Ok(())
}

impl SevenArchive {
    pub fn read<R: Read + Seek>(reader: &mut R, password: &Password) -> Result<Self> {
        let length = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;
        let mut start = [0u8; 32];
        reader.read_exact(&mut start)?;
        if start[..6] != SIGNATURE || start[6] != 0 {
            return Err(malformed("7z signature or version"));
        }
        if crc32(0, &start[12..])
            != u32::from_le_bytes(
                start[8..12]
                    .try_into()
                    .map_err(|_| malformed("start CRC"))?,
            )
        {
            return Err(Error::Integrity("7z start header CRC".into()));
        }
        let offset = u64::from_le_bytes(
            start[12..20]
                .try_into()
                .map_err(|_| malformed("next offset"))?,
        );
        let size = u64::from_le_bytes(
            start[20..28]
                .try_into()
                .map_err(|_| malformed("next size"))?,
        );
        let expected = u32::from_le_bytes(
            start[28..32]
                .try_into()
                .map_err(|_| malformed("next CRC"))?,
        );
        if size > password.limits.max_metadata_bytes {
            return Err(Error::ResourceLimit("7z next header bytes"));
        }
        let position = 32u64
            .checked_add(offset)
            .ok_or(Error::ResourceLimit("7z next offset"))?;
        if position.checked_add(size).is_none_or(|end| end > length) {
            return Err(malformed("7z next header extent"));
        }
        reader.seek(SeekFrom::Start(position))?;
        let mut bytes = vec![
            0;
            usize::try_from(size)
                .map_err(|_| Error::ResourceLimit("7z header conversion"))?
        ];
        reader.read_exact(&mut bytes)?;
        if crc32(0, &bytes) != expected {
            return Err(Error::Integrity("7z next header CRC".into()));
        }
        if size == 0 {
            if offset != 0 {
                return Err(malformed("empty 7z next header offset"));
            }
            return Ok(Self::default());
        }
        for _ in 0..password.limits.max_nesting_depth.min(16) {
            let mut h = Header {
                bytes: &bytes,
                position: 0,
                limits: password.limits,
            };
            match h.byte()? {
                23 => {
                    let encoded = streams(&mut h, 32)?;
                    if encoded.blocks.len() != 1 {
                        return Err(Error::Unsupported("multi-folder 7z encoded header".into()));
                    }
                    let f = &encoded.blocks[0];
                    if f.get_unpack_size() > password.limits.max_metadata_bytes {
                        return Err(Error::ResourceLimit("7z decoded header bytes"));
                    }
                    if h.position != h.bytes.len() {
                        return Err(malformed("7z encoded header termination"));
                    }
                    validate_extents(&encoded, position)?;
                    let mut decoded = Vec::new();
                    let mut stream = folder_reader(reader, &encoded, 0, password)?;
                    copy_exact(&mut stream, &mut decoded, f.get_unpack_size())?;
                    bytes = decoded;
                }
                1 => {
                    let mut archive = SevenArchive::default();
                    let mut property = h.byte()?;
                    if property == 2 {
                        loop {
                            let p = h.byte()?;
                            if p == 0 {
                                break;
                            }
                            let n = h.count(h.limits.max_metadata_bytes)?;
                            h.take(n)?;
                        }
                        property = h.byte()?;
                    }
                    if property == 3 {
                        return Err(Error::Unsupported("7z additional streams".into()));
                    }
                    if property == 4 {
                        archive = streams(&mut h, 32)?;
                        property = h.byte()?;
                    }
                    if property == 5 {
                        files(&mut h, &mut archive)?;
                        property = h.byte()?;
                    }
                    if property != 0 || h.position != h.bytes.len() {
                        return Err(malformed("7z header termination"));
                    }
                    validate_extents(&archive, position)?;
                    return Ok(archive);
                }
                _ => return Err(malformed("7z header kind")),
            }
        }
        Err(Error::ResourceLimit("7z encoded header nesting"))
    }
}
fn validate_extents(archive: &SevenArchive, length: u64) -> Result<()> {
    for (offset, size) in archive.pack_offsets.iter().zip(&archive.pack_sizes) {
        if offset.checked_add(*size).is_none_or(|end| end > length) {
            return Err(malformed("7z packed stream extent"));
        }
    }
    let mut extents: Vec<_> = archive
        .pack_offsets
        .iter()
        .zip(&archive.pack_sizes)
        .map(|(o, s)| (*o, *o + *s))
        .collect();
    extents.sort_unstable();
    if extents.windows(2).any(|w| w[0].1 > w[1].0) {
        return Err(malformed("overlapping 7z packed streams"));
    }
    Ok(())
}
fn copy_exact(reader: &mut impl Read, writer: &mut impl Write, size: u64) -> Result<()> {
    let mut remaining = size;
    let mut chunk = [0u8; 65536];
    while remaining > 0 {
        let n = reader.read(&mut chunk[..remaining.min(65536) as usize])?;
        if n == 0 {
            return Err(Error::Integrity("truncated 7z decoded stream".into()));
        }
        writer.write_all(&chunk[..n])?;
        remaining -= n as u64;
    }
    let mut tail = [0];
    if reader.read(&mut tail)? != 0 {
        return Err(Error::Integrity(
            "7z decoded stream exceeds declaration".into(),
        ));
    }
    Ok(())
}

struct Packed<'a, R> {
    source: Rc<RefCell<&'a mut R>>,
    position: u64,
    end: u64,
}
impl<R: Read + Seek> Read for Packed<'_, R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.position == self.end {
            return Ok(0);
        }
        let max = out.len().min(
            usize::try_from((self.end - self.position).min(out.len() as u64))
                .map_err(|_| io::Error::other("range size"))?,
        );
        let mut source = self.source.borrow_mut();
        source.seek(SeekFrom::Start(self.position))?;
        let n = source.read(&mut out[..max])?;
        self.position += n as u64;
        Ok(n)
    }
}
struct Verified<R> {
    inner: R,
    remaining: u64,
    crc: u32,
    expected: Option<u32>,
    checked: bool,
    check_end: bool,
}
impl<R: Read> Read for Verified<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            if !self.checked && self.check_end {
                let mut tail = [0];
                if self.inner.read(&mut tail)? != 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "7z decoded stream exceeds declaration",
                    ));
                }
            }
            if !self.checked && self.expected.is_some_and(|crc| crc != self.crc) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "7z decoded CRC mismatch",
                ));
            }
            self.checked = true;
            return Ok(0);
        }
        let max = out.len().min(self.remaining.min(out.len() as u64) as usize);
        let n = self.inner.read(&mut out[..max])?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "7z decoded stream truncated",
            ));
        }
        self.remaining -= n as u64;
        self.crc = crc32(self.crc, &out[..n]);
        if self.remaining == 0 && self.expected.is_some_and(|crc| crc != self.crc) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "7z decoded CRC mismatch",
            ));
        }
        Ok(n)
    }
}

fn folder_reader<'a, R: Read + Seek>(
    source: &'a mut R,
    archive: &SevenArchive,
    index: usize,
    password: &Password,
) -> Result<Box<dyn Read + 'a>> {
    let f = &archive.blocks[index];
    if super::folder_workspace(f)? > password.limits.max_active_workspace_bytes {
        return Err(Error::ResourceLimit("7z decoder workspace"));
    }
    let source = Rc::new(RefCell::new(source));
    for &i in &f.pack_indices {
        if let Some(expected) = archive.pack_crcs[i] {
            let mut raw = Packed {
                source: source.clone(),
                position: archive.pack_offsets[i],
                end: archive.pack_offsets[i] + archive.pack_sizes[i],
            };
            let (mut actual, mut bytes) = (0u32, 0u64);
            let mut buf = [0u8; 65536];
            loop {
                let n = raw.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                actual = crc32(actual, &buf[..n]);
                bytes += n as u64;
            }
            if bytes != archive.pack_sizes[i] || actual != expected {
                return Err(Error::Integrity("7z packed CRC mismatch".into()));
            }
        }
    }
    let root = (0..f.coders.len())
        .find(|o| !f.bindings.iter().any(|(_, b)| b == o))
        .ok_or_else(|| malformed("7z graph has no final output"))?;
    let reader = build_coder(&source, archive, f, root, password, 0)?;
    Ok(Box::new(Verified {
        inner: reader,
        remaining: f.get_unpack_size(),
        crc: 0,
        expected: f.crc,
        checked: false,
        check_end: f.coders[root].method != AES,
    }))
}
fn build_coder<'a, R: Read + Seek>(
    source: &Rc<RefCell<&'a mut R>>,
    archive: &SevenArchive,
    f: &Folder,
    index: usize,
    password: &Password,
    depth: usize,
) -> Result<Box<dyn Read + 'a>> {
    if depth > password.limits.max_nesting_depth {
        return Err(Error::ResourceLimit("7z graph decode depth"));
    }
    let coder = &f.coders[index];
    let first = f.coders[..index].iter().map(|c| c.inputs).sum::<usize>();
    let mut inputs = Vec::<Box<dyn Read + 'a>>::new();
    for input in first..first + coder.inputs {
        if let Some((_, output)) = f.bindings.iter().find(|(i, _)| *i == input) {
            inputs.push(build_coder(
                source,
                archive,
                f,
                *output,
                password,
                depth + 1,
            )?);
        } else {
            let packed = f
                .packed_inputs
                .iter()
                .position(|p| *p == input)
                .ok_or_else(|| malformed("7z coder input mapping"))?;
            let p = f.pack_indices[packed];
            inputs.push(Box::new(Packed {
                source: source.clone(),
                position: archive.pack_offsets[p],
                end: archive.pack_offsets[p] + archive.pack_sizes[p],
            }));
        }
    }
    if coder.method == BCJ2 {
        if inputs.len() != 4 {
            return Err(malformed("7z BCJ2 input count"));
        }
        return Ok(Box::new(lzma::filter::bcj2::Bcj2Reader::new(
            inputs,
            f.unpack_sizes[index],
        )));
    }
    if inputs.len() != 1 {
        return Err(Error::Unsupported("7z multi-input coder".into()));
    }
    let input = inputs.pop().ok_or_else(|| malformed("7z coder input"))?;
    let props = &coder.props;
    match coder.method.as_slice() {
        COPY => Ok(input),
        LZMA => {
            if props.len() != 5 {
                return Err(malformed("LZMA property length"));
            }
            let dict = u32::from_le_bytes(
                props[1..5]
                    .try_into()
                    .map_err(|_| malformed("LZMA dictionary"))?,
            );
            check_memory(
                lzma::lzma_get_memory_usage_by_props(dict, props[0])? as u64 * 1024,
                password.limits,
            )?;
            Ok(Box::new(LzmaReader::new_with_props(
                input,
                f.unpack_sizes[index],
                props[0],
                dict,
                None,
            )?))
        }
        DEFLATE => {
            if password.limits.max_dictionary_bytes < 32768 {
                return Err(Error::ResourceLimit("DEFLATE dictionary bytes"));
            }
            if !props.is_empty() {
                return Err(malformed("DEFLATE properties"));
            }
            check_memory(
                f.unpack_sizes[index]
                    .checked_add(1 << 20)
                    .ok_or(Error::ResourceLimit("DEFLATE workspace"))?,
                password.limits,
            )?;
            let mut encoded = input;
            let mut decoded = Vec::new();
            let capacity = usize::try_from(f.unpack_sizes[index])
                .map_err(|_| Error::ResourceLimit("DEFLATE unpacked size"))?;
            decoded
                .try_reserve_exact(capacity)
                .map_err(|_| Error::ResourceLimit("DEFLATE workspace allocation"))?;
            let bytes = crate::codec::inflate_window(
                &mut encoded,
                &mut decoded,
                0,
                f.unpack_sizes[index],
                1,
            )?;
            if bytes != f.unpack_sizes[index] {
                return Err(malformed("DEFLATE unpacked size"));
            }
            Ok(Box::new(io::Cursor::new(decoded)))
        }
        LZMA2 => {
            let dict = lzma2_dictionary(props)?;
            check_memory(
                lzma::lzma2_get_memory_usage(dict) as u64 * 1024,
                password.limits,
            )?;
            Ok(Box::new(Lzma2Reader::new(input, dict, None)))
        }
        AES => decrypt_reader(input, props, password),
        #[cfg(feature = "bzip2")]
        BZIP2 => {
            check_memory(16 * 1024 * 1024, password.limits)?;
            Ok(Box::new(bzip2::read::BzDecoder::new(input)))
        }
        #[cfg(feature = "brotli")]
        BROTLI => {
            check_memory(BROTLI_WORKSPACE, password.limits)?;
            Ok(Box::new(BrotliReader::new(input, password.limits)?))
        }
        [3] => {
            let distance =
                usize::from(*props.first().ok_or_else(|| malformed("delta properties"))?) + 1;
            if props.len() != 1 {
                return Err(malformed("delta property length"));
            }
            Ok(Box::new(lzma::filter::delta::DeltaReader::new(
                input, distance,
            )))
        }
        method => {
            let offset = if props.is_empty() {
                0
            } else if props.len() == 4 {
                u32::from_le_bytes(
                    props
                        .as_slice()
                        .try_into()
                        .map_err(|_| malformed("BCJ offset"))?,
                )
            } else {
                return Err(malformed("BCJ properties"));
            };
            let offset = usize::try_from(offset).map_err(|_| malformed("BCJ offset conversion"))?;
            let reader = match method {
                [3, 3, 1, 3] => lzma::filter::bcj::BcjReader::new_x86(input, offset),
                [3, 3, 5, 1] => lzma::filter::bcj::BcjReader::new_arm(input, offset),
                [3, 3, 7, 1] => lzma::filter::bcj::BcjReader::new_arm_thumb(input, offset),
                [3, 3, 2, 5] => lzma::filter::bcj::BcjReader::new_ppc(input, offset),
                [3, 3, 4, 1] => lzma::filter::bcj::BcjReader::new_ia64(input, offset),
                [3, 3, 8, 5] => lzma::filter::bcj::BcjReader::new_sparc(input, offset),
                [10] => lzma::filter::bcj::BcjReader::new_arm64(input, offset),
                [11] => lzma::filter::bcj::BcjReader::new_riscv(input, offset),
                _ => return Err(Error::Unsupported(format!("7z coder {method:?}"))),
            };
            Ok(Box::new(reader))
        }
    }
}
pub(crate) fn lzma2_dictionary(props: &[u8]) -> Result<u32> {
    if props.len() != 1 || props[0] > 40 {
        return Err(malformed("LZMA2 dictionary property"));
    }
    let p = u32::from(props[0]);
    Ok(if p == 40 {
        u32::MAX
    } else {
        (2 | (p & 1)) << (p / 2 + 11)
    })
}
fn check_memory(bytes: u64, limits: Limits) -> Result<()> {
    if bytes > limits.max_dictionary_bytes || bytes > limits.max_active_workspace_bytes {
        Err(Error::ResourceLimit("7z codec workspace"))
    } else {
        Ok(())
    }
}

#[cfg(feature = "brotli")]
use crate::brotli_budget::{BoundedBrotli, brotli_decoder};

#[cfg(feature = "brotli")]
struct BrotliReader<'a> {
    source: Option<Box<dyn Read + 'a>>,
    decoder: Option<BoundedBrotli<io::Take<Box<dyn Read + 'a>>>>,
    failed: Rc<std::cell::Cell<bool>>,
    framed: bool,
    frames: u64,
    limits: Limits,
}
#[cfg(feature = "brotli")]
impl<'a> BrotliReader<'a> {
    fn new(mut source: Box<dyn Read + 'a>, limits: Limits) -> Result<Self> {
        let mut prefix = [0u8; 4];
        source.read_exact(&mut prefix)?;
        let framed = u32::from_le_bytes(prefix) == 0x184d2a50;
        let source: Box<dyn Read + 'a> = Box::new(io::Cursor::new(prefix).chain(source));
        let mut reader = Self {
            source: Some(source),
            decoder: None,
            failed: Rc::new(std::cell::Cell::new(false)),
            framed,
            frames: 0,
            limits,
        };
        reader.next_frame()?;
        Ok(reader)
    }
    fn next_frame(&mut self) -> io::Result<bool> {
        let mut source = self
            .source
            .take()
            .ok_or_else(|| io::Error::other("missing Brotli source"))?;
        let size = if self.framed {
            let mut header = [0u8; 16];
            if source.read(&mut header[..1])? == 0 {
                return Ok(false);
            }
            source.read_exact(&mut header[1..])?;
            let word = |at| {
                u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
            };
            if word(0) != 0x184d2a50 || word(4) != 8 || header[12..14] != [0x42, 0x52] {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid zstdmt Brotli frame",
                ));
            }
            let size = u64::from(word(8));
            self.frames += 1;
            if size == 0
                || size > self.limits.max_input_bytes
                || self.frames > self.limits.max_entries
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Brotli frame limit",
                ));
            }
            size
        } else {
            self.limits.max_input_bytes
        };
        let (decoder, failed) = brotli_decoder(source.take(size));
        self.decoder = Some(decoder);
        self.failed = failed;
        Ok(true)
    }
}
#[cfg(feature = "brotli")]
impl Read for BrotliReader<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        use brotli::CustomRead;
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            let Some(decoder) = self.decoder.as_mut() else {
                return Ok(0);
            };
            let result = decoder.read(out);
            if self.failed.get() {
                return Err(io::Error::new(
                    io::ErrorKind::OutOfMemory,
                    "Brotli decoder workspace limit",
                ));
            }
            let n = result?;
            if n != 0 {
                return Ok(n);
            }
            let decoder = self
                .decoder
                .take()
                .ok_or_else(|| io::Error::other("missing Brotli decoder"))?;
            let input = decoder.into_inner().0;
            if !self.framed {
                return Ok(0);
            }
            if input.limit() != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Brotli frame compressed size mismatch",
                ));
            }
            self.source = Some(input.into_inner());
            if !self.next_frame()? {
                return Ok(0);
            }
        }
    }
}

pub(crate) fn for_folder<R: Read + Seek>(
    source: &mut R,
    archive: &SevenArchive,
    index: usize,
    password: &Password,
    each: &mut impl FnMut(&FileRecord, &mut dyn Read) -> Result<bool>,
) -> Result<bool> {
    let f = &archive.blocks[index];
    let mut reader = folder_reader(source, archive, index, password)?;
    for file in &archive.files[f.first_file..f.end_file] {
        if file.has_stream {
            let mut member = Verified {
                inner: &mut reader,
                remaining: file.size,
                crc: 0,
                expected: file.crc,
                checked: false,
                check_end: false,
            };
            if !each(file, &mut member)? {
                return Ok(false);
            }
            let mut tail = [0];
            if member.read(&mut tail)? != 0 {
                return Err(malformed("7z callback did not consume file"));
            }
        } else if !each(file, &mut io::empty())? {
            return Ok(false);
        }
    }
    let mut tail = [0];
    if reader.read(&mut tail)? != 0 {
        return Err(malformed("7z folder exceeds file mapping"));
    }
    Ok(true)
}

fn validate_aes_properties(props: &[u8], limits: Limits) -> Result<()> {
    let first = *props.first().ok_or_else(|| malformed("AES properties"))?;
    let second = props.get(1).copied().unwrap_or(0);
    let salt = usize::from((first >> 7) + (second >> 4));
    let iv = usize::from(((first >> 6) & 1) + (second & 15));
    let start = if props.len() == 1 { 1 } else { 2 };
    if start + salt + iv != props.len() || salt > 16 || iv > 16 {
        return Err(malformed("AES salt/IV properties"));
    }
    let power = first & 63;
    if power != 63
        && 1u64
            .checked_shl(u32::from(power))
            .is_none_or(|n| n > limits.max_password_iterations)
    {
        return Err(Error::ResourceLimit("7z password work"));
    }
    Ok(())
}

#[cfg(feature = "crypto")]
fn aes_key(props: &[u8], password: &Password) -> Result<([u8; 32], [u8; 16])> {
    validate_aes_properties(props, password.limits)?;
    use sha2::Digest;
    let first = *props.first().ok_or_else(|| malformed("AES properties"))?;
    let second = props.get(1).copied().unwrap_or(0);
    let salt_len = usize::from((first >> 7) + (second >> 4));
    let iv_len = usize::from(((first >> 6) & 1) + (second & 15));
    let start = if props.len() == 1 { 1 } else { 2 };
    let end = start + salt_len + iv_len;
    if end != props.len() || salt_len > 16 || iv_len > 16 {
        return Err(malformed("AES salt/IV properties"));
    }
    let salt = &props[start..start + salt_len];
    let mut iv = [0; 16];
    iv[..iv_len].copy_from_slice(&props[start + salt_len..end]);
    let bytes = password.bytes.as_deref().ok_or(Error::PasswordRequired)?;
    let power = first & 63;
    let mut key = [0; 32];
    if power == 63 {
        let n = salt.len().min(32);
        key[..n].copy_from_slice(&salt[..n]);
        let n = bytes.len().min(32 - salt.len());
        key[salt.len()..salt.len() + n].copy_from_slice(&bytes[..n]);
    } else {
        let rounds = 1u64
            .checked_shl(u32::from(power))
            .ok_or(Error::ResourceLimit("7z password work"))?;
        if rounds > password.limits.max_password_iterations {
            return Err(Error::ResourceLimit("7z password work"));
        }
        let mut sha = sha2::Sha256::new();
        for i in 0..rounds {
            sha.update(salt);
            sha.update(bytes);
            sha.update(i.to_le_bytes());
        }
        key.copy_from_slice(&sha.finalize());
    }
    Ok((key, iv))
}
#[cfg(feature = "crypto")]
struct AesReader<R: Read> {
    input: io::BufReader<R>,
    cipher: cbc::Decryptor<aes::Aes256>,
    block: [u8; 16],
    position: usize,
    done: bool,
}
#[cfg(feature = "crypto")]
impl<R: Read> Read for AesReader<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        use aes::cipher::BlockDecryptMut;
        if out.is_empty() {
            return Ok(0);
        }
        let mut written = 0;
        while written < out.len() {
            if self.position == 16 {
                let n = self.input.read(&mut self.block[..1])?;
                if n == 0 {
                    self.done = true;
                    break;
                }
                self.input.read_exact(&mut self.block[1..])?;
                let mut block = aes::cipher::Block::<aes::Aes256>::default();
                block.copy_from_slice(&self.block);
                self.cipher.decrypt_block_mut(&mut block);
                self.block.copy_from_slice(&block);
                self.position = 0;
            }
            let n = (16 - self.position).min(out.len() - written);
            out[written..written + n]
                .copy_from_slice(&self.block[self.position..self.position + n]);
            written += n;
            self.position += n;
        }
        Ok(written)
    }
}
#[cfg(feature = "crypto")]
fn decrypt_reader<'a>(
    input: Box<dyn Read + 'a>,
    props: &[u8],
    password: &Password,
) -> Result<Box<dyn Read + 'a>> {
    use aes::cipher::KeyIvInit;
    let (mut key, iv) = aes_key(props, password)?;
    let cipher = cbc::Decryptor::<aes::Aes256>::new((&key).into(), (&iv).into());
    zeroize::Zeroize::zeroize(&mut key);
    Ok(Box::new(AesReader {
        input: io::BufReader::new(input),
        cipher,
        block: [0; 16],
        position: 16,
        done: false,
    }))
}
#[cfg(not(feature = "crypto"))]
fn decrypt_reader<'a>(_: Box<dyn Read + 'a>, _: &[u8], _: &Password) -> Result<Box<dyn Read + 'a>> {
    Err(Error::PasswordRequired)
}

fn number(out: &mut Vec<u8>, value: u64) {
    for n in 0..8 {
        if value < 1u64 << (7 * (n + 1)) {
            let first = if n == 0 { 0 } else { (!0u8) << (8 - n) };
            out.push(first | ((value >> (n * 8)) as u8));
            for i in 0..n {
                out.push((value >> (i * 8)) as u8);
            }
            return;
        }
    }
    out.push(255);
    out.extend_from_slice(&value.to_le_bytes());
}
fn bits(out: &mut Vec<u8>, values: &[bool]) {
    for group in values.chunks(8) {
        let mut byte = 0;
        for (i, value) in group.iter().enumerate() {
            if *value {
                byte |= 0x80 >> i;
            }
        }
        out.push(byte);
    }
}
fn property(out: &mut Vec<u8>, id: u8, bytes: &[u8]) {
    out.push(id);
    number(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}
struct WrittenFolder {
    coders: Vec<Coder>,
    sizes: Vec<u64>,
    crc: u32,
    pack: u64,
    pack_crc: u32,
}
fn write_streams(out: &mut Vec<u8>, folders: &[WrittenFolder], position: u64) {
    out.push(6);
    number(out, position);
    number(out, folders.len() as u64);
    out.push(9);
    for f in folders {
        number(out, f.pack);
    }
    out.extend_from_slice(&[10, 1]);
    for f in folders {
        out.extend_from_slice(&f.pack_crc.to_le_bytes());
    }
    out.push(0);
    out.extend_from_slice(&[7, 11]);
    number(out, folders.len() as u64);
    out.push(0);
    for f in folders {
        number(out, f.coders.len() as u64);
        for c in &f.coders {
            out.push(c.method.len() as u8 | if c.props.is_empty() { 0 } else { 0x20 });
            out.extend_from_slice(&c.method);
            if !c.props.is_empty() {
                number(out, c.props.len() as u64);
                out.extend_from_slice(&c.props);
            }
        }
        for i in 1..f.coders.len() {
            number(out, i as u64);
            number(out, (i - 1) as u64);
        }
    }
    out.push(12);
    for f in folders {
        for size in &f.sizes {
            number(out, *size);
        }
    }
    out.extend_from_slice(&[10, 1]);
    for f in folders {
        out.extend_from_slice(&f.crc.to_le_bytes());
    }
    out.extend_from_slice(&[0, 0]);
}

#[cfg(feature = "crypto")]
fn encrypt(data: &mut Vec<u8>, options: &mut CreateOptions<'_>) -> Result<Coder> {
    use aes::cipher::{BlockEncryptMut, KeyIvInit};
    let bytes = options.password.ok_or(Error::PasswordRequired)?;
    let text = std::str::from_utf8(bytes)
        .map_err(|_| Error::Unsupported("7z password must be UTF-8 text".into()))?;
    let random = options
        .randomness
        .as_mut()
        .ok_or_else(|| Error::Unsupported("7z encryption requires random provider".into()))?;
    let mut props = vec![19 | 0xc0, 255];
    let mut random_bytes = [0u8; 32];
    random.fill(&mut random_bytes)?;
    props.extend_from_slice(&random_bytes);
    let password = Password::new(text);
    let (mut key, iv) = aes_key(&props, &password)?;
    let mut cipher = cbc::Encryptor::<aes::Aes256>::new((&key).into(), (&iv).into());
    zeroize::Zeroize::zeroize(&mut key);
    let padding = (16 - data.len() % 16) % 16;
    data.resize(data.len() + padding, 0);
    for chunk in data.chunks_exact_mut(16) {
        let mut block = aes::cipher::Block::<aes::Aes256>::default();
        block.copy_from_slice(chunk);
        cipher.encrypt_block_mut(&mut block);
        chunk.copy_from_slice(&block);
    }
    Ok(Coder {
        method: AES.to_vec(),
        props,
        inputs: 1,
    })
}
#[cfg(not(feature = "crypto"))]
fn encrypt(_: &mut Vec<u8>, _: &mut CreateOptions<'_>) -> Result<Coder> {
    Err(Error::Unsupported("7z crypto feature unavailable".into()))
}

fn encode_payload(data: &[u8], method: crate::SevenZipCompression) -> Result<(Vec<u8>, Coder)> {
    use crate::SevenZipCompression;
    let (encoded, method, props) = match method {
        SevenZipCompression::Copy => (data.to_vec(), COPY, Vec::new()),
        SevenZipCompression::Deflate => {
            let mut encoded = Vec::new();
            crate::codec::deflate(data, &mut encoded, false)?;
            (encoded, DEFLATE, Vec::new())
        }
        SevenZipCompression::Lzma => {
            let settings = lzma::LzmaOptions::with_preset(6);
            let mut encoder = lzma::LzmaWriter::new_no_header(Vec::new(), &settings, true)?;
            let mut props = vec![encoder.props()];
            props.extend_from_slice(&settings.dict_size.to_le_bytes());
            encoder.write_all(data)?;
            (encoder.finish()?, LZMA, props)
        }
        SevenZipCompression::Lzma2 => {
            let settings = Lzma2Options::with_preset(6);
            let dict = settings.lzma_options.dict_size;
            let prop = (0..=40)
                .find(|p| lzma2_dictionary(&[*p]).is_ok_and(|d| d >= dict))
                .ok_or_else(|| malformed("writer dictionary"))?;
            let mut encoder = Lzma2Writer::new(Vec::new(), settings);
            encoder.write_all(data)?;
            (encoder.finish()?, LZMA2, vec![prop])
        }
        SevenZipCompression::Bzip2 => {
            #[cfg(feature = "bzip2")]
            {
                let mut encoder =
                    bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::new(9));
                encoder.write_all(data)?;
                (encoder.finish()?, BZIP2, Vec::new())
            }
            #[cfg(not(feature = "bzip2"))]
            return Err(Error::Unsupported("BZip2 feature unavailable".into()));
        }
        SevenZipCompression::Brotli => {
            #[cfg(feature = "brotli")]
            {
                let mut encoded = Vec::new();
                {
                    let mut encoder = brotli::CompressorWriter::new(&mut encoded, 8192, 5, 22);
                    encoder.write_all(data)?;
                }
                (encoded, BROTLI, Vec::new())
            }
            #[cfg(not(feature = "brotli"))]
            return Err(Error::Unsupported("Brotli feature unavailable".into()));
        }
    };
    Ok((
        encoded,
        Coder {
            method: method.to_vec(),
            props,
            inputs: 1,
        },
    ))
}

pub(crate) fn write<W: Write + Seek>(
    entries: &[CreateEntry],
    mut output: W,
    options: &mut CreateOptions<'_>,
) -> Result<()> {
    if options.encrypt_headers && options.password.is_none() {
        return Err(Error::Unsupported(
            "encrypted 7z headers require password".into(),
        ));
    }
    output.write_all(&[0; 32])?;
    let mut folders = Vec::new();
    let mut packed = 0u64;
    for entry in entries {
        if !matches!(entry.kind, EntryKind::File | EntryKind::Directory) {
            return Err(Error::Unsupported("7z links or special files".into()));
        }
        if entry.kind == EntryKind::Directory || entry.data.is_empty() {
            continue;
        }
        if entry.kind != EntryKind::File {
            return Err(Error::Unsupported("7z links or special files".into()));
        }
        let (mut encoded, coder) = encode_payload(&entry.data, options.sevenz_compression)?;
        let compressed = encoded.len() as u64;
        let mut coders = vec![coder];
        let mut sizes = vec![entry.data.len() as u64];
        if options.password.is_some() {
            coders.insert(0, encrypt(&mut encoded, options)?);
            sizes.insert(0, compressed);
        }
        let crc = crc32(0, &entry.data);
        let pack_crc = crc32(0, &encoded);
        output.write_all(&encoded)?;
        folders.push(WrittenFolder {
            coders,
            sizes,
            crc,
            pack: encoded.len() as u64,
            pack_crc,
        });
        packed = packed
            .checked_add(encoded.len() as u64)
            .ok_or(Error::ResourceLimit("7z writer packed bytes"))?;
    }
    let mut header = vec![1];
    if !folders.is_empty() {
        header.push(4);
        write_streams(&mut header, &folders, 0);
    }
    header.push(5);
    number(&mut header, entries.len() as u64);
    let empty: Vec<_> = entries
        .iter()
        .map(|e| e.kind == EntryKind::Directory || e.data.is_empty())
        .collect();
    if empty.iter().any(|e| *e) {
        let mut flags = Vec::new();
        bits(&mut flags, &empty);
        property(&mut header, 14, &flags);
        let empty_files: Vec<_> = entries
            .iter()
            .zip(&empty)
            .filter(|(_, empty)| **empty)
            .map(|(e, _)| e.kind == EntryKind::File)
            .collect();
        flags.clear();
        bits(&mut flags, &empty_files);
        property(&mut header, 15, &flags);
    }
    let mut names = vec![0];
    for e in entries {
        if e.name.contains('\0') {
            return Err(malformed("7z filename NUL"));
        }
        for word in e.name.encode_utf16() {
            names.extend_from_slice(&word.to_le_bytes());
        }
        names.extend_from_slice(&[0, 0]);
    }
    property(&mut header, 17, &names);
    if let Some(metadata) = options.entry_metadata {
        if metadata.len() != entries.len() {
            return Err(malformed("creation metadata count mismatch"));
        }
        let times: Vec<_> = metadata
            .iter()
            .map(|value| matches!(value.modified, Some(crate::StoredTimestamp::UnixSeconds(_))))
            .collect();
        let mut values = vec![0];
        bits(&mut values, &times);
        values.push(0);
        for value in metadata {
            if let Some(crate::StoredTimestamp::UnixSeconds(seconds)) = value.modified {
                let ticks = seconds
                    .checked_mul(10_000_000)
                    .and_then(|seconds| seconds.checked_add(116_444_736_000_000_000))
                    .ok_or(Error::ResourceLimit("7z timestamp"))?;
                values.extend_from_slice(&ticks.to_le_bytes());
            }
        }
        property(&mut header, 20, &values);
        let modes: Vec<_> = metadata
            .iter()
            .map(|value| value.unix_mode.is_some())
            .collect();
        let mut values = vec![0];
        bits(&mut values, &modes);
        values.push(0);
        for (entry, metadata) in entries.iter().zip(metadata) {
            if let Some(mode) = metadata.unix_mode {
                let attributes = (mode << 16)
                    | 0x8000
                    | if entry.kind == EntryKind::Directory {
                        0x10
                    } else {
                        0x20
                    }
                    | if mode & 0o222 == 0 { 1 } else { 0 };
                values.extend_from_slice(&attributes.to_le_bytes());
            }
        }
        property(&mut header, 21, &values);
    }
    header.extend_from_slice(&[0, 0]);
    if options.encrypt_headers {
        let unpack = header.len() as u64;
        let crc = crc32(0, &header);
        let coder = encrypt(&mut header, options)?;
        let info = WrittenFolder {
            coders: vec![coder],
            sizes: vec![unpack],
            crc,
            pack: header.len() as u64,
            pack_crc: crc32(0, &header),
        };
        output.write_all(&header)?;
        let mut wrapper = vec![23];
        write_streams(&mut wrapper, &[info], packed);
        header = wrapper;
    }
    let position = output.stream_position()?;
    output.write_all(&header)?;
    let mut start = Vec::new();
    start.extend_from_slice(&(position - 32).to_le_bytes());
    start.extend_from_slice(&(header.len() as u64).to_le_bytes());
    start.extend_from_slice(&crc32(0, &header).to_le_bytes());
    output.seek(SeekFrom::Start(0))?;
    output.write_all(&SIGNATURE)?;
    output.write_all(&[0, 4])?;
    output.write_all(&crc32(0, &start).to_le_bytes())?;
    output.write_all(&start)?;
    Ok(())
}

#[cfg(all(test, feature = "brotli"))]
mod brotli_tests {
    use super::*;
    fn frame(data: &[u8]) -> Vec<u8> {
        let (encoded, _) = encode_payload(data, crate::SevenZipCompression::Brotli).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x184d2a50u32.to_le_bytes());
        bytes.extend_from_slice(&8u32.to_le_bytes());
        bytes.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&[0x42, 0x52, 1, 0]);
        bytes.extend_from_slice(&encoded);
        bytes
    }
    #[test]
    fn independently_framed_brotli_streams_decode_without_frame_buffering() {
        let mut bytes = frame(b"first portable frame");
        bytes.extend_from_slice(&frame(b"second portable frame"));
        let mut reader =
            BrotliReader::new(Box::new(io::Cursor::new(bytes)), Limits::default()).unwrap();
        let mut actual = Vec::new();
        reader.read_to_end(&mut actual).unwrap();
        assert_eq!(actual, b"first portable framesecond portable frame");
    }
    #[test]
    fn framed_brotli_rejects_bad_headers_truncation_and_frame_count_limits() {
        let valid = frame(b"bounded portable frame payload");
        for index in [4, 12] {
            let mut corrupt = valid.clone();
            corrupt[index] ^= 1;
            assert!(
                BrotliReader::new(Box::new(io::Cursor::new(corrupt)), Limits::default()).is_err()
            );
        }
        let mut truncated = valid.clone();
        truncated.pop();
        let mut reader =
            BrotliReader::new(Box::new(io::Cursor::new(truncated)), Limits::default()).unwrap();
        assert!(reader.read_to_end(&mut Vec::new()).is_err());
        let mut pair = valid.clone();
        pair.extend_from_slice(&valid);
        let limits = Limits {
            max_entries: 1,
            ..Default::default()
        };
        let mut reader = BrotliReader::new(Box::new(io::Cursor::new(pair)), limits).unwrap();
        assert!(reader.read_to_end(&mut Vec::new()).is_err());
    }
}
