use super::*;

pub(crate) fn parse(name: &str) -> archive_core::Result<Format> {
    name.parse()
}

pub(crate) fn extension(path: &Path) -> Option<Format> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    for suffix in ["tar.gz", "tar.xz", "tar.bz2", "tar.br"] {
        if name.ends_with(&format!(".{suffix}")) {
            return parse(suffix).ok();
        }
    }
    parse(path.extension()?.to_str()?).ok()
}

pub(crate) fn creation(explicit: Option<&str>, output: &Path) -> archive_core::Result<Format> {
    match explicit {
        Some(name) => parse(name),
        None => extension(output).ok_or_else(|| archive_core::Error::Unsupported(
            "cannot infer creation format from output extension; specify --format (also required for stdout)".into()
        )),
    }
}

pub(crate) fn archive_stem(path: &Path) -> io::Result<String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "archive filename must be UTF-8",
            )
        })?;
    let lower = name.to_ascii_lowercase();
    let stem = if let Some(suffix) = [".tar.gz", ".tar.xz", ".tar.bz2", ".tar.br"]
        .into_iter()
        .find(|suffix| lower.ends_with(suffix))
    {
        &name[..name.len() - suffix.len()]
    } else {
        Path::new(name)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(name)
    };
    archive_fs::validate_name(stem.as_bytes())?;
    Ok(stem.to_owned())
}
