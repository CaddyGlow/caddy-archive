//! Native ZIP name edits through provisional filesystem transactions.
use archive_core::{
    Archive, Limits,
    zip_edit::{self, EditOperation},
};
use archive_fs::update::UpdateTransaction;
use std::{
    error::Error,
    fs::File,
    io::{self, Seek},
    path::Path,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub(super) fn run(cli: &super::Cli, limits: Limits) -> Result<serde_json::Value> {
    let (archive, output, dry_run, verify, operation, operations) = match &cli.command {
        super::Command::Delete {
            archive,
            names,
            output,
            dry_run,
            verify,
        } => (
            archive,
            output,
            *dry_run,
            *verify,
            "delete",
            names
                .iter()
                .map(|name| EditOperation::Delete { name: name.clone() })
                .collect::<Vec<_>>(),
        ),
        super::Command::Rename {
            archive,
            pairs,
            output,
            dry_run,
            verify,
        } => {
            if pairs.is_empty() || pairs.len() % 2 != 0 {
                return Err("rename requires OLD NEW pairs".into());
            }
            (
                archive,
                output,
                *dry_run,
                *verify,
                "rename",
                pairs
                    .chunks_exact(2)
                    .map(|pair| EditOperation::Rename {
                        from: pair[0].clone(),
                        to: pair[1].clone(),
                    })
                    .collect(),
            )
        }
        _ => return Err("invalid ZIP edit command".into()),
    };
    if cli.password_file.is_some() && !verify {
        return Err(archive_core::Error::Unsupported(
            "ZIP editing only uses --password-file with --verify".into(),
        )
        .into());
    }
    if !cli.media.is_empty()
        || cli.bundle_entry.is_some()
        || cli.image.is_some()
        || cli.image_name.is_some()
        || cli.view.is_some()
    {
        return Err(archive_core::Error::Unsupported(
            "package, image, and optical options do not apply to ZIP editing".into(),
        )
        .into());
    }
    if operations.is_empty() {
        return Err("ZIP edit requires at least one name".into());
    }
    reject_package_suffix(archive)?;
    if let Some(output) = output {
        reject_package_suffix(output)?;
    }
    super::check_cancelled()?;
    let mut source = open_source(archive)?;
    let expected = SourceIdentity::read(&source)?;
    if let Some(output) = output {
        reject_alias(archive, &source, output)?;
        match std::fs::symlink_metadata(output) {
            Ok(_) => return Err(io::Error::from(io::ErrorKind::AlreadyExists).into()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let plan = zip_edit::plan(&mut source, &operations, limits)?;
    if plan
        .entries()
        .iter()
        .any(|entry| package_marker(&entry.original_name))
    {
        return Err(archive_core::Error::Unsupported(
            "ZIP package/signature editing requires an explicit package policy".into(),
        )
        .into());
    }
    source_unchanged(archive, &source, &expected)?;
    if dry_run {
        return Ok(
            serde_json::json!({"schema_version":1,"ok":true,"operation":operation,
            "format":"zip","dry_run":true,"published":false,
            "entries":plan.entries(),"payloads_verified":false}),
        );
    }
    let password = if verify {
        super::read_password(cli)?
    } else {
        None
    };
    let mut transaction = if let Some(output) = output {
        UpdateTransaction::create(output)?
    } else {
        let transaction = UpdateTransaction::replace(archive)?;
        let retained = transaction
            .source()
            .ok_or("replacement source handle unavailable")?;
        if SourceIdentity::read(retained)? != expected {
            return Err("archive changed after planning".into());
        }
        source = retained.try_clone()?;
        transaction
    };
    source_unchanged(archive, &source, &expected)?;
    let mut report = zip_edit::execute(&mut source, transaction.file_mut(), &plan, || {
        super::check_cancelled().is_err()
    })?;
    // Validate emitted structure even when packed payloads were intentionally
    // copied without authentication or decompression.
    let mut provisional = transaction.file_mut().try_clone()?;
    provisional.rewind()?;
    zip_edit::plan(&mut provisional, &[], limits)?;
    if verify {
        provisional.rewind()?;
        let mut archive = if let Some(password) = &password {
            Archive::open_with_password(provisional, limits, &password.0)?
        } else {
            Archive::open(provisional, limits)?
        };
        let verification = archive.test_cancellable(|| super::check_cancelled().is_err())?;
        if !verification.verified {
            return Err(archive_core::Error::Unsupported(
                "full ZIP payload verification unavailable".into(),
            )
            .into());
        }
        report.payloads_verified = true;
    }
    let publication = transaction.publish(|| {
        super::check_cancelled()?;
        source_unchanged(archive, &source, &expected)
    })?;
    Ok(
        serde_json::json!({"schema_version":1,"ok":true,"operation":operation,
        "format":"zip","dry_run":false,"published":true,
        "output":output.as_ref().unwrap_or(archive),
        "retained_entries":report.retained_entries,"removed_entries":report.removed_entries,
        "renamed_entries":report.renamed_entries,"packed_bytes_copied":report.packed_bytes_copied,
        "structurally_validated":true,"payloads_verified":report.payloads_verified,
        "directory_synced":publication.directory_sync_error.is_none(),
        "durability_error":publication.directory_sync_error.map(|error| error.to_string())}),
    )
}

pub(super) fn reject_package_suffix(path: &Path) -> Result<()> {
    if path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "msi" | "msp" | "appx" | "msix" | "appxbundle" | "msixbundle" | "cab" | "nupkg"
            )
        })
    {
        return Err(archive_core::Error::Unsupported(
            "package editing requires an explicit package/signature policy".into(),
        )
        .into());
    }
    Ok(())
}

pub(super) fn package_marker(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "appxmanifest.xml"
            | "appxblockmap.xml"
            | "appxsignature.p7x"
            | "appxmetadata/appxbundlemanifest.xml"
            | ".signature.p7s"
    ) || (name.starts_with("meta-inf/")
        && (name.ends_with(".sf")
            || name.ends_with(".rsa")
            || name.ends_with(".dsa")
            || name.ends_with(".ec")))
}

