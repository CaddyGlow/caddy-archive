//! Optional borrowed-byte WIM adapter. Images are explicit container selectors.
//! This initial adapter bounds whole-resource allocations; it is not a range source.
use std::io::Write;

use crate::{Entry, EntryId, EntryKind, Error, ExtractReport, Limits, Result};

fn parse_error(error: wim_format::ParseError) -> Error {
    Error::Malformed(format!("WIM: {error:?}"))
}

/// Container image metadata, distinct from the files in a selected image.
#[derive(Clone, Debug, serde::Serialize)]
pub struct WimImage {
    /// One-based image index.
    pub index: u32,
    /// Optional image name from the WIM XML resource.
    pub name: Option<String>,
    /// Optional image description from the WIM XML resource.
    pub description: Option<String>,
}

/// List image identities after bounding XML and lookup-resource allocations.
pub fn images(bytes: &[u8], limits: Limits) -> Result<Vec<WimImage>> {
    if bytes.len() as u64 > limits.max_input_bytes {
        return Err(Error::ResourceLimit("input bytes"));
    }
    let header = wim_format::Header::parse_seekable(bytes).map_err(parse_error)?;
    if header.total_parts != 1 {
        return Err(Error::Unsupported(
            "split WIM requires explicit part resolver".into(),
        ));
    }
    if u64::from(header.image_count) > limits.max_entries {
        return Err(Error::ResourceLimit("WIM images"));
    }
    if header.blob_table.uncompressed_size > limits.max_metadata_bytes
        || header.xml_data.uncompressed_size > limits.max_metadata_bytes
    {
        return Err(Error::ResourceLimit("WIM metadata bytes"));
    }
    if header
        .blob_table
        .uncompressed_size
        .saturating_add(header.xml_data.uncompressed_size)
        > limits.max_active_workspace_bytes
    {
        return Err(Error::ResourceLimit("WIM active workspace bytes"));
    }
    if u64::from(header.chunk_size) > limits.max_dictionary_bytes {
        return Err(Error::ResourceLimit("WIM decoder workspace"));
    }
    let archive = wim_format::archive::Archive::open(bytes).map_err(parse_error)?;
    let xml = archive.xml().map_err(parse_error)?;
    (1..=header.image_count)
        .map(|index| {
            let xml_index = i32::try_from(index).map_err(|_| Error::ResourceLimit("WIM images"))?;
            Ok(WimImage {
                index,
                name: xml.name(xml_index).map(str::to_owned),
                description: xml.description(xml_index).map(str::to_owned),
            })
        })
        .collect()
}

/// A selected WIM image with caller-retained source bytes.
pub struct WimArchive<'a> {
    archive: wim_format::archive::Archive<'a>,
    entries: Vec<Entry>,
    hashes: Vec<Option<[u8; 20]>>,
    stored_metadata: Vec<crate::EntryMetadata>,
    limits: Limits,
    image: u32,
}

