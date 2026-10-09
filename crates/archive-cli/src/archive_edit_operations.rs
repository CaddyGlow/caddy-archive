//! Timestamp and credential edits through bounded core transforms and publication.
use archive_core::{Archive, Entry, EntryKind, Format, Limits, zip_edit};
use archive_fs::update::UpdateTransaction;
use std::{
    error::Error,
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub(super) fn run(cli: &super::Cli, limits: Limits) -> Result<serde_json::Value> {
    let super::Command::Edit {
        archive,
        names,
        modified_unix_seconds,
        encrypt,
        decrypt,
        new_password_file,
        encrypt_headers,
        output,
        dry_run,
        verify,
    } = &cli.command
    else {
        return Err("invalid archive edit command".into());
    };
    if modified_unix_seconds.is_none() && !encrypt && !decrypt {
        return Err("edit requires a timestamp or encryption change".into());
    }
    if *encrypt && *decrypt {
        return Err("encrypt and decrypt cannot be combined".into());
    }
    if *encrypt != new_password_file.is_some() {
        return Err("encrypt requires --new-password-file; new passwords require --encrypt".into());
    }
    if *encrypt_headers && (!encrypt || !names.is_empty()) {
        return Err("header encryption requires archive-wide --encrypt without --name".into());
    }
    if !cli.media.is_empty()
        || cli.bundle_entry.is_some()
        || cli.image.is_some()
        || cli.image_name.is_some()
        || cli.view.is_some()
    {
        return Err(archive_core::Error::Unsupported(
            "package, image, and optical options do not apply to archive editing".into(),
        )
        .into());
    }
    super::zip_operations::reject_package_suffix(archive)?;
    if let Some(output) = output {
        super::zip_operations::reject_package_suffix(output)?;
    }
    super::check_cancelled()?;
    let old_password = super::read_password(cli)?;
    let new_password = new_password_file
        .as_deref()
        .map(read_password)
        .transpose()?;
    let old = old_password.as_ref().map(|password| password.0.as_slice());
    let new = new_password.as_ref().map(|password| password.0.as_slice());
    let mut source = super::zip_operations::open_source(archive)?;
    let expected = super::zip_operations::SourceIdentity::read(&source)?;
    if let Some(output) = output {
        super::zip_operations::reject_alias(archive, &source, output)?;
        match std::fs::symlink_metadata(output) {
            Ok(_) => return Err(io::Error::from(io::ErrorKind::AlreadyExists).into()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let indexed = open_archive(source.try_clone()?, old, limits)?;
    let format = indexed.format();
    if !matches!(format, Format::Zip | Format::SevenZip) {
        return Err(archive_core::Error::Unsupported(
            "timestamp/password editing supports ZIP and 7z only".into(),
        )
        .into());
    }
    if format == Format::Zip && *encrypt_headers {
        return Err(archive_core::Error::Unsupported(
            "ZIP cannot hide filenames with header encryption".into(),
        )
        .into());
    }
    let entries = indexed.entries().to_vec();
    if entries
        .iter()
        .any(|entry| super::zip_operations::package_marker(&entry.name))
    {
        return Err(archive_core::Error::Unsupported(
            "package/signature editing requires an explicit package policy".into(),
        )
        .into());
    }
    let selected = select_entries(&entries, names)?;
    let header_only_change =
        format == Format::SevenZip && names.is_empty() && (*encrypt_headers || *decrypt);
    if (*encrypt || *decrypt)
        && !selected.iter().any(|entry| entry.kind == EntryKind::File)
        && !header_only_change
    {
        return Err("encryption selection contains no file payloads".into());
    }
    let decisions: Vec<_> = entries.iter().map(|entry| {
        let selected = selected.iter().any(|chosen| chosen.id == entry.id);
        serde_json::json!({"name":entry.name,"selected":selected,
            "requested_modified_unix_seconds":if selected { *modified_unix_seconds } else { None },
            "was_encrypted":entry.encrypted,
            "requested_encrypted":if selected && entry.kind == EntryKind::File && (*encrypt || *decrypt) { Some(*encrypt) } else { None }})
    }).collect();
    let zip_operations = if format == Format::Zip {
        selected
            .iter()
            .flat_map(|entry| {
                let mut operations = Vec::new();
                if let Some(seconds) = modified_unix_seconds {
                    operations.push(zip_edit::EditOperation::SetModified {
                        name: entry.name.clone(),
                        modified_unix_seconds: *seconds,
                    });
                }
                if entry.kind == EntryKind::File && (*encrypt || *decrypt) {
                    operations.push(zip_edit::EditOperation::SetEncryption {
                        name: entry.name.clone(),
                        encryption: if *encrypt {
                            zip_edit::EntryEncryption::Aes256
                        } else {
                            zip_edit::EntryEncryption::None
                        },
                    });
                }
                operations
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let sevenz_operations = if format == Format::SevenZip {
        let mut operations = Vec::new();
        let targets = if names.is_empty() {
            vec![None]
        } else {
            selected
                .iter()
                .map(|entry| Some(entry.name.clone()))
                .collect()
        };
        for name in targets {
            if let Some(seconds) = modified_unix_seconds {
                operations.push(archive_core::sevenz_edit::EditOperation::SetModified {
                    name: name.clone(),
                    modified_unix_seconds: *seconds,
                });
            }
            if (*encrypt || *decrypt)
                && name.as_ref().is_none_or(|name| {
                    selected
                        .iter()
                        .any(|entry| entry.name == *name && entry.kind == EntryKind::File)
                })
            {
                operations.push(archive_core::sevenz_edit::EditOperation::SetEncryption {
                    name,
                    mode: if *encrypt {
                        archive_core::sevenz_edit::EncryptionMode::Encrypt
                    } else {
                        archive_core::sevenz_edit::EncryptionMode::Decrypt
                    },
                });
            }
        }
        operations
    } else {
        Vec::new()
    };
    source.rewind()?;
    let zip_plan = if format == Format::Zip {
        Some(zip_edit::plan(&mut source, &zip_operations, limits)?)
    } else {
        None
    };
    let header_policy = if *encrypt_headers {
        Some(true)
    } else if *decrypt && names.is_empty() {
        Some(false)
    } else {
        None
    };
    // Authenticate and validate effective transformations before creating any
    // provisional filesystem artifact. 7z currently uses a seekable discard
    // sink for preflight; transformed-group work repeats during execution.
    if let Some(plan) = &zip_plan {
        let mut randomness = super::NativeRandom;
        let options = zip_edit::ZipEditOptions {
            password: old,
            new_password: new,
            randomness: Some(&mut randomness),
        };
        zip_edit::validate_credentials(&mut source, plan, &options, || {
            super::check_cancelled().is_err()
        })?;
    } else {
        let mut randomness = super::NativeRandom;
        let mut discard = DiscardWriter::default();
        archive_core::sevenz_edit::edit(
            &mut super::CancellableSource(&mut source),
            &mut super::CancellableSink(&mut discard),
            &sevenz_operations,
            archive_core::sevenz_edit::EditOptions {
                old_password: old,
                new_password: new,
                randomness: Some(&mut randomness),
                encrypt_headers: header_policy,
            },
            limits,
        )?;
    }
    super::zip_operations::source_unchanged(archive, &source, &expected)?;
    if *dry_run {
        return Ok(
            serde_json::json!({"schema_version":1,"ok":true,"operation":"edit",
            "format":format_name(format),"dry_run":true,"published":false,"entries":decisions,
            "credentials_validated":true,"payloads_verified":false,
            "requested_header_encryption":header_policy}),
        );
    }
    let mut transaction = if let Some(output) = output {
        UpdateTransaction::create(output)?
    } else {
        let transaction = UpdateTransaction::replace(archive)?;
        let retained = transaction
            .source()
            .ok_or("replacement source handle unavailable")?;
        if super::zip_operations::SourceIdentity::read(retained)? != expected {
            return Err("archive changed after planning".into());
        }
        source = retained.try_clone()?;
        transaction
    };
    super::zip_operations::source_unchanged(archive, &source, &expected)?;
    let mut randomness = super::NativeRandom;
    let details = if let Some(plan) = &zip_plan {
        let report = zip_edit::execute_with_options(
            &mut source,
            transaction.file_mut(),
            plan,
            zip_edit::ZipEditOptions {
                password: old,
                new_password: new,
                randomness: Some(&mut randomness),
            },
            || super::check_cancelled().is_err(),
        )?;
        serde_json::to_value(report)?
    } else {
        let report = archive_core::sevenz_edit::edit(
            &mut super::CancellableSource(&mut source),
            &mut super::CancellableSink(transaction.file_mut()),
            &sevenz_operations,
            archive_core::sevenz_edit::EditOptions {
                old_password: old,
                new_password: new,
                randomness: Some(&mut randomness),
                encrypt_headers: header_policy,
            },
            limits,
        )?;
        serde_json::to_value(report)?
    };
    let mut provisional = transaction.file_mut().try_clone()?;
    if format == Format::Zip {
        zip_edit::plan(&mut provisional, &[], limits)?;
    }
    let mut fully_verified = false;
    if *verify {
        fully_verified = verify_all(
            transaction.file_mut(),
            format,
            &entries,
            &selected,
            *encrypt,
            *decrypt,
            old,
            new,
            limits,
        )?;
    }
    let publication = transaction.publish(|| {
        super::check_cancelled()?;
        super::zip_operations::source_unchanged(archive, &source, &expected)
    })?;
    Ok(
        serde_json::json!({"schema_version":1,"ok":true,"operation":"edit","format":format_name(format),
        "dry_run":false,"published":true,"entries":decisions,"details":details,
        "structurally_validated":true,"payloads_verified":fully_verified,
        "transformed_payloads_verified":true,
        "directory_synced":publication.directory_sync_error.is_none(),
        "durability_error":publication.directory_sync_error.map(|error| error.to_string())}),
    )
}

fn format_name(format: Format) -> &'static str {
    if format == Format::Zip { "zip" } else { "7z" }
}

fn open_archive(
    file: File,
    password: Option<&[u8]>,
    limits: Limits,
) -> archive_core::Result<Archive<File>> {
    let mut file = file;
    file.rewind()?;
    if let Some(password) = password {
        Archive::open_with_password(file, limits, password)
    } else {
        Archive::open(file, limits)
    }
}

fn read_password(path: &Path) -> Result<super::Password> {
    let mut password = super::Password(Vec::with_capacity(16_386));
    File::open(path)?
        .take(16_385)
        .read_to_end(&mut password.0)?;
    if password.0.len() > 16_384 {
        return Err("password source exceeds length limit".into());
    }
    if password.0.ends_with(b"\n") {
        password.0.pop();
        if password.0.ends_with(b"\r") {
            password.0.pop();
        }
    }
    Ok(password)
}

fn select_entries<'a>(entries: &'a [Entry], names: &[String]) -> Result<Vec<&'a Entry>> {
    if names.is_empty() {
        return Ok(entries.iter().collect());
    }
    if names.iter().any(|name| {
        name.is_empty()
            || !entries.iter().any(|entry| {
                entry.name == *name || name.ends_with('/') && entry.name.starts_with(name)
            })
    }) {
        return Err("selected archive name matched no entries".into());
    }
    Ok(entries
        .iter()
        .filter(|entry| {
            names.iter().any(|name| {
                entry.name == *name || name.ends_with('/') && entry.name.starts_with(name)
            })
        })
        .collect())
}

#[expect(
    clippy::too_many_arguments,
    reason = "verification needs source and requested credential groups"
)]
fn verify_all(
    output: &mut File,
    format: Format,
    entries: &[Entry],
    selected: &[&Entry],
    encrypt: bool,
    decrypt: bool,
    old: Option<&[u8]>,
    new: Option<&[u8]>,
    limits: Limits,
) -> Result<bool> {
    let hidden_headers = if format == Format::SevenZip {
        match open_archive(output.try_clone()?, None, limits) {
            Ok(_) => false,
            Err(archive_core::Error::PasswordRequired) => true,
            Err(error) => return Err(error.into()),
        }
    } else {
        false
    };
    let header_password = if encrypt { new.or(old) } else { old };
    // Mixed-password ZIP files are verified in groups rather than accidentally
    // applying a new selected-file password to retained encrypted members.
    let mut groups: Vec<(Option<&[u8]>, Vec<archive_core::EntryId>)> = Vec::new();
    for entry in entries {
        let chosen = selected.iter().any(|chosen| chosen.id == entry.id);
        let encrypted = if chosen && entry.kind == EntryKind::File && (encrypt || decrypt) {
            encrypt
        } else {
            entry.encrypted
        };
        let password = if encrypted {
            if chosen && encrypt { new } else { old }
        } else {
            None
        };
        if let Some((_, ids)) = groups
            .iter_mut()
            .find(|(existing, _)| *existing == password)
        {
            ids.push(entry.id);
        } else {
            groups.push((password, vec![entry.id]));
        }
    }
    if groups.is_empty() {
        groups.push((old.or(new), Vec::new()));
    }
    for (password, ids) in groups {
        let indexing_password = if hidden_headers {
            header_password
        } else {
            password
        };
        let mut archive = open_archive(output.try_clone()?, indexing_password, limits)?;
        let report = archive.extract_selected_cancellable(
            &ids,
            || super::check_cancelled().is_err(),
            &mut |_, _| Ok(()),
        )?;
        if !report.verified {
            return Err(archive_core::Error::Unsupported(
                "full payload verification unavailable".into(),
            )
            .into());
        }
    }
    Ok(true)
}

#[derive(Default)]
struct DiscardWriter {
    position: u64,
    length: u64,
}
impl Write for DiscardWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.position = self
            .position
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| io::Error::other("discard output offset overflow"))?;
        self.length = self.length.max(self.position);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for DiscardWriter {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.position = match from {
            SeekFrom::Start(position) => position,
            SeekFrom::Current(offset) => self
                .position
                .checked_add_signed(offset)
                .ok_or_else(|| io::Error::other("invalid discard seek"))?,
            SeekFrom::End(offset) => self
                .length
                .checked_add_signed(offset)
                .ok_or_else(|| io::Error::other("invalid discard seek"))?,
        };
        Ok(self.position)
    }
}
