#[cfg(feature = "progress")]
pub(crate) struct RenderProgress {
    bar: indicatif::ProgressBar,
    pub(crate) observer: archive_core::progress::CoalescingObserver,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
#[cfg(feature = "progress")]
impl RenderProgress {
    pub(crate) fn new() -> Self {
        let bar = indicatif::ProgressBar::new_spinner();
        if let Ok(style) = indicatif::ProgressStyle::with_template("{spinner} {bytes}") {
            bar.set_style(style);
        }
        bar.enable_steady_tick(std::time::Duration::from_millis(150));
        let observer = archive_core::progress::CoalescingObserver::default();
        let receiver = observer.clone();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_stop = stop.clone();
        let rendering = bar.clone();
        let thread = std::thread::spawn(move || {
            while !thread_stop.load(std::sync::atomic::Ordering::Relaxed) {
                if let Some(snapshot) = receiver.take() {
                    if let Some(total) = snapshot.total_selected_bytes {
                        rendering.set_length(total);
                    }
                    rendering.set_position(snapshot.selected_bytes_written);
                }
                std::thread::park_timeout(std::time::Duration::from_millis(100));
            }
        });
        Self {
            bar,
            observer,
            stop,
            thread: Some(thread),
        }
    }
    pub(crate) fn finish_and_clear(self) {}
}
#[cfg(feature = "progress")]
impl Drop for RenderProgress {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
        if let Some(snapshot) = self.observer.take() {
            self.bar.set_position(snapshot.selected_bytes_written);
        }
        self.bar.finish_and_clear();
    }
}