impl<'a> WimArchive<'a> {
    /// Select exactly one image by its XML name, without numeric-name guessing.
    pub fn open_by_name(bytes: &'a [u8], name: &str, limits: Limits) -> Result<Self> {
        let mut matches = images(bytes, limits)?
            .into_iter()
            .filter(|image| image.name.as_deref() == Some(name));
        let selected = matches
            .next()
            .ok_or_else(|| Error::Malformed("unknown WIM image name".into()))?;
        if matches.next().is_some() {
            return Err(Error::Malformed("ambiguous WIM image name".into()));
        }
        Self::open(bytes, selected.index, limits)
    }
    /// Open a one-based image, bounding lookup, image metadata and decoder resources.
    pub fn open(bytes: &'a [u8], image: u32, limits: Limits) -> Result<Self> {
        if bytes.len() as u64 > limits.max_input_bytes {
            return Err(Error::ResourceLimit("input bytes"));
        }
        let header = wim_format::Header::parse_seekable(bytes).map_err(parse_error)?;
        if header.total_parts != 1 {
            return Err(Error::Unsupported(
                "split WIM requires explicit part resolver".into(),
            ));
        }
        if image == 0 || image > header.image_count {
            return Err(Error::Malformed("invalid WIM image selector".into()));
        }
        if header.blob_table.uncompressed_size > limits.max_metadata_bytes
            || header.xml_data.uncompressed_size > limits.max_metadata_bytes
        {
            return Err(Error::ResourceLimit("WIM metadata bytes"));
        }
        if header
            .blob_table
            .uncompressed_size
            .saturating_add(header.xml_data.uncompressed_size)
            > limits.max_active_workspace_bytes
        {
            return Err(Error::ResourceLimit("WIM active workspace bytes"));
        }
        if u64::from(header.chunk_size) > limits.max_dictionary_bytes {
            return Err(Error::ResourceLimit("WIM decoder workspace"));
        }
        let archive = wim_format::archive::Archive::open(bytes).map_err(parse_error)?;
        let metadata_blob = archive
            .lookup
            .metadata
            .get((image - 1) as usize)
            .ok_or_else(|| Error::Malformed("missing WIM image metadata".into()))?;
        if metadata_blob.size > limits.max_metadata_bytes {
            return Err(Error::ResourceLimit("WIM image metadata bytes"));
        }
        let metadata_resource = archive
            .lookup
            .resources
            .get(metadata_blob.resource_index)
            .ok_or_else(|| Error::Malformed("missing WIM metadata resource".into()))?;
        if metadata_resource.uncompressed_size > limits.max_metadata_bytes {
            return Err(Error::ResourceLimit("WIM metadata resource bytes"));
        }
        for resource in &archive.lookup.resources {
            if resource
                .uncompressed_size
                .saturating_add(u64::from(resource.chunk_size))
                > limits.max_active_workspace_bytes
            {
                return Err(Error::ResourceLimit("WIM active workspace bytes"));
            }
            if resource.uncompressed_size > limits.max_total_bytes {
                return Err(Error::ResourceLimit("WIM decoded resource bytes"));
            }
            if u64::from(resource.chunk_size) > limits.max_dictionary_bytes {
                return Err(Error::ResourceLimit("WIM decoder workspace"));
            }
        }
        let metadata_bytes = archive.read_metadata(image).map_err(parse_error)?;
        let metadata =
            wim_format::metadata::Metadata::parse(&metadata_bytes).map_err(parse_error)?;
        if metadata.nodes.len().saturating_sub(1) as u64 > limits.max_entries {
            return Err(Error::ResourceLimit("entries"));
        }
        let mut paths: Vec<String> = vec![String::new(); metadata.nodes.len()];
        let mut depths = vec![0usize; metadata.nodes.len()];
        let mut entries = Vec::new();
        let mut hashes = Vec::new();
        let mut stored_metadata = Vec::new();
        let mut total = 0u64;
        let mut names = 0u64;
        for (index, node) in metadata.nodes.iter().enumerate().skip(1) {
            let units: Vec<_> = node
                .entry
                .name
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            let leaf = String::from_utf16(&units)
                .map_err(|_| Error::Unsupported("non-Unicode WIM name".into()))?;
            if leaf.contains(['/', '\\', '\0']) || leaf == "." || leaf == ".." {
                return Err(Error::Malformed("unsafe WIM filename".into()));
            }
            let parent = node
                .parent
                .ok_or_else(|| Error::Malformed("missing WIM parent".into()))?;
            if parent >= index {
                return Err(Error::Malformed("invalid WIM parent ordering".into()));
            }
            depths[index] = depths[parent]
                .checked_add(1)
                .ok_or(Error::ResourceLimit("WIM nesting depth"))?;
            if depths[index] > limits.max_nesting_depth {
                return Err(Error::ResourceLimit("WIM nesting depth"));
            }
            let path = if paths[parent].is_empty() {
                leaf
            } else {
                format!("{}/{leaf}", paths[parent])
            };
            paths[index] = path.clone();
            names = names
                .checked_add(path.len() as u64)
                .ok_or(Error::ResourceLimit("metadata bytes"))?;
            if names > limits.max_metadata_bytes {
                return Err(Error::ResourceLimit("metadata bytes"));
            }
            let inode = metadata
                .inode_entry(index)
                .ok_or_else(|| Error::Malformed("missing WIM inode".into()))?;
            let hash = inode
                .streams
                .iter()
                .find(|stream| {
                    stream.kind == wim_format::metadata::StreamType::Data && stream.name.is_empty()
                })
                .map(|stream| stream.hash)
                .filter(|hash| *hash != [0; 20]);
            let size = match hash {
                Some(hash) => {
                    archive
                        .lookup
                        .find(&hash)
                        .ok_or_else(|| Error::Malformed("missing WIM file blob".into()))?
                        .size
                }
                None => 0,
            };
            if size > limits.max_entry_bytes {
                return Err(Error::ResourceLimit("entry decoded bytes"));
            }
            total = total
                .checked_add(size)
                .ok_or(Error::ResourceLimit("total decoded bytes"))?;
            if total > limits.max_total_bytes {
                return Err(Error::ResourceLimit("total decoded bytes"));
            }
            let kind = if inode.attributes & 0x400 != 0 {
                EntryKind::Link
            } else if inode.attributes & 0x4000 != 0 {
                EntryKind::Other
            } else if inode.is_directory() {
                EntryKind::Directory
            } else {
                EntryKind::File
            };
            entries.push(Entry {
                id: EntryId(entries.len()),
                raw_name: node.entry.name.to_vec(),
                name: path,
                kind,
                size,
                compressed_size: None,
                compression: "WIM resource".into(),
                encrypted: false,
            });
            hashes.push(hash);
            stored_metadata.push(crate::EntryMetadata {
                modified: inode
                    .last_write_time
                    .checked_sub(116_444_736_000_000_000)
                    .map(|ticks| crate::StoredTimestamp::UnixSeconds(ticks / 10_000_000)),
                unix_mode: if inode.attributes & 1 != 0 {
                    Some(if kind == EntryKind::Directory {
                        0o555
                    } else {
                        0o444
                    })
                } else {
                    None
                },
                ..Default::default()
            });
        }
        Ok(Self {
            archive,
            entries,
            hashes,
            stored_metadata,
            limits,
            image,
        })
    }

