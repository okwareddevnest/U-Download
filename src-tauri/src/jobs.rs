use crate::ytdlp::{FormatChoice, TrimRange};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub type JobId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Paused,
    Probing,
    Downloading,
    Processing,
    Done,
    Failed,
    Cancelled,
}

impl JobStatus {
    /// In-flight work that occupies a concurrency slot.
    pub fn is_active(&self) -> bool {
        matches!(self, JobStatus::Probing | JobStatus::Downloading | JobStatus::Processing)
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, JobStatus::Done | JobStatus::Failed | JobStatus::Cancelled)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JobProgress {
    pub percentage: f64,
    pub speed_bytes_per_sec: u64,
    pub eta_seconds: Option<u64>,
    pub bytes_downloaded: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: JobId,
    pub url: String,
    pub title: String,
    pub thumbnail: String,
    /// `None` means "not yet known" — never substitute 0.0, which previously
    /// collapsed the frontend scrub control to a two-position slider.
    pub duration: Option<f64>,
    pub format: FormatChoice,
    pub trim: Option<TrimRange>,
    pub status: JobStatus,
    pub progress: JobProgress,
    pub output_folder: String,
    pub output_path: Option<PathBuf>,
    pub error: Option<String>,
    pub created_at: u64,
}

impl Job {
    pub fn new(
        url: String,
        format: FormatChoice,
        trim: Option<TrimRange>,
        output_folder: String,
    ) -> Self {
        Job {
            id: uuid::Uuid::new_v4().to_string(),
            url,
            title: String::new(),
            thumbnail: String::new(),
            duration: None,
            format,
            trim,
            status: JobStatus::Queued,
            progress: JobProgress::default(),
            output_folder,
            output_path: None,
            error: None,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }
}

/// A job plus the OS handle needed to cancel it.
pub struct JobHandle {
    pub job: Job,
    pub child: Option<std::process::Child>,
}

#[derive(Default)]
pub struct JobRegistry {
    handles: HashMap<JobId, JobHandle>,
    /// Insertion order, so the queue is FIFO and reorderable.
    order: Vec<JobId>,
}

impl JobRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, job: Job) -> JobId {
        let id = job.id.clone();
        self.order.push(id.clone());
        self.handles.insert(id.clone(), JobHandle { job, child: None });
        id
    }

    pub fn get(&self, id: &JobId) -> Option<Job> {
        self.handles.get(id).map(|h| h.job.clone())
    }

    pub fn set_status(&mut self, id: &JobId, status: JobStatus) {
        if let Some(h) = self.handles.get_mut(id) {
            h.job.status = status;
        }
    }

    pub fn set_error(&mut self, id: &JobId, error: String) {
        if let Some(h) = self.handles.get_mut(id) {
            h.job.status = JobStatus::Failed;
            h.job.error = Some(error);
        }
    }

    pub fn update_progress(&mut self, id: &JobId, progress: JobProgress) {
        if let Some(h) = self.handles.get_mut(id) {
            h.job.progress = progress;
        }
    }

    pub fn attach_child(&mut self, id: &JobId, child: std::process::Child) {
        if let Some(h) = self.handles.get_mut(id) {
            h.child = Some(child);
        }
    }

    /// Detaches the running process handle, leaving the job's status alone.
    ///
    /// The caller takes ownership of the process and MUST reap it: `Child`
    /// does not reap on drop, so a dropped handle leaves a zombie until the
    /// app exits. This exists so `wait()` can happen with the registry mutex
    /// released — `SharedJobs` is one mutex shared by every download thread.
    pub fn take_child(&mut self, id: &JobId) -> Option<std::process::Child> {
        self.handles.get_mut(id).and_then(|h| h.child.take())
    }

