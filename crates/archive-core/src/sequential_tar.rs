//! Forward-only USTAR/PAX enumeration with explicit payload consumption.
use crate::{Entry, EntryId, EntryKind, Error, ExtractReport, Limits, Result};
use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
};
pub struct SequentialTar<R> {
    reader: R,
    limits: Limits,
    index: usize,
    headers: u64,
    input_bytes: u64,
    metadata_bytes: u64,
    decoded_bytes: u64,
    remaining: u64,
    padding: u64,
    current_size: u64,
    current: bool,
    done: bool,
    global: BTreeMap<Vec<u8>, Vec<u8>>,
    current_metadata: crate::EntryMetadata,
}
impl<R: Read> SequentialTar<R> {
    pub fn new(reader: R, limits: Limits) -> Self {
        Self {
            reader,
            limits,
            index: 0,
            headers: 0,
            input_bytes: 0,
            metadata_bytes: 0,
            decoded_bytes: 0,
            remaining: 0,
            padding: 0,
            current_size: 0,
            current: false,
            done: false,
            global: BTreeMap::new(),
            current_metadata: Default::default(),
        }
    }
    fn exact(&mut self, bytes: &mut [u8]) -> Result<()> {
        let next = self
            .input_bytes
            .checked_add(bytes.len() as u64)
            .ok_or(Error::ResourceLimit("input bytes"))?;
        if next > self.limits.max_input_bytes {
            return Err(Error::ResourceLimit("input bytes"));
        }
        self.reader.read_exact(bytes)?;
        self.input_bytes = next;
        Ok(())
    }
    fn metadata(&mut self, size: u64) -> Result<()> {
        self.metadata_bytes = self
            .metadata_bytes
            .checked_add(size)
            .ok_or(Error::ResourceLimit("metadata bytes"))?;
        if self.metadata_bytes > self.limits.max_metadata_bytes {
            return Err(Error::ResourceLimit("metadata bytes"));
        }
        Ok(())
    }
    fn padding(&mut self) -> Result<()> {
        if self.padding != 0 {
            let mut padding = [0u8; 511];
            let size = self.padding as usize;
            self.exact(&mut padding[..size])?;
            self.padding = 0;
        }
        Ok(())
    }
    /// Advances only after the previous member has been copied or explicitly skipped.
    pub fn next_entry(&mut self) -> Result<Option<Entry>> {
        if self.done {
            return Ok(None);
        }
        if self.current {
            return Err(Error::Malformed(
                "consume or skip the current TAR member before advancing".into(),
            ));
        }
        self.padding()?;
        let mut local = BTreeMap::new();
        let mut long_name = None;
        loop {
            let mut bytes = [0u8; 512];
            self.exact(&mut bytes)?;
            if bytes.iter().all(|b| *b == 0) {
                let mut second = [0u8; 512];
                self.exact(&mut second)?;
                if second.iter().any(|b| *b != 0) {
                    return Err(Error::Malformed("TAR end marker".into()));
                }
                self.done = true;
                return Ok(None);
            }
            let header = tar::Header::from_byte_slice(&bytes);
            self.headers = self
                .headers
                .checked_add(1)
                .ok_or(Error::ResourceLimit("TAR headers"))?;
            if self.headers > self.limits.max_entries.saturating_mul(2) {
                return Err(Error::ResourceLimit("TAR headers"));
            }
            let expected = header.cksum()?;
            let sum: u32 = bytes
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    if (148..156).contains(&i) {
                        32
                    } else {
                        u32::from(*b)
                    }
                })
                .sum();
            if sum != expected {
                return Err(Error::Integrity("TAR header checksum".into()));
            }
            let size = header.size()?;
            let entry_type = bytes[156];
            if matches!(entry_type, b'x' | b'g' | b'L' | b'K') {
                self.metadata(size)?;
                let length =
                    usize::try_from(size).map_err(|_| Error::ResourceLimit("metadata bytes"))?;
                let mut body = vec![0u8; length];
                self.exact(&mut body)?;
                self.padding = (512 - size % 512) % 512;
                self.padding()?;
                match entry_type {
                    b'x' => {
                        if !local.is_empty() {
                            return Err(Error::Malformed("multiple local PAX headers".into()));
                        }
                        local = pax(&body)?;
                    }
                    b'g' => {
                        for (key, value) in pax(&body)? {
                            if value.is_empty() {
                                self.global.remove(&key);
                            } else {
                                self.global.insert(key, value);
                            }
                        }
                    }
                    b'L' => {
                        while body.last() == Some(&0) {
                            body.pop();
                        }
                        long_name = Some(body);
                    }
                    b'K' => {}
                    _ => {}
                }
                continue;
            }
            if self.index as u64 >= self.limits.max_entries {
                return Err(Error::ResourceLimit("entries"));
            }
            let value = |key: &[u8]| local.get(key).or_else(|| self.global.get(key));
            let name = if let Some(path) = value(b"path") {
                path.clone()
            } else if let Some(name) = long_name {
                name
            } else {
                header.path_bytes().into_owned()
            };
            let size = match value(b"size") {
                Some(size) => std::str::from_utf8(size)
                    .ok()
                    .and_then(|value| value.parse::<u64>().ok())
                    .ok_or_else(|| Error::Malformed("PAX size".into()))?,
                None => size,
            };
            if name
                .split(|byte| matches!(byte, b'/' | b'\\'))
                .filter(|part| !part.is_empty())
                .count()
                > self.limits.max_nesting_depth
            {
                return Err(Error::ResourceLimit("entry path depth"));
            }
            if size > self.limits.max_entry_bytes {
                return Err(Error::ResourceLimit("entry decoded bytes"));
            }
            let total = self
                .decoded_bytes
                .checked_add(size)
                .ok_or(Error::ResourceLimit("total decoded bytes"))?;
            if total > self.limits.max_total_bytes {
                return Err(Error::ResourceLimit("total decoded bytes"));
            }
            self.decoded_bytes = total;
            self.metadata(name.len() as u64)?;
            let kind = match entry_type {
                0 | b'0' => EntryKind::File,
                b'5' => EntryKind::Directory,
                b'1' | b'2' => EntryKind::Link,
                _ => EntryKind::Other,
            };
            if kind == EntryKind::Other {
                return Err(Error::Unsupported("TAR special or sparse entry".into()));
            }
            let entry = Entry {
                id: EntryId(self.index),
                name: String::from_utf8_lossy(&name).into_owned(),
                raw_name: name,
                kind,
                size,
                compressed_size: Some(size),
                compression: "stored".into(),
                encrypted: false,
            };
            self.current_metadata = crate::EntryMetadata {
                modified: Some(crate::StoredTimestamp::UnixSeconds(header.mtime()?)),
                unix_mode: Some(header.mode()?),
                user_id: Some(header.uid()?),
                group_id: Some(header.gid()?),
                link_target: header.link_name_bytes().map(|name| name.into_owned()),
                format: Some(crate::EntryFormatMetadata::Tar {
                    stored_type: entry_type,
                }),
            };
            self.index += 1;
            self.remaining = size;
            self.current_size = size;
            self.current = true;
            self.padding = (512 - size % 512) % 512;
            return Ok(Some(entry));
        }
    }
    /// Metadata from the current member's stored TAR header.
    pub fn current_metadata(&self) -> Result<crate::EntryMetadata> {
        if !self.current {
            return Err(Error::Malformed("no current TAR member".into()));
        }
        Ok(self.current_metadata.clone())
    }
    pub fn copy_current(&mut self, writer: &mut impl Write) -> Result<ExtractReport> {
        if !self.current {
            return Err(Error::Malformed("no current TAR member".into()));
        }
        let mut bytes = [0u8; 65536];
        while self.remaining != 0 {
            let n = self.remaining.min(bytes.len() as u64) as usize;
            self.exact(&mut bytes[..n])?;
            self.remaining -= n as u64;
            writer.write_all(&bytes[..n])?;
        }
        self.padding()?;
        self.current = false;
        Ok(ExtractReport {
            bytes: self.current_size,
            entries: 1,
            verified: true,
        })
    }
    pub fn skip_current(&mut self) -> Result<ExtractReport> {
        self.copy_current(&mut io::sink())
    }
    pub fn into_inner(self) -> R {
        self.reader
    }
}
fn pax(mut bytes: &[u8]) -> Result<BTreeMap<Vec<u8>, Vec<u8>>> {
    let mut values = BTreeMap::new();
    while !bytes.is_empty() {
        let space = bytes
            .iter()
            .position(|b| *b == b' ')
            .ok_or_else(|| Error::Malformed("PAX record length".into()))?;
        let length = std::str::from_utf8(&bytes[..space])
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or_else(|| Error::Malformed("PAX record length".into()))?;
        if length <= space + 2 || length > bytes.len() || bytes[length - 1] != b'\n' {
            return Err(Error::Malformed("PAX record extent".into()));
        }
        let record = &bytes[space + 1..length - 1];
        let equal = record
            .iter()
            .position(|b| *b == b'=')
            .ok_or_else(|| Error::Malformed("PAX record separator".into()))?;
        if equal == 0 {
            return Err(Error::Malformed("PAX empty key".into()));
        }
        values.insert(record[..equal].to_vec(), record[equal + 1..].to_vec());
        bytes = &bytes[length..];
    }
    Ok(values)
}
