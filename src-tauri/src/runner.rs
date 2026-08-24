//! Executes queued jobs as yt-dlp child processes and reports their progress.
//!
//! Two invariants govern everything here:
//!
//! 1. **The registry mutex is never held across a blocking or slow call.**
//!    `SharedJobs` is a single `Mutex` shared by every download thread, the
//!    scheduler and all five Tauri commands. Holding it across `Child::wait`,
//!    `Command::spawn` or a webview `emit` would stall every other job's
//!    progress update. Each lock scope below is a few map lookups, and the
//!    comment on it says what it covers.
//! 2. **A job is claimed before its thread starts.** See `pump`.

use crate::binary_manager;
use crate::jobs::{JobId, JobProgress, JobStatus, SharedJobs};
use crate::queue;
use crate::ytdlp::{build_download_args, parse_progress_line, DownloadSpec};
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tauri::{Emitter, Runtime, Window};

/// Emits at most one progress event per job per interval, so several concurrent
/// downloads cannot flood the webview bridge.
const PROGRESS_EMIT_INTERVAL_MS: u128 = 500;

/// The per-run inputs that do not live in the job registry.
///
/// Bundled as one value because `pump` and `run_job` hand them to each other
/// recursively; five loose `PathBuf`/`u32` arguments were easy to transpose.
#[derive(Debug, Clone)]
pub struct RunnerContext {
    pub yt_dlp: PathBuf,
    pub ffmpeg: PathBuf,
    pub binaries_dir: PathBuf,
    pub concurrency: u32,
}

/// Sends the current state of one job to the frontend.
///
/// The lock is dropped before `emit`: serialising into the webview bridge is
/// not something the registry mutex should ever be held across.
pub fn emit_job<R: Runtime>(window: &Window<R>, jobs: &SharedJobs, id: &JobId) {
    let job = { jobs.lock().unwrap().get(id) };
    if let Some(job) = job {
        let _ = window.emit("job-updated", job);
    }
}

/// Marks a job failed and tells the frontend, in that order.
fn fail<R: Runtime>(window: &Window<R>, jobs: &SharedJobs, id: &JobId, message: String) {
    {
        // Lock scope: one map write.
        jobs.lock().unwrap().set_error(id, message.clone());
    }
    emit_job(window, jobs, id);
    let _ = window.emit(
        "job-failed",
        serde_json::json!({ "job_id": id, "error": message }),
    );
}

/// Recognises the lines on which yt-dlp names the file it is writing.
///
/// Reading yt-dlp's own report is what replaces the previous implementation's
/// directory scan for a name containing `_temp`, which could match — and then
/// overwrite — an unrelated file. Later lines win: for a merged download the
/// per-stream `Destination:` lines come first and `[Merger]` names the real
/// result, and for audio extraction `[ExtractAudio]` follows the container
/// download the same way.
fn destination_from_line(line: &str) -> Option<String> {
    for prefix in [
        "[download] Destination: ",
        "[ExtractAudio] Destination: ",
        "[Merger] Merging formats into ",
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let path = rest.trim().trim_matches('"');
            if !path.is_empty() {
                return Some(path.to_string());
            }
        }
    }
    None
}

