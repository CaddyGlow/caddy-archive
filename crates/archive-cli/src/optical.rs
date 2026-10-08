use super::*;
pub(crate) fn operation(
    cli: &Cli,
    path: &Path,
    limits: Limits,
) -> Result<Option<serde_json::Value>, Box<dyn std::error::Error>> {
    if matches!(cli.view, Some(OpticalView::Udf)) {
        let archive = archive_core::udf::UdfArchive::open_reader(
            CancellableSource(std::fs::File::open(path)?),
            limits,
        )?;
        let result = match &cli.command {
            Command::List { .. } => {
                if !cli.json {
                    for entry in archive.entries() {
                        println!("{:>12} {}", entry.size, entry.name);
                    }
                }
                serde_json::json!({"schema_version":1,"ok":true,"operation":"list","format":"udf","entries":archive.entries()})
            }
            Command::Test { .. } => {
                let report = archive.test()?;
                serde_json::json!({"schema_version":1,"ok":true,"operation":"test","verified":report.verified,"entries":report.entries,"bytes":report.bytes})
            }
            Command::Extract {
                output, threads, ..
            } => {
                if *threads == 0 {
                    return Err("worker count must be positive".into());
                }
                preflight(archive.entries().iter().map(|entry| entry.name.as_bytes()))?;
                if archive
                    .entries()
                    .iter()
                    .any(|e| !matches!(e.kind, EntryKind::File | EntryKind::Directory))
                {
                    return Err("UDF links and special files are unsupported".into());
                }
                std::fs::create_dir_all(output)?;
                let mut destination = archive_fs::Destination::open(output)?;
                for entry in archive.entries() {
                    check_cancelled()?;
                    match entry.kind {
                        EntryKind::Directory => destination.directory(entry.name.as_bytes())?,
                        EntryKind::File => {
                            destination.file(entry.name.as_bytes(), |sink| {
                                let report = archive
                                    .extract(entry.id, &mut CancellableSink(sink))
                                    .map_err(io::Error::other)?;
                                check_cancelled()?;
                                Ok(report.bytes)
                            })?;
                        }
                        _ => return Err("UDF special file".into()),
                    }
                }
                serde_json::json!({"schema_version":1,"ok":true,"operation":"extract","verified":true,"workers_used":1})
            }
            _ => return Ok(None),
        };
        return Ok(Some(result));
    }
    Ok(None)
}
