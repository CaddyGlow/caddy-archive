//! Adjacent provisional archive publication with retained Unix directory handles.
//!
//! Transactions take an advisory exclusive lock on the parent directory. All
//! writers must cooperate with that lock and refrain from changing open sources.
//! The final identity check followed by rename is not a compare-and-swap against
//! uncooperative writers. Parent traversal rejects links and `..`; retained
//! directory handles keep operations in the admitted directory if ancestors move.
//! Hardlinked sources are rejected. Windows replacement is unavailable pending
//! native sharing, reparse-point, and ReplaceFile validation.

use std::{fs::File, io, path::Path};

/// Successful publication, including a possible failure to sync its directory.
/// A directory sync error means publication occurred but crash durability could
/// not be established; callers must not retry as if the original survived.
#[derive(Debug)]
pub struct UpdatePublication {
    /// Failure to sync the parent directory after successful publication.
    pub directory_sync_error: Option<io::Error>,
}

/// A provisional archive removed on drop until explicitly published.
pub struct UpdateTransaction {
    #[cfg(unix)]
    temporary: super::Temporary,
    #[cfg(unix)]
    target: std::ffi::CString,
    #[cfg(unix)]
    original: Option<(File, Identity)>,
}

#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
    length: u64,
    modified: (i64, i64),
    changed: (i64, i64),
    links: u64,
}

#[cfg(unix)]
impl Identity {
    fn read(file: &File) -> io::Result<Self> {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err(super::invalid(
                "update source must be a regular file with one link",
            ));
        }
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            length: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
            links: metadata.nlink(),
        })
    }
}

impl UpdateTransaction {
    /// Stage replacement of an existing regular archive, retaining its handle
    /// and immutable initial identity until publication or drop.
    pub fn replace(path: &Path) -> io::Result<Self> {
        Self::begin(path, true)
    }

    /// Stage a new artifact; publication never overwrites an existing name.
    pub fn create(path: &Path) -> io::Result<Self> {
        Self::begin(path, false)
    }

    fn begin(path: &Path, replace: bool) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let (parent, target) = admit_parent(path)?;
            // Directory locks coordinate even when replacement changes inodes.
            if unsafe { libc::flock(parent.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err(io::Error::last_os_error());
            }
            let original = if replace {
                let file = open_source(&parent, &target)?;
                let identity = Identity::read(&file)?;
                Some((file, identity))
            } else {
                // Early rejection is helpful; linkat still enforces no-clobber
                // at publication against writers that ignore the advisory lock.
                match open_source(&parent, &target) {
                    Ok(_) => return Err(io::ErrorKind::AlreadyExists.into()),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
                None
            };
            let temporary = super::Temporary::new(parent)?;
            Ok(Self {
                temporary,
                target,
                original,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = (path, replace);
            Err(io::ErrorKind::Unsupported.into())
        }
    }

    /// Retained original reader, absent for new-output transactions.
    pub fn source(&self) -> Option<&File> {
        #[cfg(unix)]
        {
            self.original.as_ref().map(|(file, _)| file)
        }
        #[cfg(not(unix))]
        {
            None
        }
    }

    /// Seekable provisional output. Complete and verify the archive before
    /// calling `publish`; this transaction does not validate archive structure.
    pub fn file_mut(&mut self) -> &mut File {
        #[cfg(unix)]
        {
            &mut self.temporary.file
        }
        #[cfg(not(unix))]
        {
            unreachable!("unsupported platform cannot create an update transaction")
        }
    }

    /// Sync and publish after caller verification. The cancellation/check hook
    /// runs before output sync and again immediately before the identity check
    /// and publication. Every error returned preserves the admitted target.
    pub fn publish(
        self,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<UpdatePublication> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            check()?;
            if let Some((original, _)) = &self.original {
                self.temporary
                    .file
                    .set_permissions(original.metadata()?.permissions())?;
            }
            self.temporary.file.sync_all()?;
            check()?;
            if let Some((original, expected)) = &self.original {
                let current = open_source(&self.temporary.parent, &self.target)?;
                if Identity::read(original)? != *expected || Identity::read(&current)? != *expected
                {
                    return Err(io::Error::other("archive changed during update"));
                }
            }
            let fd = self.temporary.parent.as_raw_fd();
            let result = unsafe {
                if self.original.is_some() {
                    libc::renameat(fd, self.temporary.name.as_ptr(), fd, self.target.as_ptr())
                } else {
                    libc::linkat(
                        fd,
                        self.temporary.name.as_ptr(),
                        fd,
                        self.target.as_ptr(),
                        0,
                    )
                }
            };
            if result != 0 {
                return Err(io::Error::last_os_error());
            }
            // New-output link publication leaves the provisional link until drop.
            // Remove it before directory sync so the successful durable state has
            // only the final name. Drop's second unlink is harmless.
            unsafe {
                libc::unlinkat(fd, self.temporary.name.as_ptr(), 0);
            }
            Ok(UpdatePublication {
                directory_sync_error: self.temporary.parent.sync_all().err(),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = &mut check;
            Err(io::ErrorKind::Unsupported.into())
        }
    }
}

#[cfg(unix)]
fn admit_parent(path: &Path) -> io::Result<(File, std::ffi::CString)> {
    use std::{
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::{ffi::OsStrExt, fs::OpenOptionsExt},
        },
        path::Component,
    };
    let name = path
        .file_name()
        .ok_or_else(|| super::invalid("missing archive filename"))?;
    let target =
        std::ffi::CString::new(name.as_bytes()).map_err(|_| super::invalid("NUL filename"))?;
    let mut parent = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(if path.is_absolute() {
            Path::new("/")
        } else {
            Path::new(".")
        })?;
    for component in path.parent().unwrap_or(Path::new(".")).components() {
        let component = match component {
            Component::RootDir | Component::CurDir => continue,
            Component::Normal(name) => name,
            _ => return Err(super::invalid("unsafe archive parent path")),
        };
        let name =
            std::ffi::CString::new(component.as_bytes()).map_err(|_| super::invalid("NUL path"))?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        parent = unsafe { File::from_raw_fd(fd) };
    }
    Ok((parent, target))
}

#[cfg(unix)]
fn open_source(parent: &File, target: &std::ffi::CStr) -> io::Result<File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            target.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn replacement_publishes_finished_output_and_retains_original_reader() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("archive");
        std::fs::write(&path, b"original").unwrap();
        let mut transaction = UpdateTransaction::replace(&path).unwrap();
        let mut bytes = Vec::new();
        transaction
            .source()
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, b"original");
        transaction.file_mut().write_all(b"replacement").unwrap();
        assert!(
            transaction
                .publish(|| Ok(()))
                .unwrap()
                .directory_sync_error
                .is_none()
        );
        assert_eq!(std::fs::read(path).unwrap(), b"replacement");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn cancellation_at_each_publication_checkpoint_preserves_original() {
        for cancel_at in [1, 2] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("archive");
            std::fs::write(&path, b"original").unwrap();
            let mut transaction = UpdateTransaction::replace(&path).unwrap();
            transaction.file_mut().write_all(b"replacement").unwrap();
            let mut calls = 0;
            assert!(
                transaction
                    .publish(|| {
                        calls += 1;
                        if calls == cancel_at {
                            Err(io::Error::other("cancelled"))
                        } else {
                            Ok(())
                        }
                    })
                    .is_err()
            );
            assert_eq!(std::fs::read(path).unwrap(), b"original");
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn concurrent_replacement_is_detected_and_preserved() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("archive");
        std::fs::write(&path, b"original").unwrap();
        let transaction = UpdateTransaction::replace(&path).unwrap();
        let concurrent = root.path().join("concurrent");
        std::fs::write(&concurrent, b"concurrent").unwrap();
        std::fs::rename(concurrent, &path).unwrap();
        assert!(transaction.publish(|| Ok(())).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"concurrent");
    }

