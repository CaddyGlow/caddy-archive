//! Run each worker count in a fresh process to compare process peak RSS fairly.
#[cfg(all(feature = "sevenz", feature = "parallel"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use archive_core::{Archive, Limits};
    use std::{fs, time::Instant};
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("archive path required")?;
    let workers: usize = args.next().ok_or("workers required")?.parse()?;
    let start = Instant::now();
    let mut archive = Archive::open(fs::File::open(path)?, Limits::default())?;
    let indexed = start.elapsed();
    let ids: Vec<_> = archive.entries().iter().map(|entry| entry.id).collect();
    let extraction = Instant::now();
    let mut first = None;
    let mut delivered = 0u64;
    let report = archive.extract_selected_parallel(&ids, workers, &|| false, &mut |_, bytes| {
        first.get_or_insert_with(|| extraction.elapsed());
        delivered += bytes.len() as u64;
        Ok(())
    })?;
    let elapsed = extraction.elapsed();
    let peak = fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with("VmHWM:"))
                .map(str::to_owned)
        });
    println!(
        "requested={workers} used={} folders={} decoded={} delivered={delivered} verified={} index_ms={:.3} first_byte_ms={} extract_ms={:.3} peak_rss={} fallback={:?}",
        report.workers_used,
        report.folder_tasks,
        report.decoded_bytes,
        report.report.verified,
        indexed.as_secs_f64() * 1000.0,
        first
            .map(|time| format!("{:.3}", time.as_secs_f64() * 1000.0))
            .unwrap_or_else(|| "none".into()),
        elapsed.as_secs_f64() * 1000.0,
        peak.unwrap_or_else(|| "unavailable".into()),
        report.fallback_reason
    );
    Ok(())
}
#[cfg(not(all(feature = "sevenz", feature = "parallel")))]
fn main() {
    eprintln!("enable sevenz and parallel features");
}
