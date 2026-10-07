use std::ffi::OsString;

/// Translate a leading tar-style option word without changing ordinary arc flags.
pub(crate) fn expand(mut args: Vec<OsString>) -> Result<Vec<OsString>, clap::Error> {
    let index = if args
        .get(1)
        .is_some_and(|arg| arg == "-j" || arg == "--json")
    {
        2
    } else {
        1
    };
    let Some(word) = args.get(index).and_then(|arg| arg.to_str()) else {
        return Ok(args);
    };
    let word = word.strip_prefix('-').unwrap_or(word);
    if word.len() < 2 || !word.chars().all(|flag| "cxtfvzJja".contains(flag)) {
        return Ok(args);
    }
    let error = |message| clap::Error::raw(clap::error::ErrorKind::InvalidValue, message);
    let operations: Vec<_> = word.chars().filter(|flag| "cxt".contains(*flag)).collect();
    if operations.len() != 1 || !word.contains('f') {
        return Err(error(
            "tar-style flags require exactly one of c, x, t and an f followed by the archive filename",
        ));
    }
    let wrappers: Vec<_> = word.chars().filter(|flag| "zJja".contains(*flag)).collect();
    if wrappers.len() > 1 {
        return Err(error(
            "tar-style compression flags z, J, j, and a are mutually exclusive",
        ));
    }
    let verbose = word.contains('v');
    let operation = operations[0];
    let wrapper = wrappers.first().copied();
    let archive = args
        .get(index + 1)
        .cloned()
        .ok_or_else(|| error("tar-style f requires an archive filename"))?;
    let mut expanded = args.drain(..index).collect::<Vec<_>>();
    expanded.push(
        if operation == 'c' {
            "create"
        } else if operation == 'x' {
            "extract"
        } else {
            "list"
        }
        .into(),
    );
    if verbose {
        expanded.push("--verbose".into());
    }
    let format = match wrapper {
        Some('z') => Some("tar.gz"),
        Some('J') => Some("tar.xz"),
        Some('j') => Some("tar.bz2"),
        _ if operation == 'c' && wrapper != Some('a') => Some("tar"),
        _ => None,
    };
    if let Some(format) = format {
        expanded.extend(["--format".into(), format.into()]);
    }
    let remaining = if operation == 'c' {
        let source = args.get(2).cloned().ok_or_else(|| {
            error("tar-style creation requires one source directory after the archive filename")
        })?;
        expanded.extend(["--output".into(), archive, "--input".into(), source]);
        3
    } else {
        expanded.push(archive);
        2
    };
    for argument in args.into_iter().skip(remaining) {
        if operation == 'x' && argument == "-C" {
            expanded.push("--output".into());
        } else {
            expanded.push(argument);
        }
    }
    Ok(expanded)
}