    #[test]
    fn source_in_place_change_is_detected() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("archive");
        std::fs::write(&path, b"original").unwrap();
        let transaction = UpdateTransaction::replace(&path).unwrap();
        std::fs::write(&path, b"changed source").unwrap();
        assert!(transaction.publish(|| Ok(())).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"changed source");
    }

    #[test]
    fn new_output_never_clobbers_a_concurrent_writer() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("archive");
        let transaction = UpdateTransaction::create(&path).unwrap();
        std::fs::write(&path, b"concurrent").unwrap();
        assert_eq!(
            transaction.publish(|| Ok(())).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read(path).unwrap(), b"concurrent");
    }

    #[test]
    fn links_and_parent_traversal_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("archive");
        std::fs::write(&path, b"original").unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(UpdateTransaction::replace(&link).is_err());
        std::fs::remove_file(&link).unwrap();
        std::fs::hard_link(&path, &link).unwrap();
        assert!(UpdateTransaction::replace(&path).is_err());
        let parent_link = root.path().join("parent-link");
        std::os::unix::fs::symlink(root.path(), &parent_link).unwrap();
        assert!(UpdateTransaction::create(&parent_link.join("new")).is_err());
        assert!(UpdateTransaction::create(&root.path().join("../new")).is_err());
    }

    #[test]
    fn retained_parent_survives_directory_rename() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("original");
        std::fs::create_dir(&original).unwrap();
        let mut transaction = UpdateTransaction::create(&original.join("archive")).unwrap();
        transaction.file_mut().write_all(b"new").unwrap();
        let moved = root.path().join("moved");
        std::fs::rename(&original, &moved).unwrap();
        transaction.publish(|| Ok(())).unwrap();
        assert_eq!(std::fs::read(moved.join("archive")).unwrap(), b"new");
    }

    #[test]
    fn directory_lock_rejects_another_cooperating_transaction() {
        let root = tempfile::tempdir().unwrap();
        let first = UpdateTransaction::create(&root.path().join("first")).unwrap();
        assert!(UpdateTransaction::create(&root.path().join("second")).is_err());
        drop(first);
        assert!(UpdateTransaction::create(&root.path().join("second")).is_ok());
    }

    #[test]
    fn dropping_unfinished_output_preserves_original_and_removes_provisional() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("archive");
        std::fs::write(&path, b"original").unwrap();
        let mut transaction = UpdateTransaction::replace(&path).unwrap();
        transaction
            .file_mut()
            .write_all(b"partial archive")
            .unwrap();
        drop(transaction);
        assert_eq!(std::fs::read(path).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
