use super::*;
use std::{
    collections::BTreeMap,
    io::{Read, Seek, SeekFrom, Write},
};

struct Output {
    name: archive_fs::ValidatedPath,
    offset: u64,
    expected: u64,
    written: u64,
    metadata: archive_core::EntryMetadata,
}

/// Keep provisional output in one anonymous file until the entire operation verifies.
/// Disjoint ranges let interleaved folder workers share a bounded number of handles.
pub(crate) struct BatchSpool {
    file: std::fs::File,
    outputs: BTreeMap<usize, Output>,
    names: std::collections::BTreeSet<String>,
    end: u64,
    directories: Vec<(archive_fs::ValidatedPath, archive_core::EntryMetadata)>,
}

impl BatchSpool {
    pub(crate) fn new(destination: &archive_fs::Destination) -> io::Result<Self> {
        Ok(Self {
            file: destination.scratch_file()?,
            outputs: BTreeMap::new(),
            names: Default::default(),
            end: 0,
            directories: Vec::new(),
        })
    }

    pub(crate) fn stage(&mut self, id: usize, name: &[u8], size: u64) -> io::Result<()> {
        self.stage_validated(id, archive_fs::ValidatedPath::new(name)?, size)
    }

    pub(crate) fn stage_validated(
        &mut self,
        id: usize,
        name: archive_fs::ValidatedPath,
        size: u64,
    ) -> io::Result<()> {
        let key = name.collision_key().to_owned();
        if self.outputs.contains_key(&id) || !self.names.insert(key) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "duplicate provisional output",
            ));
        }
        let end = self
            .end
            .checked_add(size)
            .ok_or_else(|| io::Error::other("output size overflow"))?;
        self.outputs.insert(
            id,
            Output {
                name,
                offset: self.end,
                expected: size,
                written: 0,
                metadata: Default::default(),
            },
        );
        self.end = end;
        Ok(())
    }

    pub(crate) fn write(&mut self, id: usize, bytes: &[u8]) -> io::Result<()> {
        check_cancelled()?;
        let output = self
            .outputs
            .get_mut(&id)
            .ok_or_else(|| io::Error::other("unexpected output entry ID"))?;
        let written = output
            .written
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| io::Error::other("output size overflow"))?;
        if written > output.expected {
            return Err(io::Error::other("entry decoded size exceeds declaration"));
        }
        self.file
            .seek(SeekFrom::Start(output.offset + output.written))?;
        self.file.write_all(bytes)?;
        output.written = written;
        Ok(())
    }

    pub(crate) fn metadata(
        &mut self,
        id: usize,
        metadata: archive_core::EntryMetadata,
    ) -> io::Result<()> {
        self.outputs
            .get_mut(&id)
            .ok_or_else(|| io::Error::other("unknown metadata entry ID"))?
            .metadata = metadata;
        Ok(())
    }

    pub(crate) fn directory_metadata(
        &mut self,
        name: &[u8],
        metadata: archive_core::EntryMetadata,
    ) -> io::Result<()> {
        self.directory_metadata_validated(archive_fs::ValidatedPath::new(name)?, metadata);
        Ok(())
    }

    pub(crate) fn directory_metadata_validated(
        &mut self,
        name: archive_fs::ValidatedPath,
        metadata: archive_core::EntryMetadata,
    ) {
        self.directories.push((name, metadata));
    }

    pub(crate) fn complete(&self) -> bool {
        self.outputs
            .values()
            .all(|output| output.expected == output.written)
    }

    pub(crate) fn writer(&mut self, id: usize) -> impl Write + '_ {
        SpoolWriter { spool: self, id }
    }

    pub(crate) fn publish(
        mut self,
        destination: &mut archive_fs::Destination,
        mut published: impl FnMut(),
    ) -> io::Result<()> {
        if !self.complete() {
            return Err(io::Error::other("batch output size mismatch"));
        }
        for output in self.outputs.into_values() {
            check_cancelled()?;
            self.file.seek(SeekFrom::Start(output.offset))?;
            destination.file_with_validated_metadata(&output.name, &output.metadata, |sink| {
                let bytes = io::copy(
                    &mut (&mut self.file).take(output.expected),
                    &mut CancellableSink(sink),
                )?;
                if bytes != output.expected {
                    return Err(io::Error::other("truncated provisional output"));
                }
                check_cancelled()?;
                Ok(bytes)
            })?;
            published();
        }
        self.directories
            .sort_by_key(|(name, _)| std::cmp::Reverse(name.depth()));
        for (name, metadata) in self.directories {
            check_cancelled()?;
            destination.directory_metadata_validated(&name, &metadata)?;
        }
        Ok(())
    }
}

struct SpoolWriter<'a> {
    spool: &'a mut BatchSpool,
    id: usize,
}

impl Write for SpoolWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.spool.write(self.id, bytes)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        check_cancelled()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interleaved_ranges_publish_exact_contents_including_empty_files() {
        let root = tempfile::tempdir().unwrap();
        let mut destination = archive_fs::Destination::open(root.path()).unwrap();
        let mut spool = BatchSpool::new(&destination).unwrap();
        spool.stage(0, b"a", 4).unwrap();
        spool.stage(1, b"b", 3).unwrap();
        spool.stage(2, b"empty", 0).unwrap();
        spool.write(0, b"ab").unwrap();
        spool.write(1, b"xyz").unwrap();
        spool.write(0, b"cd").unwrap();
        spool.publish(&mut destination, || {}).unwrap();
        assert_eq!(std::fs::read(root.path().join("a")).unwrap(), b"abcd");
        assert_eq!(std::fs::read(root.path().join("b")).unwrap(), b"xyz");
        assert_eq!(std::fs::read(root.path().join("empty")).unwrap(), b"");
    }

    #[test]
    fn incomplete_batch_publishes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let mut destination = archive_fs::Destination::open(root.path()).unwrap();
        let mut spool = BatchSpool::new(&destination).unwrap();
        spool.stage(0, b"complete", 2).unwrap();
        spool.stage(1, b"incomplete", 2).unwrap();
        spool.write(0, b"ok").unwrap();
        spool.write(1, b"x").unwrap();
        assert!(spool.publish(&mut destination, || {}).is_err());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
}