/// Runs one job to completion on a blocking thread.
///
/// The caller (`pump`) has already moved the job out of `Queued`, so this
/// function owns the concurrency slot from its first line.
pub fn run_job<R: Runtime>(window: Window<R>, jobs: SharedJobs, id: JobId, ctx: RunnerContext) {
    // Lock scope: copy the job's inputs out. Everything below works from the
    // copy, so the registry stays free while the download runs.
    let job = match { jobs.lock().unwrap().get(&id) } {
        // Anything other than the claim `pump` just made means the job was
        // cancelled or paused between dispatch and this thread starting.
        Some(job) if job.status == JobStatus::Probing => job,
        _ => return,
    };

    let spec = DownloadSpec {
        url: job.url.clone(),
        format: job.format.clone(),
        trim: job.trim,
        output_template: format!("{}/%(title)s.%(ext)s", job.output_folder),
        concurrency: ctx.concurrency,
    };

    // Trimming lives entirely in these arguments (`--download-sections` plus
    // `--force-keyframes-at-cuts`). There is deliberately no FFmpeg post-pass:
    // the `-c copy` cut it used to perform could only land on a keyframe.
    let args = build_download_args(&spec, &ctx.ffmpeg.to_string_lossy());

    let mut cmd = Command::new(&ctx.yt_dlp);
    binary_manager::augment_path_env(&mut cmd, &ctx.binaries_dir);
    cmd.args(&args).stdout(Stdio::piped()).stderr(Stdio::piped());

    // Spawned with no lock held.
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            fail(&window, &jobs, &id, format!("Failed to start yt-dlp: {e}"));
            return;
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Drained on its own thread. Reading stderr only after stdout closes would
    // deadlock the moment yt-dlp writes more warnings than the pipe buffer
    // holds, because it would then block before finishing its stdout output.
    let stderr_reader = stderr.map(|mut stderr| {
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        })
    });

    // Lock scope: one read and two writes. The promotion is conditional
    // because `cancel_job`/`pause_job` can land in the window that spans
    // `cmd.spawn()` and this point — milliseconds, not microseconds. Such a
    // caller finds no child to kill (it has not been attached yet) and writes
    // only its own terminal status; promoting unconditionally would erase it
    // and the download would run to completion and report Done, with the
    // user's cancel silently discarded.
    let orphan = {
        let mut reg = jobs.lock().unwrap();
        if queue::promote_to_downloading(&mut reg, &id) {
            // Attached in the same scope as the promotion, so no other caller
            // can observe a Downloading job with no cancellable process.
            reg.attach_child(&id, child);
            None
        } else {
            Some(child)
        }
    };

    if let Some(mut child) = orphan {
        // The job was taken from us before we could attach the process, so
        // nobody else can kill it. Killed and reaped here, outside the lock.
        // The status the other caller published stands, and it has already
        // emitted for it, so this thread reports nothing.
        let _ = child.kill();
        let _ = child.wait();
        return;
    }

    emit_job(&window, &jobs, &id);

    let mut output_path: Option<String> = None;

    if let Some(stdout) = stdout {
        let reader = BufReader::new(stdout);
        let mut last_emit = std::time::Instant::now();

        // aria2c redraws its readout with carriage returns rather than
        // newlines, so a single `\n`-delimited line can carry several updates.
        // Flattening them here keeps the body below one loop over readouts.
        let readouts = reader
            .lines()
            .map_while(Result::ok)
            .flat_map(|line| line.split('\r').map(str::to_owned).collect::<Vec<_>>());

        for line in readouts {
            if let Some(path) = destination_from_line(&line) {
                output_path = Some(path);
                continue;
            }

            let Some(p) = parse_progress_line(&line) else {
                continue;
            };

            let total_bytes = p.total_bytes.unwrap_or(0);
            {
                // Lock scope: one map write, once per progress line.
                jobs.lock().unwrap().update_progress(
                    &id,
                    JobProgress {
                        percentage: p.percentage,
                        speed_bytes_per_sec: p.speed_bytes_per_sec.unwrap_or(0),
                        eta_seconds: p.eta_seconds,
                        bytes_downloaded: (total_bytes as f64 * p.percentage / 100.0) as u64,
                        total_bytes,
                    },
                );
            }

            let due = last_emit.elapsed().as_millis() >= PROGRESS_EMIT_INTERVAL_MS;
            if due || p.percentage >= 100.0 {
                emit_job(&window, &jobs, &id);
                last_emit = std::time::Instant::now();
            }
        }
    }

    let stderr_text = stderr_reader
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();

    // Lock scope: one map write. Taking the child back out is what lets the
    // wait below happen with the mutex released. `None` means `cancel` got
    // there first and has already killed and reaped the process.
    let child = { jobs.lock().unwrap().take_child(&id) };
    let status = child.map(|mut child| child.wait());

    // Lock scope: one read. `queue::still_running` is false once a cancel or
    // pause has taken the job over and published its own terminal state; see
    // its doc comment for why this thread must then report nothing.
    let still_ours = { queue::still_running(&jobs.lock().unwrap(), &id) };
    if !still_ours {
        return;
    }

    match status {
        Some(Ok(exit)) if exit.success() => {
            let finished = {
                // Lock scope: the completion writes, then one read to build
                // the event payload.
                let mut reg = jobs.lock().unwrap();
                if let Some(path) = output_path {
                    reg.set_output_path(&id, PathBuf::from(path));
                }
                let mut progress = reg.get(&id).map(|job| job.progress).unwrap_or_default();
                progress.percentage = 100.0;
                progress.speed_bytes_per_sec = 0;
                progress.eta_seconds = Some(0);
                progress.bytes_downloaded = progress.total_bytes;
                reg.update_progress(&id, progress);
                reg.set_status(&id, JobStatus::Done);
                reg.get(&id)
            };

            if let Some(job) = finished {
                let _ = window.emit("job-updated", &job);
                let _ = window.emit(
                    "job-done",
                    serde_json::json!({
                        "job_id": id,
                        "output_path": job.output_path,
                        "title": job.title,
                    }),
                );
            }
        }
        Some(Ok(exit)) => {
            let code = exit.code().unwrap_or(-1);
            let detail = stderr_text.trim();
            let message = if detail.is_empty() {
                format!("yt-dlp exited with {code}")
            } else {
                format!("yt-dlp exited with {code}: {detail}")
            };
            fail(&window, &jobs, &id, message);
        }
        Some(Err(e)) => fail(&window, &jobs, &id, format!("Process error: {e}")),
        // The registry no longer held the child and the job is not cancelled:
        // the process was reaped by something else, so there is no status to
        // report. Treat it as a failure rather than silently claiming success.
        None => fail(
            &window,
            &jobs,
            &id,
            "yt-dlp process handle was lost before it could be waited on".to_string(),
        ),
    }
}

