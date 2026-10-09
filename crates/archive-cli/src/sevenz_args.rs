//! Read-only compatibility namespace lowering into native operations.
//!
//! Supports a documented subset of the pinned upstream command grammar, with
//! component-local byte wildcard semantics supplied by the native selector.
//! No encoder/update/password property is silently accepted. Error messages never
//! contain caller-supplied command, option, path, pattern or password values.

use archive_core::selection::{MatchScope, NamePattern, PatternMode, SelectionLimits};
use std::ffi::OsString;
use std::fmt;

/// Stable error category for the CLI's existing exit-code mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Argument,
    Unsupported,
}

/// Frontend errors contain static messages only, protecting secret arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrontendError {
    pub kind: ErrorKind,
    pub message: &'static str,
}

impl fmt::Display for FrontendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for FrontendError {}

fn argument(message: &'static str) -> FrontendError {
    FrontendError {
        kind: ErrorKind::Argument,
        message,
    }
}

fn unsupported(message: &'static str) -> FrontendError {
    FrontendError {
        kind: ErrorKind::Unsupported,
        message,
    }
}

/// Lower a full argv beginning `PROGRAM 7z` into native argv.
///
/// A single native-extension `-j`/`--json` can precede the namespace or appear
/// inside it. Other invocations are returned unchanged. Switches can appear around command
/// operands. `--` terminates switch parsing. The archive path is kept as OsString;
/// UTF-8 is required only for command/format/pattern values. Additional names
/// become include patterns. `@listfile` expansion is unavailable and rejected;
/// use `-i!@name` to select an actual name beginning with `@`.
pub fn lower(args: Vec<OsString>) -> Result<Vec<OsString>, FrontendError> {
    let namespace_index = args
        .iter()
        .skip(1)
        .take_while(|arg| *arg == "-j" || *arg == "--json")
        .count()
        + 1;
    if args.get(namespace_index).is_none_or(|arg| arg != "7z") {
        if has_namespace_after_native_prefix(&args) {
            return Err(unsupported(
                "7z namespace must follow only an optional --json/-j",
            ));
        }
        return Ok(args);
    }
    if namespace_index > 2 {
        return Err(argument("duplicate JSON switch"));
    }
    let mut json = namespace_index == 2;
    let program = args
        .first()
        .cloned()
        .ok_or_else(|| argument("missing program name"))?;
    let mut command = None;
    let mut archive = None;
    let mut format = None;
    let mut output = None;
    let mut includes = Vec::new();
    let mut excludes = Vec::new();
    let mut switch_parsing = true;
    let mut literal = false;
    let mut literal_seen = false;
    let mut insensitive = false;
    let mut case_seen = false;
    let limits = SelectionLimits::default();
    let mut total_pattern_bytes = 0usize;

    for arg in args.into_iter().skip(namespace_index + 1) {
        if switch_parsing && arg == "--" {
            switch_parsing = false;
            continue;
        }
        if switch_parsing && arg.as_encoded_bytes().starts_with(b"-") {
            let option = arg
                .to_str()
                .ok_or_else(|| argument("switch values must be UTF-8"))?;
            if option.starts_with("-p") {
                return Err(unsupported(
                    "7z password input is not implemented; use native --password-file",
                ));
            }
            if option.starts_with("-m") {
                return Err(unsupported("7z method properties are not implemented"));
            }
            if matches!(option, "-j" | "--json") {
                if json {
                    return Err(argument("duplicate JSON switch"));
                }
                json = true;
            } else if option == "-spd" {
                if literal_seen {
                    return Err(argument("duplicate wildcard policy switch"));
                }
                literal_seen = true;
                literal = true;
            } else if matches!(option, "-ssc" | "-ssc-") {
                if case_seen {
                    return Err(argument("duplicate case policy switch"));
                }
                case_seen = true;
                insensitive = option == "-ssc-";
            } else if let Some(value) = option.strip_prefix("-t") {
                if value.is_empty() {
                    return Err(argument("7z -t requires an attached format name"));
                }
                if format.is_some() {
                    return Err(argument("duplicate format switch"));
                }
                let normalized = value.to_ascii_lowercase();
                if !matches!(
                    normalized.as_str(),
                    "zip"
                        | "7z"
                        | "tar"
                        | "gzip"
                        | "bzip2"
                        | "xz"
                        | "lzma"
                        | "cab"
                        | "wim"
                        | "iso"
                        | "udf"
                ) {
                    return Err(unsupported(
                        "7z format/profile is not supported by this frontend",
                    ));
                }
                format = Some(normalized);
            } else if let Some(value) = option.strip_prefix("-o") {
                if value.is_empty() {
                    return Err(argument("7z -o requires an attached output directory"));
                }
                if output.is_some() {
                    return Err(argument("duplicate output directory switch"));
                }
                output = Some(value.to_owned());
            } else if let Some(value) = option.strip_prefix("-i!") {
                add_pattern(&mut includes, value, &mut total_pattern_bytes, limits)?;
            } else if let Some(value) = option.strip_prefix("-x!") {
                add_pattern(&mut excludes, value, &mut total_pattern_bytes, limits)?;
            } else if option.starts_with("-i") || option.starts_with("-x") {
                return Err(unsupported(
                    "only immediate -i!PATTERN/-x!PATTERN selectors are implemented",
                ));
            } else {
                return Err(unsupported("7z switch is not implemented"));
            }
            continue;
        }
        if command.is_none() {
            let name = arg
                .to_str()
                .ok_or_else(|| argument("7z command must be UTF-8"))?;
            command = Some(match name.to_ascii_lowercase().as_str() {
                "l" => "list",
                "t" => "test",
                "x" => "extract",
                "e" => return Err(unsupported("7z flat extraction is not implemented")),
                "a" | "u" | "d" | "rn" => {
                    return Err(unsupported("7z archive editing is not implemented"));
                }
                "b" | "i" | "h" => return Err(unsupported("7z command is not implemented")),
                _ => {
                    return Err(argument(
                        "unknown 7z command; supported commands are l, t and x",
                    ));
                }
            });
        } else if archive.is_none() {
            if arg.as_encoded_bytes().starts_with(b"@") {
                return Err(unsupported("7z archive listfiles are not implemented"));
            }
            if arg.is_empty() {
                return Err(argument("7z requires a nonempty archive path"));
            }
            archive = Some(arg);
        } else {
            let name = arg
                .to_str()
                .ok_or_else(|| argument("selection operands must be UTF-8"))?;
            if name.starts_with('@') {
                return Err(unsupported(
                    "7z listfiles are not implemented; use -i! for literal @ names",
                ));
            }
            add_pattern(&mut includes, name, &mut total_pattern_bytes, limits)?;
        }
    }
    let command = command.ok_or_else(|| argument("7z requires command l, t or x"))?;
    let archive = archive
        .ok_or_else(|| argument("7z requires exactly one archive path before selection names"))?;
    if output.is_some() && command != "extract" {
        return Err(unsupported("7z output directory is supported only for x"));
    }
    if includes
        .len()
        .checked_add(excludes.len())
        .is_none_or(|count| count > limits.max_patterns)
    {
        return Err(argument("too many selection patterns"));
    }
    let mode = if literal {
        PatternMode::Literal
    } else {
        PatternMode::Wildcard
    };
    for pattern in includes.iter().chain(&excludes) {
        NamePattern::new(pattern.as_bytes(), mode, MatchScope::FullPath, true).map_err(|_| {
            unsupported("selection pattern is outside the supported native wildcard profile")
        })?;
    }

    let mut native = vec![program, OsString::from(command)];
    if json {
        native.push(OsString::from("--json"));
    }
    if let Some(format) = format {
        native.push(OsString::from(format!("--format={format}")));
    }
    if let Some(output) = output {
        native.push(OsString::from(format!("--output={output}")));
    }
    if literal {
        native.push(OsString::from("--literal-names"));
    }
    if insensitive {
        native.push(OsString::from("--ignore-ascii-case"));
    }
    for pattern in includes {
        native.push(OsString::from(format!("--include={pattern}")));
    }
    for pattern in excludes {
        native.push(OsString::from(format!("--exclude={pattern}")));
    }
    // Keep arbitrary archive paths from being interpreted as native options.
    native.push(OsString::from("--"));
    native.push(archive);
    Ok(native)
}

