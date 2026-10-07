//! Native archive extraction with strict names and verified publication.
/// Version of this library, as declared in `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

use std::{collections::BTreeSet, io, path::Path};

/// Reject ambiguous and platform-dependent names before touching the destination.
pub fn validate_name(name: &[u8]) -> io::Result<Vec<String>> {
    let name = std::str::from_utf8(name).map_err(|_| invalid("non-UTF-8 destination name"))?;
    if name.len() > 4096
        || name.starts_with('/')
        || name.contains(['\\', ':', '\0', '<', '>', '|', '?', '*', '"'])
    {
        return Err(invalid("unsafe archive path"));
    }
    let name = name.strip_suffix('/').unwrap_or(name);
    let parts: Vec<_> = name.split('/').collect();
    if parts.len() > 256 {
        return Err(invalid("destination nesting limit exceeded"));
    }
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == ".." || p.ends_with(['.', ' ']))
    {
        return Err(invalid("unsafe archive path component"));
    }
    for part in &parts {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return Err(invalid("reserved destination name"));
        }
    }
    Ok(parts.into_iter().map(str::to_owned).collect())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

/// Capture the modification time and permissions available from a native source.
pub fn source_metadata(path: &Path) -> io::Result<archive_core::EntryMetadata> {
    let metadata = std::fs::symlink_metadata(path)?;
    let modified = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| invalid("timestamps before the Unix epoch are unsupported"))?
        .as_secs();
    let mut result = archive_core::EntryMetadata {
        modified: Some(archive_core::StoredTimestamp::UnixSeconds(modified)),
        ..Default::default()
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        result.unix_mode = Some(metadata.mode());
        result.user_id = Some(u64::from(metadata.uid()));
        result.group_id = Some(u64::from(metadata.gid()));
    }
    #[cfg(windows)]
    {
        result.unix_mode = Some(
            (if metadata.is_dir() { 0o40755 } else { 0o100644 })
                & if metadata.permissions().readonly() {
                    !0o222
                } else {
                    u32::MAX
                },
        );
    }
    Ok(result)
}

/// Capture CAB-compatible local DOS wall time instead of an absolute Unix time.
pub fn source_dos_metadata(path: &Path) -> io::Result<archive_core::EntryMetadata> {
    let mut metadata = source_metadata(path)?;
    if let Some(archive_core::StoredTimestamp::UnixSeconds(seconds)) = metadata.modified {
        #[cfg(any(unix, windows))]
        {
            let seconds = libc::time_t::try_from(seconds)
                .map_err(|_| invalid("timestamp exceeds system range"))?;
            let mut local: libc::tm = unsafe { std::mem::zeroed() };
            #[cfg(unix)]
            if unsafe { libc::localtime_r(&seconds, &mut local) }.is_null() {
                return Err(invalid("timestamp exceeds system range"));
            }
            #[cfg(windows)]
            if unsafe { libc::localtime_s(&mut local, &seconds) } != 0 {
                return Err(invalid("timestamp exceeds system range"));
            }
            metadata.modified = Some(archive_core::StoredTimestamp::DosLocal {
                year: u16::try_from(local.tm_year + 1900)
                    .map_err(|_| invalid("DOS timestamp year"))?,
                month: (local.tm_mon + 1) as u8,
                day: local.tm_mday as u8,
                hour: local.tm_hour as u8,
                minute: local.tm_min as u8,
                second: local.tm_sec as u8,
            });
        }
    }
    Ok(metadata)
}

