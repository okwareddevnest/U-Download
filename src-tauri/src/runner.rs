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
use crate::proc;
use crate::queue;
use crate::ytdlp::{build_download_args, parse_progress_line, DownloadSpec};
use std::io::Read;
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
    /// The JavaScript runtime yt-dlp should be told about, if this machine has
    /// one. `None` is a supported state: the download then runs with exactly
    /// the arguments it used before runtimes were wired in.
    pub js_runtime: Option<binary_manager::JsRuntime>,
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

/// Splits a byte stream into records on either `\n` or `\r`.
///
/// yt-dlp's own progress lines are newline-terminated — we pass `--newline` —
/// but they are not what an untrimmed job produces. yt-dlp hands those to
/// aria2c, and although it disables aria2c's periodic summary
/// (`--summary-interval=0`, `--download-result=hide`) it keeps the live
/// single-line readout (`--show-console-readout=true`). aria2c redraws that
/// readout with a carriage return and never terminates it with a newline, so a
/// line-oriented reader sees one enormous record at end of stream and the user
/// watches 0% until the download finishes.
///
/// Empty records are dropped. That is also what stops a `\r\n` pair from
/// producing a spurious blank record between its two bytes — and a blank
/// record is not something either parser could use anyway.
#[derive(Default)]
struct RecordSplitter {
    pending: Vec<u8>,
}

impl RecordSplitter {
    /// Feeds one chunk of bytes and returns every record it completed.
    ///
    /// A record straddling two chunks stays in `pending` until the delimiter
    /// arrives, so chunk boundaries are invisible to the caller.
    fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut records = Vec::new();
        for &byte in chunk {
            if byte == b'\n' || byte == b'\r' {
                if let Some(record) = self.take_pending() {
                    records.push(record);
                }
            } else {
                self.pending.push(byte);
            }
        }
        records
    }

    /// Takes whatever is buffered but not yet terminated.
    ///
    /// Called once at end of stream, where it is not an edge case: aria2c's
    /// last readout never gets a terminator, so this is how the final progress
    /// update arrives.
    fn take_pending(&mut self) -> Option<String> {
        if self.pending.is_empty() {
            return None;
        }
        let record = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        Some(record)
    }
}

/// Turns one parsed progress record into a percentage, or `None` if it carries
/// no usable one.
///
/// ffmpeg — the downloader on every trimmed job — reports a position on the
/// media timeline and no percentage at all, because it does not know how long
/// the requested section is. This function does: `section_seconds` is the span
/// of the job's own trim range. Without this the whole trim path reported 0%
/// until the runner force-wrote 100 at the end.
///
/// An ffmpeg position with no known section length yields `None` rather than
/// 0.0: an untrimmed job's post-download merge also prints these lines, and
/// writing 0% there would undo the real progress aria2c had already reported.
fn percentage_from(p: &crate::ytdlp::ProgressLine, section_seconds: Option<f64>) -> Option<f64> {
    match p.out_time_seconds {
        None => Some(p.percentage),
        Some(position) => match section_seconds {
            Some(total) if total > 0.0 => Some((position / total * 100.0).clamp(0.0, 100.0)),
            _ => None,
        },
    }
}