// Scan only the leading native option region. Once a native command or -- is
// reached, a later filename "7z" is ordinary input. Known option operands are
// skipped even when their literal value is "7z" (for example password-file).
fn has_namespace_after_native_prefix(args: &[OsString]) -> bool {
    let mut index = 1;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            return false;
        }
        if arg == "7z" {
            return true;
        }
        let Some(text) = arg.to_str() else {
            return false;
        };
        let word = text.strip_prefix('-').unwrap_or(text);
        let tar_word = word.len() >= 2 && word.bytes().all(|byte| b"cxtfvzJja".contains(&byte));
        if tar_word || !text.starts_with('-') {
            // This includes every native command/alias, and conservatively stops
            // at unknown positional commands rather than interpreting filenames.
            return false;
        }
        let takes_next = if text.starts_with("--") {
            !text.contains('=')
                && matches!(
                    text,
                    "--password-file"
                        | "--bundle-entry"
                        | "--media"
                        | "--image"
                        | "--image-name"
                        | "--view"
                        | "--max-input-bytes"
                        | "--max-entry-bytes"
                        | "--max-total-bytes"
                        | "--max-entries"
                        | "--memuse"
                        | "--mmemuse"
                        | "--max-codec-workspace-bytes"
                        | "--max-dictionary-bytes"
                        | "--progress"
                        | "--include"
                        | "--exclude"
                )
        } else if text.starts_with("-mmemuse=") {
            false
        } else {
            // Clap permits clusters of boolean flags followed by a value flag;
            // an inline remainder belongs to that flag, not another switch.
            let short = &text.as_bytes()[1..];
            let mut takes_next = false;
            for (offset, &byte) in short.iter().enumerate() {
                if b"pbmInvMP".contains(&byte) {
                    takes_next = offset + 1 == short.len();
                    break;
                }
                if !b"jhV".contains(&byte) {
                    break;
                }
            }
            takes_next
        };
        index += if takes_next { 2 } else { 1 };
    }
    false
}

