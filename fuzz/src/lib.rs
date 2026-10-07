//! Bounded archive, CAB, package and optical fuzz harnesses.
pub mod archives;
pub mod udf;
use cabinet::Cabinet;
use std::io::{Cursor, Read};
/// Maximum CAB input and cumulative decoded member size.
pub const ARCHIVE_LIMIT: usize = 1 << 20;

/// Parse a CAB and stream bounded member bytes without filesystem extraction.
pub fn cab(data: &[u8]) {
    if data.len() > ARCHIVE_LIMIT {
        return;
    }
    let Ok(mut cabinet) = Cabinet::new(Cursor::new(data)) else {
        return;
    };
    let names: Vec<_> = cabinet
        .entries()
        .iter()
        .take(64)
        .map(|e| e.name.clone())
        .collect();
    let mut budget = ARCHIVE_LIMIT as u64;
    for name in names {
        if budget == 0 {
            break;
        }
        if let Ok(member) = cabinet.read_file(&name) {
            let mut limited = member.take(budget);
            let mut buffer = [0; 4096];
            loop {
                match limited.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => budget -= n as u64,
                }
            }
        }
    }
}

/// Replay a named archive harness.
pub fn run(target: &str, data: &[u8]) -> Result<(), &'static str> {
    match target {
        "archive" => archives::archive(data),
        "package" => archives::package(data),
        "optical" => archives::optical(data),
        "cab" => cab(data),
        "udf" => {
            let _ = udf::read(data);
        }
        "udf_roundtrip" => {
            let _ = udf::roundtrip(data);
        }
        _ => return Err("unknown archive fuzz target"),
    }
    Ok(())
}
