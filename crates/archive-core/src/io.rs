//! Byte range adapters without filesystem, runtime, or whole-input buffering.
use crate::{Error, Result};
use std::io::{self, Read, Seek, SeekFrom};

/// Fallible retained-output allocation, checked before extending the buffer.
#[cfg(any(feature = "gzip", feature = "xz", feature = "streams"))]
pub(crate) struct BoundedBuffer {
    bytes: Vec<u8>,
    limit: u64,
}
#[cfg(any(feature = "gzip", feature = "xz", feature = "streams"))]
impl BoundedBuffer {
    pub(crate) fn new(limit: u64) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
        }
    }
    pub(crate) fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}
#[cfg(any(feature = "gzip", feature = "xz", feature = "streams"))]
impl io::Write for BoundedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let end = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|&end| end as u64 <= self.limit)
            .ok_or_else(|| error(Error::ResourceLimit("buffered decoded bytes")))?;
        if end > self.bytes.capacity() {
            // Grow geometrically, but never request capacity beyond the budget.
            let capacity =
                (self.bytes.capacity().saturating_mul(2).max(end) as u64).min(self.limit) as usize;
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(|_| error(Error::ResourceLimit("buffered allocation")))?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
/// A stable immutable input. Implementations must return at most `output.len()` bytes.
pub trait RangeSource {
    fn length(&self) -> Result<u64>;
    fn read_at(&self, offset: u64, output: &mut [u8]) -> Result<usize>;
}
impl RangeSource for &[u8] {
    fn length(&self) -> Result<u64> {
        Ok(self.len() as u64)
    }
    fn read_at(&self, offset: u64, output: &mut [u8]) -> Result<usize> {
        let start = usize::try_from(offset).map_err(|_| Error::ResourceLimit("range offset"))?;
        if start >= self.len() {
            return Ok(0);
        }
        let count = output.len().min(self.len() - start);
        output[..count].copy_from_slice(&self[start..start + count]);
        Ok(count)
    }
}
/// Adds a private seek cursor to a caller-supplied range source.
pub struct RangeReader<S> {
    source: S,
    position: u64,
    length: u64,
}
impl<S: RangeSource> RangeReader<S> {
    pub fn new(source: S) -> Result<Self> {
        let length = source.length()?;
        Ok(Self {
            source,
            position: 0,
            length,
        })
    }
    pub fn into_inner(self) -> S {
        self.source
    }
}
fn error(error: Error) -> io::Error {
    match error {
        Error::Io(error) => error,
        other => io::Error::other(other),
    }
}
impl<S: RangeSource> Read for RangeReader<S> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.position >= self.length {
            return Ok(0);
        }
        let maximum = (self.length - self.position).min(output.len() as u64) as usize;
        let count = self
            .source
            .read_at(self.position, &mut output[..maximum])
            .map_err(error)?;
        if count > maximum {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "range source returned an invalid byte count",
            ));
        }
        self.position = self
            .position
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::other("range offset overflow"))?;
        Ok(count)
    }
}
impl<S: RangeSource> Seek for RangeReader<S> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let value = match position {
            SeekFrom::Start(value) => i128::from(value),
            SeekFrom::Current(delta) => i128::from(self.position) + i128::from(delta),
            SeekFrom::End(delta) => i128::from(self.length) + i128::from(delta),
        };
        self.position = u64::try_from(value)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid range seek"))?;
        Ok(self.position)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(any(
        feature = "gzip",
        feature = "xz",
        feature = "streams",
        feature = "sevenz"
    ))]
    #[test]
    fn bounded_buffer_grows_geometrically_for_short_writes() {
        use std::io::Write;
        let mut buffer = BoundedBuffer::new(4096);
        let mut growths = 0;
        for _ in 0..4096 {
            let before = buffer.bytes.capacity();
            buffer.write_all(&[42]).unwrap();
            growths += usize::from(buffer.bytes.capacity() != before);
        }
        assert!(growths <= 13);
        assert_eq!(buffer.bytes.len(), 4096);
        assert!(buffer.write_all(&[42]).is_err());
        assert_eq!(buffer.bytes.len(), 4096);
    }
    #[test]
    fn range_reader_seek_and_read_are_bounded() {
        let mut reader = RangeReader::new(&b"abcdef"[..]).unwrap();
        reader.seek(SeekFrom::End(-2)).unwrap();
        let mut bytes = [0; 8];
        assert_eq!(reader.read(&mut bytes).unwrap(), 2);
        assert_eq!(&bytes[..2], b"ef");
        assert!(reader.seek(SeekFrom::Current(-100)).is_err());
    }
}