/// Apply stored metadata to an already admitted file handle without following paths.
/// Ownership is recorded in archives but is not changed during extraction.
pub fn apply_metadata(
    file: &std::fs::File,
    metadata: &archive_core::EntryMetadata,
) -> io::Result<()> {
    if let Some(modified) = metadata.modified {
        let time = match modified {
            archive_core::StoredTimestamp::UnixSeconds(seconds) => std::time::UNIX_EPOCH
                .checked_add(std::time::Duration::from_secs(seconds))
                .ok_or_else(|| invalid("timestamp exceeds system range"))?,
            archive_core::StoredTimestamp::DosLocal {
                year,
                month,
                day,
                hour,
                minute,
                second,
            } => {
                #[cfg(any(unix, windows))]
                {
                    let mut local: libc::tm = unsafe { std::mem::zeroed() };
                    local.tm_year = i32::from(year) - 1900;
                    local.tm_mon = i32::from(month) - 1;
                    local.tm_mday = i32::from(day);
                    local.tm_hour = i32::from(hour);
                    local.tm_min = i32::from(minute);
                    local.tm_sec = i32::from(second);
                    local.tm_isdst = -1;
                    #[cfg(unix)]
                    let seconds = unsafe { libc::mktime(&mut local) };
                    #[cfg(windows)]
                    let seconds = unsafe {
                        unsafe extern "C" {
                            fn _mktime64(time: *mut libc::tm) -> i64;
                        }
                        _mktime64(&mut local)
                    };
                    if seconds < 0 {
                        return Err(invalid("DOS timestamp exceeds system range"));
                    }
                    std::time::UNIX_EPOCH
                        .checked_add(std::time::Duration::from_secs(seconds as u64))
                        .ok_or_else(|| invalid("timestamp exceeds system range"))?
                }
                #[cfg(not(any(unix, windows)))]
                {
                    let _ = (year, month, day, hour, minute, second);
                    return Err(io::ErrorKind::Unsupported.into());
                }
            }
        };
        file.set_times(std::fs::FileTimes::new().set_modified(time))?;
    }
    if let Some(mode) = metadata.unix_mode {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(mode & 0o777))?;
        }
        #[cfg(windows)]
        {
            let mut permissions = file.metadata()?.permissions();
            permissions.set_readonly(mode & 0o222 == 0);
            file.set_permissions(permissions)?;
        }
        #[cfg(not(any(unix, windows)))]
        let _ = mode;
    }
    Ok(())
}

/// Cancellation is independent of progress rendering and checked at sink writes.
#[derive(Clone, Default)]
pub struct CancellationToken(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl CancellationToken {
    /// Request cancellation of queued and active extraction writes.
    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    /// Check whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
    fn check(&self) -> io::Result<()> {
        if self.is_cancelled() {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "operation cancelled",
            ))
        } else {
            Ok(())
        }
    }
}
struct CancellableWriter<'a> {
    file: &'a mut std::fs::File,
    cancellation: &'a CancellationToken,
}
impl std::io::Write for CancellableWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.cancellation.check()?;
        std::io::Write::write(self.file, bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.cancellation.check()?;
        std::io::Write::flush(self.file)
    }
}

/// Operation-owned destination. Existing files are never overwritten.
pub struct Destination {
    #[cfg(unix)]
    root: std::fs::File,
    #[cfg(windows)]
    root: windows::Root,
    names: BTreeSet<String>,
}