/// Starts every job the scheduler currently permits.
///
/// Called on enqueue, on cancel/pause/resume, and by each job thread as it
/// finishes — so two `pump`s can easily run at once.
pub fn pump<R: Runtime>(window: Window<R>, jobs: SharedJobs, ctx: RunnerContext) {
    // Lock scope: the dispatch decision AND the claim that follows from it,
    // which `queue::claim_next` performs as one operation precisely so they
    // cannot be split. Nothing is spawned inside the scope.
    let ready = {
        let mut reg = jobs.lock().unwrap();
        queue::claim_next(&mut reg, ctx.concurrency.max(1) as usize)
    };

    for id in ready {
        emit_job(&window, &jobs, &id);
        let (window, jobs, ctx) = (window.clone(), jobs.clone(), ctx.clone());
        std::thread::spawn(move || {
            run_job(window.clone(), jobs.clone(), id, ctx.clone());
            // A finished job frees a slot; start whatever is next.
            pump(window, jobs, ctx);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::destination_from_line;

    #[test]
    fn reads_the_plain_download_destination() {
        assert_eq!(
            destination_from_line("[download] Destination: /out/Some Video.f137.mp4").as_deref(),
            Some("/out/Some Video.f137.mp4")
        );
    }

    #[test]
    fn reads_the_merged_destination_without_its_quotes() {
        assert_eq!(
            destination_from_line("[Merger] Merging formats into \"/out/Some Video.mp4\"")
                .as_deref(),
            Some("/out/Some Video.mp4")
        );
    }

    #[test]
    fn reads_the_extracted_audio_destination() {
        assert_eq!(
            destination_from_line("[ExtractAudio] Destination: /out/Some Track.mp3").as_deref(),
            Some("/out/Some Track.mp3")
        );
    }

    #[test]
    fn ignores_progress_and_other_chatter() {
        assert!(destination_from_line("[download]  42.0% of 10.00MiB at 1.00MiB/s").is_none());
        assert!(destination_from_line("[youtube] Extracting URL: https://e/v").is_none());
    }
}
