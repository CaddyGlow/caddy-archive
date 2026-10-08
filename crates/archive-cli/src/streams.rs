use super::*;

pub(crate) fn stdin_operation(
    cli: &Cli,
    limits: Limits,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let interpretation = match &cli.command {
        Command::List { format, .. }
        | Command::Test { format, .. }
        | Command::Extract { format, .. } => format.as_deref(),
        _ => None,
    };
    if interpretation.is_some_and(|format| format != "tar") {
        return Err(archive_core::Error::Unsupported(
            "stdin supports TAR; indexed formats require seekable input".into(),
        )
        .into());
    }
    if cli.password_file.is_some() {
        return Err(archive_core::Error::Unsupported("TAR password encryption".into()).into());
    }
    let input = io::stdin();
    use io::Read;
    let mut source = CancellableSource(input.lock());
    let mut prefix = Vec::new();
    if interpretation.is_none() {
        (&mut source).take(512).read_to_end(&mut prefix)?;
        if archive_core::probe(&prefix).ok() != Some(Format::Tar) {
            return Err(archive_core::Error::Unsupported(
                "stdin format is unrecognized or requires seekable input; use --format tar for a headerless TAR".into(),
            ).into());
        }
    }
    let mut archive = archive_core::sequential_tar::SequentialTar::new(
        io::Cursor::new(prefix).chain(source),
        limits,
    );
    let mut destination = if let Command::Extract {
        output, threads, ..
    } = &cli.command
    {
        if *threads == 0 {
            return Err("worker count must be positive".into());
        }
        std::fs::create_dir_all(output)?;
        Some(archive_fs::Destination::open(output)?)
    } else {
        None
    };
    let mut entries = Vec::new();
    let mut bytes = 0u64;
    let mut pending = destination
        .as_ref()
        .map(staging::BatchSpool::new)
        .transpose()?;
    while let Some(entry) = archive.next_entry()? {
        check_cancelled()?;
        if cli.verbose && !matches!(cli.command, Command::List { .. }) {
            eprintln!("{}", entry.name);
        }
        let report = if let Some(destination) = &mut destination {
            match entry.kind {
                EntryKind::Directory => {
                    destination.directory(&entry.raw_name)?;
                    pending
                        .as_mut()
                        .ok_or("missing provisional spool")?
                        .directory_metadata(&entry.raw_name, archive.current_metadata()?)?;
                    archive.skip_current()?
                }
                EntryKind::File => {
                    let spool = pending.as_mut().ok_or("missing provisional spool")?;
                    spool.stage(entry.id.0, &entry.raw_name, entry.size)?;
                    spool.metadata(entry.id.0, archive.current_metadata()?)?;
                    archive.copy_current(&mut spool.writer(entry.id.0))?
                }
                _ => {
                    return Err(
                        archive_core::Error::Unsupported("links and special files".into()).into(),
                    );
                }
            }
        } else {
            archive.skip_current()?
        };
        bytes = bytes
            .checked_add(report.bytes)
            .ok_or("decoded bytes overflow")?;
        if !cli.json && matches!(cli.command, Command::List { .. }) {
            println!("{:>12} {}", entry.size, entry.name);
        }
        entries.push(entry);
    }
    check_cancelled()?;
    if let (Some(spool), Some(destination)) = (pending, &mut destination) {
        spool.publish(destination, || {})?;
    }
    let operation = match cli.command {
        Command::List { .. } => "list",
        Command::Test { .. } => "test",
        _ => "extract",
    };
    Ok(
        serde_json::json!({"schema_version":1,"ok":true,"operation":operation,"format":"tar","verified":true,"bytes":bytes,"entry_count":entries.len(),"entries":if operation=="list"{Some(entries)}else{None}}),
    )
}