impl Destination {
    /// Restore a directory's metadata after its children have been published.
    pub fn directory_metadata(
        &self,
        name: &[u8],
        metadata: &archive_core::EntryMetadata,
    ) -> io::Result<()> {
        let parts = validate_name(name)?;
        #[cfg(unix)]
        {
            apply_metadata(&self.parent(&parts)?, metadata)
        }
        #[cfg(windows)]
        {
            self.root.directory_metadata(&parts, metadata)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (parts, metadata);
            Err(io::ErrorKind::Unsupported.into())
        }
    }
    /// Create an anonymous scratch file on the destination filesystem.
    /// It is never published and is removed when its last handle is closed.
    pub fn scratch_file(&self) -> io::Result<std::fs::File> {
        #[cfg(unix)]
        {
            let temporary = Temporary::new(self.root.try_clone()?)?;
            // Unlink the operation-owned name while retaining the inode for spooling.
            temporary.file.try_clone()
        }
        #[cfg(windows)]
        {
            self.root.scratch_file()
        }
        #[cfg(not(any(unix, windows)))]
        {
            Err(io::ErrorKind::Unsupported.into())
        }
    }
    /// Reserve an entry and open its operation-owned provisional output.
    /// Retain this object until the archive operation verifies the corresponding data.
    pub fn stage_file(&mut self, name: &[u8]) -> io::Result<StagedFile> {
        let parts = self.reserve(name)?;
        let leaf = parts.last().ok_or_else(|| invalid("empty name"))?;
        #[cfg(unix)]
        {
            let parent = self.parent(&parts[..parts.len() - 1])?;
            let temporary = Temporary::new(parent)?;
            let target =
                std::ffi::CString::new(leaf.as_bytes()).map_err(|_| invalid("NUL name"))?;
            Ok(StagedFile { temporary, target })
        }
        #[cfg(windows)]
        {
            let parent = self.root.parent(&parts[..parts.len() - 1])?;
            let temporary = tempfile::NamedTempFile::new_in(&parent.path)?;
            Ok(StagedFile {
                temporary,
                parent,
                target: leaf.clone(),
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = leaf;
            Err(io::ErrorKind::Unsupported.into())
        }
    }
    /// Extract through a cancellable provisional sink; cancellation prevents publication.
    pub fn file_cancellable<F>(
        &mut self,
        name: &[u8],
        cancellation: &CancellationToken,
        verified_writer: F,
    ) -> io::Result<u64>
    where
        F: FnOnce(&mut dyn std::io::Write) -> io::Result<u64>,
    {
        cancellation.check()?;
        self.file(name, |file| {
            let bytes = verified_writer(&mut CancellableWriter { file, cancellation })?;
            cancellation.check()?;
            Ok(bytes)
        })
    }
    /// Open an existing extraction root without following its final symlink.
    pub fn open(path: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let root = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(path)?;
            Ok(Self {
                root,
                names: BTreeSet::new(),
            })
        }
        #[cfg(windows)]
        {
            Ok(Self {
                root: windows::Root::open(path)?,
                names: BTreeSet::new(),
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = path;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "handle-relative extraction is unavailable on this platform",
            ))
        }
    }

    fn reserve(&mut self, name: &[u8]) -> io::Result<Vec<String>> {
        let parts = validate_name(name)?;
        let key = parts.join("/").to_lowercase();
        if !self.names.insert(key) {
            return Err(invalid("duplicate or normalized destination collision"));
        }
        Ok(parts)
    }

    /// Create a directory entry. Links and special entries must be rejected by callers.
    pub fn directory(&mut self, name: &[u8]) -> io::Result<()> {
        let parts = self.reserve(name)?;
        #[cfg(unix)]
        {
            self.parent(&parts).map(|_| ())
        }
        #[cfg(windows)]
        {
            self.root.parent(&parts).map(|_| ())
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = parts;
            Err(io::ErrorKind::Unsupported.into())
        }
    }

    /// Publish only after the writer returns successful integrity verification.
    pub fn file<F>(&mut self, name: &[u8], verified_writer: F) -> io::Result<u64>
    where
        F: FnOnce(&mut std::fs::File) -> io::Result<u64>,
    {
        let mut staged = self.stage_file(name)?;
        let bytes = verified_writer(staged.file_mut())?;
        staged.publish()?;
        Ok(bytes)
    }

    /// Publish verified content and restore metadata through its retained file handle.
    pub fn file_with_metadata<F>(
        &mut self,
        name: &[u8],
        metadata: &archive_core::EntryMetadata,
        verified_writer: F,
    ) -> io::Result<u64>
    where
        F: FnOnce(&mut std::fs::File) -> io::Result<u64>,
    {
        let mut staged = self.stage_file(name)?;
        let bytes = verified_writer(staged.file_mut())?;
        staged.publish_with_metadata(metadata)?;
        Ok(bytes)
    }

    #[cfg(unix)]
    fn parent(&self, parts: &[String]) -> io::Result<std::fs::File> {
        use std::os::fd::{AsRawFd, FromRawFd};
        let mut dir = self.root.try_clone()?;
        for part in parts {
            let name = std::ffi::CString::new(part.as_bytes()).map_err(|_| invalid("NUL name"))?;
            let result = unsafe { libc::mkdirat(dir.as_raw_fd(), name.as_ptr(), 0o700) };
            if result != 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
                return Err(io::Error::last_os_error());
            }
            let fd = unsafe {
                libc::openat(
                    dir.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            dir = unsafe { std::fs::File::from_raw_fd(fd) };
        }
        Ok(dir)
    }
}

/// Provisional output removed on drop unless published after successful verification.
pub struct StagedFile {
    #[cfg(unix)]
    temporary: Temporary,
    #[cfg(unix)]
    target: std::ffi::CString,
    #[cfg(windows)]
    temporary: tempfile::NamedTempFile,
    #[cfg(windows)]
    parent: windows::Parent,
    #[cfg(windows)]
    target: String,
}
impl StagedFile {
    /// Native sink for batched archive decoding.
    pub fn file_mut(&mut self) -> &mut std::fs::File {
        #[cfg(unix)]
        {
            &mut self.temporary.file
        }
        #[cfg(windows)]
        {
            self.temporary.as_file_mut()
        }
        #[cfg(not(any(unix, windows)))]
        {
            unreachable!("unsupported platform cannot create StagedFile")
        }
    }
    /// Atomically publish without overwriting an existing destination.
    pub fn publish(self) -> io::Result<()> {
        self.publish_with_metadata(&archive_core::EntryMetadata::default())
    }

    /// Publish without overwriting, applying stored metadata through the output handle.
    pub fn publish_with_metadata(self, metadata: &archive_core::EntryMetadata) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            apply_metadata(&self.temporary.file, metadata)?;
            self.temporary.file.sync_all()?;
            let fd = self.temporary.parent.as_raw_fd();
            let result = unsafe {
                libc::linkat(
                    fd,
                    self.temporary.name.as_ptr(),
                    fd,
                    self.target.as_ptr(),
                    0,
                )
            };
            if result != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
        #[cfg(windows)]
        {
            self.temporary.as_file().sync_all()?;
            let file = self
                .temporary
                .persist_noclobber(self.parent.path.join(&self.target))
                .map_err(|e| e.error)?;
            // Apply read-only attributes after publication so failed provisional files
            // remain removable by NamedTempFile's cleanup.
            apply_metadata(&file, metadata)?;
            file.sync_all()?;
            Ok(())
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = metadata;
            Err(io::ErrorKind::Unsupported.into())
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{
        os::windows::fs::{MetadataExt, OpenOptionsExt},
        path::PathBuf,
    };
    pub struct Root {
        path: PathBuf,
        _handles: Vec<std::fs::File>,
    }
    pub struct Parent {
        pub path: PathBuf,
        _handles: Vec<std::fs::File>,
    }
    fn pin(path: &Path) -> io::Result<std::fs::File> {
        // Denying FILE_SHARE_DELETE pins the name until every directory handle is released.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .custom_flags(0x00200000 | 0x02000000)
            .open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
            return Err(invalid("destination reparse point or non-directory"));
        }
        Ok(file)
    }
    impl Root {
        pub fn directory_metadata(
            &self,
            parts: &[String],
            metadata: &archive_core::EntryMetadata,
        ) -> io::Result<()> {
            let parent = self.parent(parts)?;
            apply_metadata(&pin(&parent.path)?, metadata)
        }
        pub fn scratch_file(&self) -> io::Result<std::fs::File> {
            tempfile::tempfile_in(&self.path)
        }
        pub fn open(path: &Path) -> io::Result<Self> {
            let path = std::path::absolute(path)?;
            let mut ancestors: Vec<_> = path.ancestors().collect();
            ancestors.reverse();
            let mut handles = Vec::new();
            for ancestor in ancestors {
                handles.push(pin(ancestor)?);
            }
            Ok(Self {
                path,
                _handles: handles,
            })
        }
        pub fn parent(&self, parts: &[String]) -> io::Result<Parent> {
            let mut path = self.path.clone();
            let mut handles = Vec::new();
            for part in parts {
                path.push(part);
                match std::fs::create_dir(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e),
                }
                handles.push(pin(&path)?);
            }
            Ok(Parent {
                path,
                _handles: handles,
            })
        }
    }
}

#[cfg(unix)]
struct Temporary {
    parent: std::fs::File,
    file: std::fs::File,
    name: std::ffi::CString,
}
#[cfg(unix)]
impl Temporary {
    fn new(parent: std::fs::File) -> io::Result<Self> {
        use std::{
            io::Read,
            os::fd::{AsRawFd, FromRawFd},
        };
        let mut random = [0u8; 16];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
        let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let name = std::ffi::CString::new(format!(".arc-{suffix}"))
            .map_err(|_| invalid("temporary name"))?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            parent,
            file: unsafe { std::fs::File::from_raw_fd(fd) },
            name,
        })
    }
}
#[cfg(unix)]
impl Drop for Temporary {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        unsafe {
            libc::unlinkat(self.parent.as_raw_fd(), self.name.as_ptr(), 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn unsafe_names_are_rejected() {
        for name in ["../x", "/x", "C:x", "a\\b", "a//b", "a/./b", "CON", "a."] {
            assert!(validate_name(name.as_bytes()).is_err(), "{name}");
        }
    }
    #[cfg(any(unix, windows))]
    #[test]
    fn failed_verification_never_publishes() {
        let root = tempfile::tempdir().unwrap();
        let mut destination = Destination::open(root.path()).unwrap();
        assert!(
            destination
                .file(b"bad", |file| {
                    file.write_all(b"bad")?;
                    Err(invalid("checksum"))
                })
                .is_err()
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
    #[cfg(any(unix, windows))]
    #[test]
    fn scratch_file_is_readable_and_removed_on_close() {
        use std::io::{Read, Seek, SeekFrom};
        let root = tempfile::tempdir().unwrap();
        let destination = Destination::open(root.path()).unwrap();
        let mut file = destination.scratch_file().unwrap();
        file.write_all(b"provisional").unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"provisional");
        drop(file);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
    #[cfg(unix)]
    #[test]
    fn symlink_parent_cannot_redirect_output() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("link")).unwrap();
        let mut destination = Destination::open(root.path()).unwrap();
        assert!(destination.file(b"link/file", |_| Ok(0)).is_err());
        assert!(!outside.path().join("file").exists());
    }
    #[cfg(any(unix, windows))]
    #[test]
    fn verified_output_is_published_and_collision_rejected() {
        let root = tempfile::tempdir().unwrap();
        let mut destination = Destination::open(root.path()).unwrap();
        destination
            .file(b"nested/a", |file| {
                file.write_all(b"ok")?;
                Ok(2)
            })
            .unwrap();
        assert_eq!(std::fs::read(root.path().join("nested/a")).unwrap(), b"ok");
        assert!(destination.file(b"nested/A", |_| Ok(0)).is_err());
    }
    #[cfg(any(unix, windows))]
    #[test]
    fn existing_file_is_preserved() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a"), b"original").unwrap();
        let mut destination = Destination::open(root.path()).unwrap();
        assert!(
            destination
                .file(b"a", |file| {
                    file.write_all(b"new")?;
                    Ok(3)
                })
                .is_err()
        );
        assert_eq!(std::fs::read(root.path().join("a")).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
    #[cfg(any(unix, windows))]
    #[test]
    fn cancellation_before_verification_cleans_temporary() {
        let root = tempfile::tempdir().unwrap();
        let mut destination = Destination::open(root.path()).unwrap();
        let cancellation = CancellationToken::default();
        assert!(
            destination
                .file_cancellable(b"a", &cancellation, |sink| {
                    sink.write_all(b"partial")?;
                    cancellation.cancel();
                    Ok(7)
                })
                .is_err()
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
    #[cfg(windows)]
    #[test]
    fn reparse_parent_and_root_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link = root.path().join("link");
        std::os::windows::fs::symlink_dir(outside.path(), &link).unwrap();
        assert!(Destination::open(&link).is_err());
        let mut destination = Destination::open(root.path()).unwrap();
        assert!(
            destination
                .file(b"link/file", |file| {
                    file.write_all(b"escape")?;
                    Ok(6)
                })
                .is_err()
        );
        assert!(!outside.path().join("file").exists());
    }
    #[cfg(windows)]
    #[test]
    fn retained_root_and_staged_parent_cannot_be_renamed() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let mut destination = Destination::open(&root).unwrap();
        let staged = destination.stage_file(b"nested/file").unwrap();
        assert!(std::fs::rename(&root, parent.path().join("moved")).is_err());
        assert!(std::fs::rename(root.join("nested"), root.join("other")).is_err());
        drop(staged);
        drop(destination);
        assert_eq!(std::fs::read_dir(root.join("nested")).unwrap().count(), 0);
    }
}
