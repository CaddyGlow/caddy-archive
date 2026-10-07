//! Run with `cargo run --release -p archive-cli --example progress-bench`.
use archive_core::{
    Archive, CreateEntry, EntryId, EntryKind, Format, Limits,
    progress::{CoalescingObserver, NoProgress},
};
use std::{
    io::{Cursor, sink},
    time::Instant,
};
#[cfg(feature = "progress")]
#[path = "../src/render_progress.rs"]
mod render_progress;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = 0x12345678u32;
    let mib = std::env::var("ARC_BENCH_MIB")
        .ok()
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(32);
    let payload_bytes = mib
        .checked_mul(1024 * 1024)
        .ok_or("benchmark size overflow")?;
    let rounds = std::env::var("ARC_BENCH_ROUNDS")
        .ok()
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(20);
    if rounds < 2 || rounds % 2 != 0 {
        return Err("benchmark rounds must be positive and even".into());
    }
    let data: Vec<_> = (0..payload_bytes)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state as u8) & 15
        })
        .collect();
    let mut encoded = Cursor::new(Vec::new());
    archive_core::create(
        Format::Zip,
        &[CreateEntry {
            name: "payload".into(),
            data,
            kind: EntryKind::File,
        }],
        &mut encoded,
        Limits::default(),
    )?;
    let mut results = Vec::new();
    // Rotate scenarios each round to avoid assigning warmup or thermal effects to one path.
    let scenarios = if cfg!(feature = "progress") { 4 } else { 3 };
    let mut timings = vec![Vec::new(); scenarios];
    for round in 0..=rounds {
        for offset in 0..scenarios {
            let scenario = (round + offset) % scenarios;
            let mut archive = Archive::open(Cursor::new(encoded.get_ref()), Limits::default())?;
            let start = Instant::now();
            match scenario {
                0 => {
                    archive.extract(EntryId(0), &mut sink())?;
                }
                1 => {
                    archive.extract_observed(EntryId(0), &mut sink(), &mut NoProgress)?;
                }
                2 => {
                    let mut observer = CoalescingObserver::default();
                    archive.extract_observed(EntryId(0), &mut sink(), &mut observer)?;
                    assert!(
                        observer
                            .take()
                            .is_some_and(|snapshot| snapshot.verified && snapshot.terminal)
                    );
                }
                #[cfg(feature = "progress")]
                _ => {
                    let mut rendering = render_progress::RenderProgress::new();
                    archive.extract_observed(EntryId(0), &mut sink(), &mut rendering.observer)?;
                    rendering.finish_and_clear();
                }
                #[cfg(not(feature = "progress"))]
                _ => unreachable!(),
            }
            if round > 0 {
                timings[scenario].push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
    }
    for (scenario, mut samples) in timings.into_iter().enumerate() {
        samples.sort_by(f64::total_cmp);
        let middle = samples.len() / 2;
        let median = (samples[middle - 1] + samples[middle]) / 2.0;
        let label = [
            "no_reporting",
            "compiled_disabled",
            "coalesced_snapshots",
            "terminal_adapter",
        ][scenario];
        results.push(serde_json::json!({"scenario":label,"median_ms":median,"min_ms":samples[0],"max_ms":samples[samples.len()-1],"samples_ms":samples}));
    }
    println!(
        "{}",
        serde_json::json!({"payload_bytes":payload_bytes,"rounds":rounds,"results":results,"terminal_rendering_measured":cfg!(feature="progress")&&std::io::IsTerminal::is_terminal(&std::io::stderr()),"browser_measured":false})
    );
    Ok(())
}