    /// Records the file the downloader actually produced.
    ///
    /// The file's stem doubles as the job title while nothing better is known
    /// — the registry itself never probes for metadata.
    pub fn set_output_path(&mut self, id: &JobId, path: PathBuf) {
        if let Some(h) = self.handles.get_mut(id) {
            if h.job.title.is_empty() {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    h.job.title = stem.to_string();
                }
            }
            h.job.output_path = Some(path);
        }
    }

    /// Kills the running process, if any, and marks the job cancelled.
    ///
    /// The kill and the reap run on a short-lived detached thread. `cancel`
    /// takes `&mut self`, so every caller holds the shared registry mutex
    /// while it runs; blocking on `wait()` here would stall every progress
    /// update, dispatch and other cancel for as long as the child took to
    /// die. The reap itself cannot be dropped — `Child` does not reap on
    /// drop, and skipping it leaves a zombie until the app exits.
    pub fn cancel(&mut self, id: &JobId) {
        let child = self.take_child(id);
        if let Some(h) = self.handles.get_mut(id) {
            h.job.status = JobStatus::Cancelled;
        }
        if let Some(mut child) = child {
            std::thread::spawn(move || {
                let _ = child.kill();
                let _ = child.wait();
            });
        }
    }

    pub fn list(&self) -> Vec<Job> {
        self.order.iter().filter_map(|id| self.get(id)).collect()
    }

    pub fn queued_ids(&self) -> Vec<JobId> {
        self.order
            .iter()
            .filter(|id| {
                self.handles
                    .get(*id)
                    .map(|h| h.job.status == JobStatus::Queued)
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    }

    pub fn active_count(&self) -> usize {
        self.handles.values().filter(|h| h.job.status.is_active()).count()
    }
}

pub type SharedJobs = Arc<Mutex<JobRegistry>>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ytdlp::{FormatChoice, MediaKind, TrimRange};

    fn sample_job() -> Job {
        Job::new(
            "https://example.com/v".to_string(),
            FormatChoice::Quick { kind: MediaKind::Mp4, height: Some(720) },
            None,
            "/out".to_string(),
        )
    }

    #[test]
    fn new_job_starts_queued_with_zero_progress() {
        let job = sample_job();
        assert_eq!(job.status, JobStatus::Queued);
        assert_eq!(job.progress.percentage, 0.0);
        assert!(job.output_path.is_none());
    }

    // Spec section 2.4: duration must never silently default to 0.0.
    #[test]
    fn new_job_has_unknown_duration_rather_than_zero() {
        assert_eq!(sample_job().duration, None);
    }

    #[test]
    fn inserted_jobs_get_distinct_ids() {
        let mut reg = JobRegistry::new();
        let a = reg.insert(sample_job());
        let b = reg.insert(sample_job());
        assert_ne!(a, b);
        assert_eq!(reg.list().len(), 2);
    }

    #[test]
    fn progress_updates_are_isolated_per_job() {
        let mut reg = JobRegistry::new();
        let a = reg.insert(sample_job());
        let b = reg.insert(sample_job());

        reg.update_progress(&a, JobProgress { percentage: 42.0, ..Default::default() });

        assert_eq!(reg.get(&a).unwrap().progress.percentage, 42.0);
        assert_eq!(reg.get(&b).unwrap().progress.percentage, 0.0);
    }

    #[test]
    fn active_count_counts_only_in_flight_work() {
        let mut reg = JobRegistry::new();
        let a = reg.insert(sample_job());
        let b = reg.insert(sample_job());
        let c = reg.insert(sample_job());

        reg.set_status(&a, JobStatus::Downloading);
        reg.set_status(&b, JobStatus::Processing);
        reg.set_status(&c, JobStatus::Done);

        assert_eq!(reg.active_count(), 2);
    }

    #[test]
    fn queued_ids_preserve_insertion_order_and_exclude_paused() {
        let mut reg = JobRegistry::new();
        let a = reg.insert(sample_job());
        let b = reg.insert(sample_job());
        let c = reg.insert(sample_job());
        reg.set_status(&b, JobStatus::Paused);

        assert_eq!(reg.queued_ids(), vec![a, c]);
    }

    #[test]
    fn trim_range_round_trips_on_the_job() {
        let mut reg = JobRegistry::new();
        let mut job = sample_job();
        job.trim = Some(TrimRange { start: 5.0, end: 12.0 });
        let id = reg.insert(job);

        let stored = reg.get(&id).unwrap().trim.unwrap();
        assert_eq!(stored.start, 5.0);
        assert_eq!(stored.end, 12.0);
    }

    #[test]
    fn updating_a_missing_job_is_a_no_op_not_a_panic() {
        let mut reg = JobRegistry::new();
        reg.set_status(&"nonexistent".to_string(), JobStatus::Done);
        reg.update_progress(&"nonexistent".to_string(), JobProgress::default());
        assert_eq!(reg.list().len(), 0);
    }
}