pub(super) fn open_source(path: &Path) -> io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() || std::fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(io::Error::other(
            "ZIP source must be a regular file without links",
        ));
    }
    Ok(file)
}

#[derive(PartialEq, Eq)]
pub(super) struct SourceIdentity {
    length: u64,
    modified: std::time::SystemTime,
    #[cfg(unix)]
    native: (u64, u64, i64, i64, u64),
}

impl SourceIdentity {
    pub(super) fn read(file: &File) -> io::Result<Self> {
        Self::metadata(&file.metadata()?)
    }
    fn metadata(metadata: &std::fs::Metadata) -> io::Result<Self> {
        if !metadata.is_file() {
            return Err(io::Error::other("ZIP source changed type"));
        }
        Ok(Self {
            length: metadata.len(),
            modified: metadata.modified()?,
            #[cfg(unix)]
            native: {
                use std::os::unix::fs::MetadataExt;
                (
                    metadata.dev(),
                    metadata.ino(),
                    metadata.ctime(),
                    metadata.ctime_nsec(),
                    metadata.nlink(),
                )
            },
        })
    }
}

pub(super) fn source_unchanged(
    path: &Path,
    source: &File,
    expected: &SourceIdentity,
) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink()
        || SourceIdentity::read(source)? != *expected
        || SourceIdentity::metadata(&metadata)? != *expected
    {
        return Err(io::Error::other("archive changed during ZIP edit"));
    }
    Ok(())
}

pub(super) fn reject_alias(input: &Path, source: &File, output: &Path) -> Result<()> {
    if let Ok(metadata) = std::fs::metadata(output) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let original = source.metadata()?;
            if metadata.dev() == original.dev() && metadata.ino() == original.ino() {
                return Err("ZIP output aliases the source archive".into());
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (metadata, source);
        }
    }
    if output.exists() && std::fs::canonicalize(input)? == std::fs::canonicalize(output)? {
        return Err("ZIP output aliases the source archive".into());
    }
    Ok(())
}