    /// Selected one-based image index.
    pub fn image(&self) -> u32 {
        self.image
    }
    /// Number of container images, distinct from file entries.
    pub fn image_count(&self) -> u32 {
        self.archive.header.image_count
    }
    /// Selected-image file metadata. Raw names contain stored UTF-16LE leaf bytes.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    /// Stored modification time and read-only permissions for a selected image entry.
    pub fn entry_metadata(&self, id: EntryId) -> Result<crate::EntryMetadata> {
        self.stored_metadata
            .get(id.0)
            .cloned()
            .ok_or_else(|| Error::Malformed("unknown WIM entry ID".into()))
    }
    /// Decode and verify a whole blob with an explicit allocation ceiling.
    pub fn read_entry(&self, id: EntryId, maximum: u64) -> Result<Vec<u8>> {
        let entry = self
            .entries
            .get(id.0)
            .ok_or_else(|| Error::Malformed("unknown entry ID".into()))?;
        if !matches!(entry.kind, EntryKind::File | EntryKind::Directory) {
            return Err(Error::Unsupported(
                "WIM links, encrypted raw streams and special files".into(),
            ));
        }
        if entry.size > maximum.min(self.limits.max_entry_bytes) {
            return Err(Error::ResourceLimit("buffered entry bytes"));
        }
        match self.hashes[id.0] {
            Some(hash) => self.archive.read_blob(&hash).map_err(parse_error),
            None => Ok(Vec::new()),
        }
    }
    /// Deliver bytes only after their WIM blob hash has verified.
    pub fn extract(&self, id: EntryId, output: &mut impl Write) -> Result<ExtractReport> {
        let bytes = self.read_entry(id, self.limits.max_entry_bytes)?;
        output.write_all(&bytes)?;
        Ok(ExtractReport {
            bytes: bytes.len() as u64,
            entries: 1,
            verified: true,
        })
    }
    /// Verify every supported selected-image payload.
    pub fn test(&self) -> Result<ExtractReport> {
        let mut report = ExtractReport {
            verified: true,
            ..Default::default()
        };
        for entry in &self.entries {
            let next = self.extract(entry.id, &mut std::io::sink())?;
            report.bytes = report
                .bytes
                .checked_add(next.bytes)
                .ok_or(Error::ResourceLimit("total decoded bytes"))?;
            report.entries += 1;
        }
        Ok(report)
    }
}
