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
    /// — the registry itself never probes for metadata. A real metadata title
    /// from `set_metadata` always wins over this guess, in either arrival
    /// order: this only fills the title in when it is still empty, and
    /// `set_metadata` unconditionally overwrites whatever a stem guess left
    /// behind once real metadata arrives. See `set_metadata`.
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

    /// Applies a metadata probe's result to a job (see `probe::fetch_job_metadata`).
    ///
    /// A non-empty `title` always overwrites — including a filename-stem guess
    /// `set_output_path` already wrote — because a real video title is what
    /// the user recognises and should win regardless of which arrived first:
    /// the probe usually finishes well before a download does, but a fast
    /// download racing a slow probe is not ruled out. An empty `title` (the
    /// probe failed or the site returned nothing) leaves whatever is already
    /// there alone, so a stem guess set earlier — or one set later, once the
    /// download starts — still gets its chance. Same reasoning for
    /// `thumbnail`, which has no other writer at all.
    pub fn set_metadata(&mut self, id: &JobId, title: &str, thumbnail: &str) {
        if let Some(h) = self.handles.get_mut(id) {
            if !title.is_empty() {
                h.job.title = title.to_string();
            }
            if !thumbnail.is_empty() {
                h.job.thumbnail = thumbnail.to_string();
            }
        }
    }

    /// Kills the running process *and everything it spawned*, then marks the
    /// job cancelled.
    ///
    /// The whole process group is signalled, not just yt-dlp. yt-dlp does not
    /// do the downloading itself: trimmed jobs are fetched by an ffmpeg child
    /// and untrimmed ones by aria2c. Killing only yt-dlp left that grandchild
    /// running — it went on writing the output file, so a job the UI called
    /// cancelled still produced one, and it held the inherited pipes open, so
    /// the runner's reader threads blocked until it finished anyway. See
    /// `crate::proc`.
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
                crate::proc::kill_tree(&mut child);
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

    // --- metadata precedence --------------------------------------------
    //
    // Spec: a real metadata title always wins over the filename-stem guess
    // `set_output_path` makes, in either arrival order; a failed/empty probe
    // must not erase whatever title is already there.

    #[test]
    fn metadata_fills_in_title_and_thumbnail_on_a_fresh_job() {
        let mut reg = JobRegistry::new();
        let id = reg.insert(sample_job());

        reg.set_metadata(&id, "Real Video Title", "https://img/thumb.jpg");

        let job = reg.get(&id).unwrap();
        assert_eq!(job.title, "Real Video Title");
        assert_eq!(job.thumbnail, "https://img/thumb.jpg");
    }

    #[test]
    fn a_failed_probe_leaves_an_empty_title_and_thumbnail_alone() {
        let mut reg = JobRegistry::new();
        let id = reg.insert(sample_job());

        reg.set_metadata(&id, "", "");

        let job = reg.get(&id).unwrap();
        assert_eq!(job.title, "", "the UI falls back to the URL when this is empty");
        assert_eq!(job.thumbnail, "");
    }

    // The common case: the metadata probe usually resolves well before the
    // download's own output filename is known.
    #[test]
    fn metadata_arriving_before_the_output_path_is_not_overwritten_by_the_stem() {
        let mut reg = JobRegistry::new();
        let id = reg.insert(sample_job());

        reg.set_metadata(&id, "Real Video Title", "https://img/thumb.jpg");
        reg.set_output_path(&id, PathBuf::from("/out/Real Video Title.mp4"));

        assert_eq!(reg.get(&id).unwrap().title, "Real Video Title");
    }

    // The rare case this whole design exists for: a fast download finishes
    // before a slow metadata probe returns. The stem guess is a fine title
    // in the meantime, but the real title must still win once it lands.
    #[test]
    fn metadata_arriving_after_the_output_path_overwrites_the_stem_guess() {
        let mut reg = JobRegistry::new();
        let id = reg.insert(sample_job());

        reg.set_output_path(&id, PathBuf::from("/out/some_filename_stem.mp4"));
        assert_eq!(reg.get(&id).unwrap().title, "some_filename_stem");

        reg.set_metadata(&id, "Real Video Title", "https://img/thumb.jpg");

        let job = reg.get(&id).unwrap();
        assert_eq!(job.title, "Real Video Title");
        assert_eq!(job.thumbnail, "https://img/thumb.jpg");
    }

    // A probe that fails after the stem already set a title must not blank it
    // back out — empty is worse than a filename guess.
    #[test]
    fn a_late_failed_probe_does_not_erase_an_existing_stem_title() {
        let mut reg = JobRegistry::new();
        let id = reg.insert(sample_job());

        reg.set_output_path(&id, PathBuf::from("/out/some_filename_stem.mp4"));
        reg.set_metadata(&id, "", "");

        assert_eq!(reg.get(&id).unwrap().title, "some_filename_stem");
    }

    #[test]
    fn set_metadata_on_a_missing_job_is_a_no_op_not_a_panic() {
        let mut reg = JobRegistry::new();
        reg.set_metadata(&"nonexistent".to_string(), "Title", "https://img/t.jpg");
        assert_eq!(reg.list().len(), 0);
    }
}