/// Applies one record of downloader output to the job.
///
/// Split out so the reader loop and the end-of-stream fragment go through
/// exactly the same path.
fn consume_record<R: Runtime>(
    window: &Window<R>,
    jobs: &SharedJobs,
    id: &JobId,
    record: &str,
    output_path: &mut Option<String>,
    last_emit: &mut std::time::Instant,
    section_seconds: Option<f64>,
) {
    if let Some(path) = destination_from_line(record) {
        *output_path = Some(path);
        return;
    }

    let Some(p) = parse_progress_line(record) else {
        return;
    };

    let Some(percentage) = percentage_from(&p, section_seconds) else {
        return;
    };

    let total_bytes = p.total_bytes.unwrap_or(0);
    {
        // Lock scope: one map write, once per progress record.
        jobs.lock().unwrap().update_progress(
            id,
            JobProgress {
                percentage,
                speed_bytes_per_sec: p.speed_bytes_per_sec.unwrap_or(0),
                eta_seconds: p.eta_seconds,
                bytes_downloaded: (total_bytes as f64 * percentage / 100.0) as u64,
                total_bytes,
            },
        );
    }

    let due = last_emit.elapsed().as_millis() >= PROGRESS_EMIT_INTERVAL_MS;
    if due || percentage >= 100.0 {
        emit_job(window, jobs, id);
        *last_emit = std::time::Instant::now();
    }
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
    // Recent yt-dlp cannot resolve a YouTube format without a JS runtime, so
    // without this the selectors above fail as "Requested format is not
    // available"; `None` reproduces the pre-runtime command line exactly.
    let js_runtime = ctx.js_runtime.as_ref().map(|rt| rt.flag_value());
    let args = build_download_args(
        &spec,
        &ctx.ffmpeg.to_string_lossy(),
        js_runtime.as_deref(),
    );

    // The span of the requested section, which is what ffmpeg's timeline
    // progress has to be measured against on a trimmed job. `None` for an
    // untrimmed one, where progress is byte-based and this is not consulted.
    let section_seconds = spec
        .trim
        .map(|range| range.end - range.start)
        .filter(|span| *span > 0.0);

    let mut cmd = Command::new(&ctx.yt_dlp);
    binary_manager::augment_path_env(&mut cmd, &ctx.binaries_dir);
    cmd.args(&args).stdout(Stdio::piped()).stderr(Stdio::piped());
    // So that cancelling reaches the process actually doing the download —
    // ffmpeg on a trimmed job, aria2c otherwise — and not just yt-dlp.
    proc::spawn_in_own_process_group(&mut cmd);

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
    //
    // It is also where a trimmed job's progress lives. ffmpeg writes its
    // `frame=… time=…` readout to stderr, not stdout, and on the
    // `--download-sections` path ffmpeg is the only thing reporting anything —
    // so parsing the ffmpeg line shape without also routing stderr through the
    // parser would leave the trim path exactly as blind as before. The same
    // `RecordSplitter` is used because ffmpeg redraws that readout with a
    // carriage return, just as aria2c does.
    let stderr_reader = stderr.map(|mut stderr| {
        let (window, jobs, id) = (window.clone(), jobs.clone(), id.clone());
        std::thread::spawn(move || {
            let mut raw: Vec<u8> = Vec::new();
            let mut splitter = RecordSplitter::default();
            let mut chunk = [0u8; 4096];
            let mut last_emit = std::time::Instant::now();
            // yt-dlp names its output file on stdout, never here; this exists
            // only to satisfy the shared signature and is intentionally dropped.
            let mut ignored_path: Option<String> = None;

            loop {
                let read = match stderr.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => read,
                };
                raw.extend_from_slice(&chunk[..read]);
                for record in splitter.push(&chunk[..read]) {
                    consume_record(
                        &window,
                        &jobs,
                        &id,
                        &record,
                        &mut ignored_path,
                        &mut last_emit,
                        section_seconds,
                    );
                }
            }

            // ffmpeg's last readout, like aria2c's, has no terminator.
            if let Some(record) = splitter.take_pending() {
                consume_record(
                    &window,
                    &jobs,
                    &id,
                    &record,
                    &mut ignored_path,
                    &mut last_emit,
                    section_seconds,
                );
            }

            // Decoded once at the end rather than per chunk, so a multi-byte
            // character straddling a chunk boundary is not mangled in the text
            // that becomes the user-visible error message.
            String::from_utf8_lossy(&raw).into_owned()
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
        proc::kill_tree(&mut child);
        let _ = child.wait();
        return;
    }

    emit_job(&window, &jobs, &id);

    let mut output_path: Option<String> = None;

    if let Some(mut stdout) = stdout {
        // Read raw bytes rather than `BufReader::lines()`. aria2c's live
        // readout is the only progress an untrimmed job produces, and aria2c
        // redraws it with a carriage return and never terminates it with a
        // newline — `lines()` would yield nothing at all until the pipe closed
        // and then replay every update at once, leaving the bar at 0% for the
        // whole download.
        let mut splitter = RecordSplitter::default();
        let mut chunk = [0u8; 4096];
        let mut last_emit = std::time::Instant::now();

        loop {
            let read = match stdout.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            for record in splitter.push(&chunk[..read]) {
                consume_record(
                    &window,
                    &jobs,
                    &id,
                    &record,
                    &mut output_path,
                    &mut last_emit,
                    section_seconds,
                );
            }
        }

        // aria2c's final readout has no terminator at all, so the last
        // progress update only exists as this fragment.
        if let Some(record) = splitter.take_pending() {
            consume_record(
                &window,
                &jobs,
                &id,
                &record,
                &mut output_path,
                &mut last_emit,
                section_seconds,
            );
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
    use super::{destination_from_line, percentage_from, RecordSplitter};
    use crate::ytdlp::parse_progress_line;

    /// Feeds the whole stream as one chunk and closes it.
    fn split_all(stream: &[u8]) -> Vec<String> {
        let mut splitter = RecordSplitter::default();
        let mut records = splitter.push(stream);
        records.extend(splitter.take_pending());
        records
    }

    /// Feeds the stream in the given pieces, so chunk boundaries fall wherever
    /// the test wants them.
    fn split_chunks(chunks: &[&[u8]]) -> Vec<String> {
        let mut splitter = RecordSplitter::default();
        let mut records = Vec::new();
        for chunk in chunks {
            records.extend(splitter.push(chunk));
        }
        records.extend(splitter.take_pending());
        records
    }

    // aria2c's readout is redrawn with `\r` and never gets a `\n`.
    #[test]
    fn splits_records_on_carriage_returns_alone() {
        assert_eq!(split_all(b"one\rtwo\rthree\r"), vec!["one", "two", "three"]);
    }

    #[test]
    fn splits_records_on_newlines_alone() {
        assert_eq!(split_all(b"one\ntwo\nthree\n"), vec!["one", "two", "three"]);
    }

    #[test]
    fn treats_crlf_as_one_separator() {
        assert_eq!(split_all(b"one\r\ntwo\r\n"), vec!["one", "two"]);
    }

    #[test]
    fn joins_a_record_split_across_chunk_boundaries() {
        assert_eq!(
            split_chunks(&[b"one\rtw", b"o\rthr", b"ee\r"]),
            vec!["one", "two", "three"]
        );
    }

    // A `\r` ending one chunk and the matching `\n` opening the next must not
    // manufacture a blank record between them.
    #[test]
    fn a_crlf_straddling_a_chunk_boundary_is_still_one_separator() {
        assert_eq!(split_chunks(&[b"one\r", b"\ntwo\r\n"]), vec!["one", "two"]);
    }

    // This is how the final progress update arrives — aria2c never terminates
    // its last readout.
    #[test]
    fn returns_the_unterminated_trailing_fragment_at_eof() {
        assert_eq!(split_all(b"one\rtwo\rlast"), vec!["one", "two", "last"]);
    }

    #[test]
    fn produces_nothing_from_an_empty_or_blank_stream() {
        assert!(split_all(b"").is_empty());
        assert!(split_all(b"\r\n\r\n\n\r").is_empty());
    }

    // End to end over the splitter: a realistic aria2c readout stream, redrawn
    // with carriage returns and cut off mid-record at EOF, must yield every
    // percentage the user should have seen.
    #[test]
    fn an_aria2c_readout_stream_yields_every_update() {
        let stream: &[&[u8]] = &[
            b"[#f1a2b3 1.0MiB/10MiB(10%) CN:16 DL:1.0MiB ETA:9s]\r[#f1a2b3 5.0MiB/10M",
            b"iB(50%) CN:16 DL:1.0MiB ETA:5s]\r[#f1a2b3 10MiB/10MiB(100%) CN:16 DL:1.0MiB]",
        ];

        let percentages: Vec<f64> = split_chunks(stream)
            .iter()
            .filter_map(|record| parse_progress_line(record))
            .map(|p| p.percentage)
            .collect();

        assert_eq!(percentages, vec![10.0, 50.0, 100.0]);
    }

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

    fn pct_of(record: &str, section_seconds: Option<f64>) -> Option<f64> {
        let parsed = parse_progress_line(record)?;
        percentage_from(&parsed, section_seconds)
    }

    // The whole of FIX 2: ffmpeg reports a timeline position, the runner owns
    // the section length, and only together do they make a percentage.
    #[test]
    fn an_ffmpeg_position_becomes_a_percentage_of_the_requested_section() {
        let line = "frame=  150 fps= 36 q=32.0 Lsize=  358KiB time=00:00:05.00 bitrate= 586.2kbits/s speed=1.21x";
        assert_eq!(pct_of(line, Some(20.0)), Some(25.0));
    }

    // ffmpeg can overshoot the requested end slightly when it re-encodes the
    // boundary GOP; a bar past 100% would be a visible defect.
    #[test]
    fn an_ffmpeg_overshoot_is_clamped_to_one_hundred() {
        let line = "frame=  150 fps= 36 q=32.0 size=  358KiB time=00:00:21.00 bitrate=N/A speed=1.21x";
        assert_eq!(pct_of(line, Some(20.0)), Some(100.0));
    }

    // An untrimmed job's merge pass also prints ffmpeg readouts. With no
    // section length there is no percentage to compute, and writing 0 would
    // wipe out the real progress aria2c had already reported.
    #[test]
    fn an_ffmpeg_line_on_an_untrimmed_job_reports_nothing_rather_than_zero() {
        let line = "frame=  150 fps= 36 q=32.0 size=  358KiB time=00:00:05.00 bitrate=N/A speed=1.21x";
        assert_eq!(pct_of(line, None), None);
    }

    // Regression guard: byte-based progress must be passed through untouched,
    // whether or not a section length happens to be known.
    #[test]
    fn byte_based_progress_is_unaffected_by_the_section_length() {
        let ytdlp = "[download]  42.5% of  120.50MiB at   3.20MiB/s ETA 00:22";
        assert_eq!(pct_of(ytdlp, None), Some(42.5));
        assert_eq!(pct_of(ytdlp, Some(20.0)), Some(42.5));

        let aria = "[#f1a2b3 1.2MiB/10MiB(12%) CN:16 DL:2.1MiB ETA:4s]";
        assert_eq!(pct_of(aria, None), Some(12.0));
        assert_eq!(pct_of(aria, Some(20.0)), Some(12.0));
    }

    // End to end over the splitter, as for aria2c: ffmpeg redraws its readout
    // with carriage returns and leaves the last one unterminated, so every
    // update must survive that on the way to a percentage.
    #[test]
    fn an_ffmpeg_readout_stream_yields_every_update() {
        let stream: &[&[u8]] = &[
            b"frame=   0 fps=0.0 q=0.0 size=N/A time=N/A bitrate=N/A speed=N/A\rframe=  25 fps=25 q=28.0 size=  64KiB time=00:00:0",
            b"1.00 bitrate=N/A speed=1.0x\rframe= 250 fps=25 q=-1.0 Lsize= 640KiB time=00:00:10.00 bitrate=N/A speed=1.0x",
        ];

        let percentages: Vec<Option<f64>> = split_chunks(stream)
            .iter()
            .filter_map(|record| parse_progress_line(record))
            .map(|p| percentage_from(&p, Some(10.0)))
            .collect();

        // The `time=N/A` record parses to nothing at all and never reaches here.
        assert_eq!(percentages, vec![Some(10.0), Some(100.0)]);
    }
}