fn add_pattern(
    patterns: &mut Vec<String>,
    pattern: &str,
    total: &mut usize,
    limits: SelectionLimits,
) -> Result<(), FrontendError> {
    if pattern.is_empty() {
        return Err(argument("selection pattern is empty"));
    }
    if patterns.len() >= limits.max_patterns || pattern.len() > limits.max_pattern_bytes {
        return Err(argument("selection pattern resource limit exceeded"));
    }
    *total = total
        .checked_add(pattern.len())
        .ok_or_else(|| argument("selection pattern resource limit exceeded"))?;
    if *total > limits.max_total_pattern_bytes {
        return Err(argument("selection pattern resource limit exceeded"));
    }
    patterns.push(pattern.to_owned());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn lowers_read_commands_and_attached_upstream_switches() {
        assert_eq!(
            lower(argv(&[
                "arc",
                "7z",
                "x",
                "input.zip",
                "-tzip",
                "-odest",
                "-i!dir/*",
                "-x!dir/private",
                "dir/name"
            ]))
            .unwrap(),
            argv(&[
                "arc",
                "extract",
                "--format=zip",
                "--output=dest",
                "--include=dir/*",
                "--include=dir/name",
                "--exclude=dir/private",
                "--",
                "input.zip"
            ])
        );
        assert_eq!(
            lower(argv(&["arc", "7z", "-t7z", "L", "input"])).unwrap(),
            argv(&["arc", "list", "--format=7z", "--", "input"])
        );
        assert_eq!(
            lower(argv(&["arc", "7z", "t", "input"])).unwrap(),
            argv(&["arc", "test", "--", "input"])
        );
    }

    #[test]
    fn terminator_and_immediate_names_cannot_inject_native_switches() {
        assert_eq!(
            lower(argv(&[
                "arc",
                "7z",
                "l",
                "-i!@entry",
                "--",
                "-archive",
                "--json"
            ]))
            .unwrap(),
            argv(&[
                "arc",
                "list",
                "--include=@entry",
                "--include=--json",
                "--",
                "-archive"
            ])
        );
    }

    #[test]
    fn literal_and_ascii_case_policies_lower_explicitly() {
        assert_eq!(
            lower(argv(&[
                "arc", "7z", "l", "input", "[name]", "-spd", "-ssc-"
            ]))
            .unwrap(),
            argv(&[
                "arc",
                "list",
                "--literal-names",
                "--ignore-ascii-case",
                "--include=[name]",
                "--",
                "input"
            ])
        );
        assert_eq!(
            lower(argv(&["arc", "7z", "l", "input", "-ssc"])).unwrap(),
            argv(&["arc", "list", "--", "input"])
        );
    }

    #[test]
    fn unsupported_commands_and_options_never_echo_values() {
        for args in [
            vec!["arc", "7z", "a", "input"],
            vec!["arc", "7z", "rn", "input", "old", "new"],
            vec!["arc", "7z", "e", "input"],
            vec!["arc", "7z", "l", "input", "-pTOPSECRET"],
            vec!["arc", "7z", "l", "input", "-mTOPSECRET"],
            vec!["arc", "7z", "l", "input", "--json=TOPSECRET"],
            vec!["arc", "7z", "l", "input", "-i@TOPSECRET"],
        ] {
            let error = lower(argv(&args)).unwrap_err();
            assert_eq!(error.kind, ErrorKind::Unsupported);
            assert!(!error.to_string().contains("TOPSECRET"));
        }
    }

    #[test]
    fn missing_and_duplicate_values_are_errors() {
        for args in [
            vec!["arc", "7z"],
            vec!["arc", "7z", "l"],
            vec!["arc", "7z", "l", "input", "-t"],
            vec!["arc", "7z", "x", "input", "-o"],
            vec!["arc", "7z", "l", "input", "-tzip", "-t7z"],
            vec!["arc", "7z", "l", "input", "-ssc", "-ssc-"],
        ] {
            assert_eq!(lower(argv(&args)).unwrap_err().kind, ErrorKind::Argument);
        }
    }

    #[test]
    fn unknown_grammars_and_listfiles_fail_closed() {
        for option in [
            "-ir!dir",
            "-x@file",
            "-r",
            "-tbr",
            "-i!**",
            "-i![ab]",
            "@listfile",
        ] {
            assert_eq!(
                lower(argv(&["arc", "7z", "l", "input", option]))
                    .unwrap_err()
                    .kind,
                ErrorKind::Unsupported
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_archive_paths_are_preserved() {
        use std::os::unix::ffi::OsStringExt;
        let path = OsString::from_vec(vec![b'-', 0xff, b'.', b'z']);
        let mut args = argv(&["arc", "7z", "l", "--"]);
        args.push(path.clone());
        assert_eq!(lower(args).unwrap().last(), Some(&path));
    }

    #[test]
    fn json_extension_before_or_inside_namespace_is_safe() {
        for args in [
            vec!["arc", "--json", "7z", "l", "input"],
            vec!["arc", "-j", "7z", "l", "input"],
            vec!["arc", "7z", "l", "input", "--json"],
            vec!["arc", "7z", "l", "input", "-j"],
        ] {
            assert_eq!(
                lower(argv(&args)).unwrap(),
                argv(&["arc", "list", "--json", "--", "input"])
            );
        }
        for prefix in ["-j", "--json"] {
            for option in ["-pTOPSECRET", "--unknown=TOPSECRET"] {
                let error = lower(argv(&["arc", prefix, "7z", "l", "input", option])).unwrap_err();
                assert_eq!(error.kind, ErrorKind::Unsupported);
                assert!(!error.to_string().contains("TOPSECRET"));
            }
        }
    }

    #[test]
    fn unsupported_native_prefixes_use_constant_namespace_errors() {
        for args in [
            vec![
                "arc",
                "--json",
                "-mmemuse=TOPSECRET",
                "7z",
                "l",
                "missing",
                "-pPASSWORD",
            ],
            vec![
                "arc",
                "--progress",
                "never",
                "7z",
                "l",
                "missing",
                "-pPASSWORD",
            ],
            vec![
                "arc",
                "--json",
                "--view",
                "udf",
                "7z",
                "l",
                "missing",
                "-pPASSWORD",
            ],
        ] {
            let error = lower(argv(&args)).unwrap_err();
            assert_eq!(error.kind, ErrorKind::Unsupported);
            assert!(!error.to_string().contains("TOPSECRET"));
            assert!(!error.to_string().contains("PASSWORD"));
        }
        for args in [
            vec!["arc", "--password-file", "7z", "list", "input"],
            vec!["arc", "-p", "7z", "list", "input"],
            vec!["arc", "-jp", "7z", "list", "input"],
            vec!["arc", "--include", "7z", "list", "input"],
            vec!["arc", "--json", "list", "7z"],
            vec!["arc", "tf", "7z"],
            vec!["arc", "-xvf", "7z"],
            vec!["arc", "--", "7z"],
        ] {
            let original = argv(&args);
            assert_eq!(lower(original.clone()).unwrap(), original);
        }
    }

    #[test]
    fn native_invocations_are_untouched() {
        let args = argv(&["arc", "create", "-i", "source", "-o", "output"]);
        assert_eq!(lower(args.clone()).unwrap(), args);
    }
}
