use std::{fmt::Display, sync::{atomic::{AtomicU64, AtomicU8, Ordering}, Arc}, time::Duration};

use compact_str::{CompactString, ToCompactString};
use console::{style, pad_str};
use indicatif::{ProgressBar, ProgressStyle, ProgressFinish, HumanBytes};
use tokio::{sync::Mutex, time::sleep};

#[derive(Clone, Default)]
pub struct Progress {
    pub step: Arc<AtomicU8>,
    step_name: Arc<Mutex<CompactString>>,
    pub files: ProgressPart,
    pub bytes: ProgressPart,
    pub total_bytes: Arc<AtomicU64>,
    total_steps: Arc<AtomicU8>
}

impl Progress {
    pub fn new() -> Self {
        Self {
            step_name: Arc::new(Mutex::new(CompactString::new(""))),
            step: Arc::new(AtomicU8::new(0)),
            files: ProgressPart::new(),
            bytes: ProgressPart::new(),
            total_bytes: Arc::new(AtomicU64::new(0)),
            total_steps: Arc::new(AtomicU8::new(4))
        }
    }

    pub fn with_step(step: &str) -> Self {
        Self {
            step_name: Arc::new(Mutex::new(step.to_compact_string())),
            step: Arc::new(AtomicU8::new(0)),
            files: ProgressPart::new(),
            bytes: ProgressPart::new(),
            total_bytes: Arc::new(AtomicU64::new(0)),
            total_steps: Arc::new(AtomicU8::new(4))
        }
    }

    pub async fn create_prefix(&self) -> String {
        pad_str(
            &style(format!(
                "[{}/{}] {}", 
                self.step.load(Ordering::SeqCst),
                self.total_steps.load(Ordering::SeqCst), 
                pad_str(self.step_name.lock().await.as_str(), 17, console::Alignment::Right, None)
            )).bold().to_string(), 
            23, 
            console::Alignment::Left, 
            None
        ).to_string()
    }

    pub async fn create_prefix_stepless(&self) -> String {
        pad_str(
            &style(format!(
                "{}", 
                pad_str(self.step_name.lock().await.as_str(), 14, console::Alignment::Right, None)
            )).bold().to_string(), 
            23, 
            console::Alignment::Right, 
            None
        ).to_string()
    }

    pub async fn create_processing_progress_bar(&self) -> ProgressBar {
        let prefix = self.create_prefix_stepless().await;

        ProgressBar::new(self.bytes.total())
            .with_style(
                ProgressStyle::default_bar()
                    .template(
                        "{prefix} [{wide_bar:.green/dim}] {pos}/{len}",
                    )
                    .expect("template string should follow the syntax")
                    .progress_chars("###"),
            )
            .with_finish(ProgressFinish::AndLeave)
            .with_prefix(prefix)
    }

    pub async fn create_download_progress_bar(&self) -> ProgressBar {
        let prefix = self.create_prefix().await;

        ProgressBar::new(self.files.total())
            .with_style(
                ProgressStyle::default_bar()
                    .template(
                        "{prefix} [{wide_bar:.cyan/dim}] {pos}/{len} [{elapsed_precise}] [{msg}]",
                    )
                    .expect("template string should follow the syntax")
                    .progress_chars("###"),
                    
            )
            .with_finish(ProgressFinish::AndLeave)
            .with_prefix(prefix)
    }

    pub async fn update_for_files(&self, progress_bar: &ProgressBar) {
        progress_bar.set_length(self.files.total());
        progress_bar.set_position(self.files.total() - self.files.remaining());
        progress_bar.set_message(HumanBytes(self.bytes.success()).to_compact_string());

        if self.step.load(Ordering::SeqCst) == 0 {
            progress_bar.set_prefix(self.create_prefix_stepless().await);
        }
    }

    pub fn set_total_steps(&self, num_steps: u8) {
        self.total_steps.store(num_steps, Ordering::SeqCst);
    }

    pub async fn set_step(&self, step_name: &str) {
        *self.step_name.lock().await = step_name.to_compact_string();
    }

    pub async fn next_step(&self, step_name: &str) {
        *self.step_name.lock().await = step_name.to_compact_string();

        self.bytes.reset();
        self.files.reset();

        self.step.fetch_add(1, Ordering::SeqCst);
    }
    
    pub async fn wait_for_idle(&self, progress_bar: &ProgressBar)  {
        while self.files.remaining() > 0 {
            self.update_for_files(progress_bar).await;
            sleep(Duration::from_millis(100)).await
        }

        self.update_for_files(progress_bar).await;
    }

    pub async fn wait_for_completion(&self, progress_bar: &ProgressBar)  {
        while self.files.remaining() > 0 {
            self.update_for_files(progress_bar).await;
            sleep(Duration::from_millis(100)).await
        }

        self.total_bytes.fetch_add(self.bytes.success(), Ordering::SeqCst);

        self.update_for_files(progress_bar).await;

        progress_bar.finish_using_style();
    }
}


#[derive(Clone, Default, Debug)]
pub struct ProgressPart {
    total: Arc<AtomicU64>,
    success: Arc<AtomicU64>,
    skipped: Arc<AtomicU64>,
    failed: Arc<AtomicU64>
}

impl Display for ProgressPart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(
            format_args!(
                "{} succeeded, {} skipped, {} failed",
                self.success(), self.skipped(), self.failed()
            )
        )
    }
}

impl ProgressPart {
    pub fn new() -> Self {
        Self {
            total: Arc::new(AtomicU64::new(0)),
            success: Arc::new(AtomicU64::new(0)),
            skipped: Arc::new(AtomicU64::new(0)),
            failed: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn inc_total(&self, count: u64) {
        self.total.fetch_add(count, Ordering::SeqCst);
    }

    pub fn inc_success(&self, count: u64) {
        self.success.fetch_add(count, Ordering::SeqCst);
    }

    pub fn inc_skipped(&self, count: u64) {
        self.skipped.fetch_add(count, Ordering::SeqCst);
    }

    pub fn inc_failed(&self, count: u64) {
        self.failed.fetch_add(count, Ordering::SeqCst);
    }

    pub fn total(&self) -> u64 {
        self.total.load(Ordering::SeqCst)
    }

    pub fn success(&self) -> u64 {
        self.success.load(Ordering::SeqCst)
    }

    pub fn skipped(&self) -> u64 {
        self.skipped.load(Ordering::SeqCst)
    }

    pub fn failed(&self) -> u64 {
        self.failed.load(Ordering::SeqCst)
    }

    pub fn remaining(&self) -> u64 {
        self.total.load(Ordering::SeqCst) -
            self.success.load(Ordering::SeqCst) -
            self.skipped.load(Ordering::SeqCst) -
            self.failed.load(Ordering::SeqCst)
    }

    pub fn reset(&self) {
        self.total.store(0, Ordering::SeqCst);
        self.success.store(0, Ordering::SeqCst);
        self.skipped.store(0, Ordering::SeqCst);
        self.failed.store(0, Ordering::SeqCst);
    }
}
