use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct TrimRange {
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Mp4,
    Mp3,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum FormatChoice {
    Quick { kind: MediaKind, height: Option<u32> },
    Exact { format_id: String },
}

#[derive(Debug, Clone)]
pub struct DownloadSpec {
    pub url: String,
    pub format: FormatChoice,
    pub trim: Option<TrimRange>,
    pub output_template: String,
    pub concurrency: u32,
}

/// aria2c splits the link N ways per download. As more downloads run at once,
/// each gets proportionally fewer connections so the link is not saturated.
pub fn aria2c_connections(concurrency: u32) -> u32 {
    let c = concurrency.max(1);
    (16 / c).clamp(4, 16)
}

/// Builds the full yt-dlp argument vector for one job.
///
/// Trimming is performed by yt-dlp itself via `--download-sections`, never by a
/// post-hoc FFmpeg stream copy. `-c copy` cannot cut on a non-keyframe, so it
/// silently snaps the cut to the nearest keyframe — the cause of the off-target
/// output this replaces. `--force-keyframes-at-cuts` re-encodes only the
/// boundary GOPs, keeping the cut exact and the interior fast.
/// `js_runtime` is the `<name>:<path>` value for yt-dlp's `--js-runtimes`, or
/// `None` when the machine has no JavaScript runtime. Recent yt-dlp needs one
/// to extract YouTube formats — without it every selector here fails as
/// "Requested format is not available" — but a missing runtime must still
/// produce a working command line, so `None` adds no argument at all.
///
/// It is a parameter rather than a `DownloadSpec` field for the same reason
/// `ffmpeg_path` is: both are properties of the installation the runner
/// resolved, not of the job the user queued.
pub fn build_download_args(
    spec: &DownloadSpec,
    ffmpeg_path: &str,
    js_runtime: Option<&str>,
) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    let trimming = spec.trim.is_some();

    if let Some(runtime) = js_runtime {
        args.push("--js-runtimes".into());
        args.push(runtime.to_string());
    }

    args.push("--progress".into());
    args.push("--newline".into());
    args.push("--no-playlist".into());
    args.push("--ffmpeg-location".into());
    args.push(ffmpeg_path.into());

    // An external downloader cannot serve ranged section requests, so trimmed
    // jobs fall back to yt-dlp's native ranged fetch.
    if !trimming {
        let conns = aria2c_connections(spec.concurrency);
        args.push("--external-downloader".into());
        args.push("aria2c".into());
        args.push("--external-downloader-args".into());
        args.push(format!("-x {} -s {} -k 1M", conns, conns));
    }

    match &spec.format {
        FormatChoice::Quick { kind: MediaKind::Mp3, .. } => {
            args.push("-x".into());
            args.push("--audio-format".into());
            args.push("mp3".into());
            args.push("--audio-quality".into());
            args.push("192K".into());
        }
        FormatChoice::Quick { kind: MediaKind::Mp4, height } => {
            args.push("--merge-output-format".into());
            args.push("mp4".into());
            args.push("-f".into());
            args.push(match height {
                Some(h) => format!("bestvideo[height<={h}]+bestaudio/best[height<={h}]"),
                None => "bestvideo+bestaudio/best".into(),
            });
        }
        FormatChoice::Exact { format_id } => {
            args.push("--merge-output-format".into());
            args.push("mp4".into());
            args.push("-f".into());
            // Pairing with bestaudio guards against a video-only id producing a
            // silent file; the bare id is kept as fallback for muxed formats.
            args.push(format!("{format_id}+bestaudio/{format_id}"));
        }
    }

    if let Some(range) = spec.trim {
        args.push("--download-sections".into());
        args.push(format!(
            "*{}-{}",
            format_section_timestamp(range.start),
            format_section_timestamp(range.end)
        ));
        args.push("--force-keyframes-at-cuts".into());
    }

    args.push("-o".into());
    args.push(spec.output_template.clone());
    args.push(spec.url.clone());

    args
}

use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProgressLine {
    pub percentage: f64,
    pub total_bytes: Option<u64>,
    pub speed_bytes_per_sec: Option<u64>,
    pub eta_seconds: Option<u64>,
    /// Media position reached, in seconds, for downloaders that report a
    /// timeline rather than a byte count.
    ///
    /// Only ffmpeg fills this in. ffmpeg never prints a percentage — it does
    /// not know the length of what it is fetching — so the percentage has to
    /// be derived by the caller, which is the only party that knows the
    /// requested section's duration. When this is `Some`, `percentage` is 0.0
    /// and means nothing; the caller must use this field instead.
    pub out_time_seconds: Option<f64>,
}

fn unit_multiplier(unit: &str) -> f64 {
    match unit.to_ascii_uppercase().as_str() {
        "KIB" | "KB" | "K" => 1024.0,
        "MIB" | "MB" | "M" => 1024.0 * 1024.0,
        "GIB" | "GB" | "G" => 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    }
}

fn parse_size(value: &str, unit: &str) -> Option<u64> {
    value.parse::<f64>().ok().map(|v| (v * unit_multiplier(unit)) as u64)
}

/// Parses `HH:MM:SS` or `MM:SS` into seconds. Returns None for "Unknown".
fn parse_clock(value: &str) -> Option<u64> {
    let parts: Vec<&str> = value.split(':').collect();
    let nums: Option<Vec<u64>> = parts.iter().map(|p| p.parse::<u64>().ok()).collect();
    let nums = nums?;
    match nums.len() {
        2 => Some(nums[0] * 60 + nums[1]),
        3 => Some(nums[0] * 3600 + nums[1] * 60 + nums[2]),
        _ => None,
    }
}

/// Parses one `--newline` progress line from yt-dlp stdout.
///
/// The four patterns are constant string literals compiled once per process
/// via `OnceLock` rather than once per call — this function runs on the hot
/// path (once per stdout line of an active download). A bad pattern here
/// would be a programming error, so it panics loudly at first use instead of
/// silently disabling progress reporting for the rest of the job.
pub fn parse_progress_line(line: &str) -> Option<ProgressLine> {
    // While an external downloader is running, yt-dlp emits no `[download] N%`
    // lines of its own — aria2c's console readout is passed through instead.
    // Every untrimmed job takes that path, so without this branch the common
    // case would report no progress at all until it finished.
    let trimmed = line.trim();
    if trimmed.starts_with("[#") {
        return parse_aria2c_line(trimmed);
    }

    // Every *trimmed* job takes this path. `--download-sections` makes yt-dlp
    // fetch through ffmpeg instead of the external downloader, so neither
    // branch above nor the `[download]` branch below ever matches and the job
    // would sit at 0% for its whole duration.
    if trimmed.starts_with("frame=") || trimmed.starts_with("size=") {
        return parse_ffmpeg_line(trimmed);
    }

    if !line.starts_with("[download]") {
        return None;
    }

    static PCT_RE: OnceLock<Regex> = OnceLock::new();
    let pct_re = PCT_RE.get_or_init(|| Regex::new(r"(\d+(?:\.\d+)?)%").unwrap());
    let percentage = pct_re
        .captures(line)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<f64>().ok())?;

    static SIZE_RE: OnceLock<Regex> = OnceLock::new();
    let size_re = SIZE_RE.get_or_init(|| Regex::new(r"of\s+~?\s*(\d+(?:\.\d+)?)([KMG]i?B)").unwrap());
    let total_bytes = size_re
        .captures(line)
        .and_then(|c| parse_size(c.get(1)?.as_str(), c.get(2)?.as_str()));

    static SPEED_RE: OnceLock<Regex> = OnceLock::new();
    let speed_re = SPEED_RE.get_or_init(|| Regex::new(r"at\s+(\d+(?:\.\d+)?)([KMG]i?B)/s").unwrap());
    let speed_bytes_per_sec = speed_re
        .captures(line)
        .and_then(|c| parse_size(c.get(1)?.as_str(), c.get(2)?.as_str()));

    static ETA_RE: OnceLock<Regex> = OnceLock::new();
    let eta_re = ETA_RE.get_or_init(|| Regex::new(r"ETA\s+(\d{1,2}(?::\d{2})+)").unwrap());
    let eta_seconds = eta_re
        .captures(line)
        .and_then(|c| c.get(1))
        .and_then(|m| parse_clock(m.as_str()));

    Some(ProgressLine {
        percentage,
        total_bytes,
        speed_bytes_per_sec,
        eta_seconds,
        out_time_seconds: None,
    })
}

/// Parses one aria2c console-readout line, of the shape
///
/// ```text
/// [#f1a2b3 1.2MiB/10MiB(12%) CN:16 DL:2.1MiB ETA:4s]
/// ```
///
/// onto the same `ProgressLine` the `[download]` parser produces, so the runner
/// does not care which downloader is driving. `CN:` (connection count) has no
/// field to land in and is ignored. Lines without a percentage — aria2c prints
/// `[#f1a2b3 0B/0B CN:1 DL:0B]` before the size is known — yield `None` rather
/// than a bogus 0%.
fn parse_aria2c_line(line: &str) -> Option<ProgressLine> {
    static ARIA_PCT_RE: OnceLock<Regex> = OnceLock::new();
    let pct_re = ARIA_PCT_RE.get_or_init(|| Regex::new(r"\((\d+(?:\.\d+)?)%\)").unwrap());
    let percentage = pct_re
        .captures(line)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<f64>().ok())?;

    // "1.2MiB/10MiB(12%)" — the figure after the slash is the total.
    static ARIA_SIZE_RE: OnceLock<Regex> = OnceLock::new();
    let size_re = ARIA_SIZE_RE
        .get_or_init(|| Regex::new(r"/(\d+(?:\.\d+)?)([KMG]?i?B)\(").unwrap());
    let total_bytes = size_re
        .captures(line)
        .and_then(|c| parse_size(c.get(1)?.as_str(), c.get(2)?.as_str()));

    // aria2c reports DL: in bytes per second without saying so.
    static ARIA_SPEED_RE: OnceLock<Regex> = OnceLock::new();
    let speed_re = ARIA_SPEED_RE
        .get_or_init(|| Regex::new(r"DL:(\d+(?:\.\d+)?)([KMG]?i?B)").unwrap());
    let speed_bytes_per_sec = speed_re
        .captures(line)
        .and_then(|c| parse_size(c.get(1)?.as_str(), c.get(2)?.as_str()));

    static ARIA_ETA_RE: OnceLock<Regex> = OnceLock::new();
    let eta_re = ARIA_ETA_RE.get_or_init(|| Regex::new(r"ETA:([0-9hms]+)").unwrap());
    let eta_seconds = eta_re
        .captures(line)
        .and_then(|c| c.get(1))
        .and_then(|m| parse_aria2c_eta(m.as_str()));

    Some(ProgressLine {
        percentage,
        total_bytes,
        speed_bytes_per_sec,
        eta_seconds,
        out_time_seconds: None,
    })
}

/// Parses aria2c's compact duration (`4s`, `1m4s`, `1h2m3s`) into seconds.
fn parse_aria2c_eta(token: &str) -> Option<u64> {
    let mut total: u64 = 0;
    let mut digits = String::new();
    let mut saw_unit = false;

    for ch in token.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
            continue;
        }
        let value = digits.parse::<u64>().ok()?;
        digits.clear();
        total += match ch {
            'h' => value.checked_mul(3600)?,
            'm' => value.checked_mul(60)?,
            's' => value,
            _ => return None,
        };
        saw_unit = true;
    }

    // A trailing bare number ("4") is not a duration aria2c would print.
    if !digits.is_empty() || !saw_unit {
        return None;
    }
    Some(total)
}

/// Parses one ffmpeg status readout, of the shape
///
/// ```text
/// frame=  150 fps= 36 q=32.0 Lsize=     358KiB time=00:00:05.00 bitrate= 586.2kbits/s speed=1.21x
/// size=     358KiB time=00:00:05.00 bitrate= 586.2kbits/s speed=1.21x
/// ```
///
/// This is the *only* progress a trimmed job produces: `--download-sections`
/// makes yt-dlp fetch through ffmpeg rather than the external downloader, and
/// yt-dlp prints no percentage of its own while ffmpeg runs.
///
/// `time=` is the one field worth having. ffmpeg reports a position on the
/// media timeline, not a fraction — it has no idea how long the section is —
/// so this returns the position and leaves `percentage` at 0.0 for the runner
/// to fill in from the job's own trim range. `size=` is bytes written so far,
/// not a total, and `speed=` is a realtime ratio rather than bytes per second;
/// neither maps onto the byte fields, so both are deliberately dropped instead
/// of being reported as something they are not.
///
/// ffmpeg really does print `time=N/A` — it appears in live output before the
/// first frame lands. That yields `None`, matching the aria2c startup line:
/// a 0% update would overwrite a real one.
fn parse_ffmpeg_line(line: &str) -> Option<ProgressLine> {
    static FFMPEG_TIME_RE: OnceLock<Regex> = OnceLock::new();
    let time_re = FFMPEG_TIME_RE
        .get_or_init(|| Regex::new(r"time=\s*(-?)(\d+):(\d{2}):(\d{2}(?:\.\d+)?)").unwrap());

    let caps = time_re.captures(line)?;
    let hours: f64 = caps.get(2)?.as_str().parse().ok()?;
    let minutes: f64 = caps.get(3)?.as_str().parse().ok()?;
    let seconds: f64 = caps.get(4)?.as_str().parse().ok()?;

    let mut total = hours * 3600.0 + minutes * 60.0 + seconds;
    // ffmpeg can emit a small negative position while it primes its buffers.
    // Clamped rather than rejected: the readout is genuine progress output.
    if caps.get(1).map(|m| m.as_str() == "-").unwrap_or(false) {
        total = 0.0;
    }

    Some(ProgressLine {
        percentage: 0.0,
        total_bytes: None,
        speed_bytes_per_sec: None,
        eta_seconds: None,
        out_time_seconds: Some(total),
    })
}

/// Fallback trim for extractors that do not support `--download-sections`.
///
/// `-ss` is placed *before* `-i` so ffmpeg seeks by index instead of decoding
/// from the start, and the streams are genuinely re-encoded. Stream copy is
/// deliberately absent: it can only cut on keyframes.
// Not called yet: the runner trims exclusively through yt-dlp, and detecting
// "this extractor ignored --download-sections" is the follow-up that will
// reach for this. Kept (with its tests) rather than deleted so that follow-up
// does not have to re-derive the argument order that makes the cut exact.
#[allow(dead_code)]
pub fn build_ffmpeg_trim_args(
    input: &str,
    output: &str,
    range: TrimRange,
    audio_only: bool,
) -> Vec<String> {
    let duration = (range.end - range.start).max(0.0);

    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
        "-ss".into(),
        format_section_timestamp(range.start),
        "-i".into(),
        input.into(),
        "-t".into(),
        format_section_timestamp(duration),
    ];

    if audio_only {
        args.push("-c:a".into());
        args.push("libmp3lame".into());
    } else {
        args.push("-c:v".into());
        args.push("libx264".into());
        args.push("-preset".into());
        args.push("veryfast".into());
        args.push("-c:a".into());
        args.push("aac".into());
    }

    args.push(output.into());
    args
}

/// Formats a timestamp for yt-dlp's `--download-sections` argument.
///
/// yt-dlp accepts `HH:MM:SS.mmm`. Milliseconds are rounded, then carried into
/// seconds so that 9.9999 becomes `00:00:10.000` rather than an invalid
/// `00:00:09.1000`.
pub fn format_section_timestamp(seconds: f64) -> String {
    let clamped = if seconds.is_finite() && seconds > 0.0 { seconds } else { 0.0 };

    let total_millis = (clamped * 1000.0).round() as u64;
    let millis = total_millis % 1000;
    let total_secs = total_millis / 1000;

    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let secs = total_secs % 60;

    format!("{:02}:{:02}:{:02}.{:03}", hours, minutes, secs, millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_zero() {
        assert_eq!(format_section_timestamp(0.0), "00:00:00.000");
    }

    #[test]
    fn formats_whole_seconds() {
        assert_eq!(format_section_timestamp(65.0), "00:01:05.000");
    }

    #[test]
    fn formats_fractional_seconds() {
        assert_eq!(format_section_timestamp(5.25), "00:00:05.250");
    }

    #[test]
    fn formats_past_one_hour() {
        assert_eq!(format_section_timestamp(3661.5), "01:01:01.500");
    }

    #[test]
    fn clamps_negative_to_zero() {
        assert_eq!(format_section_timestamp(-3.0), "00:00:00.000");
    }

    #[test]
    fn rounds_milliseconds_without_overflowing_seconds() {
        // 9.9999 must not produce "00:00:09.1000"
        assert_eq!(format_section_timestamp(9.9999), "00:00:10.000");
    }

    fn spec_with_trim(trim: Option<TrimRange>) -> DownloadSpec {
        DownloadSpec {
            url: "https://example.com/v".to_string(),
            format: FormatChoice::Quick { kind: MediaKind::Mp4, height: Some(720) },
            trim,
            output_template: "/out/%(title)s.%(ext)s".to_string(),
            concurrency: 1,
        }
    }

    fn joined(args: &[String]) -> String {
        args.join(" ")
    }

    #[test]
    fn trimmed_job_requests_only_the_selected_section() {
        let spec = spec_with_trim(Some(TrimRange { start: 10.0, end: 20.0 }));
        let args = build_download_args(&spec, "/bin/ffmpeg", None);
        assert!(joined(&args).contains("--download-sections *00:00:10.000-00:00:20.000"));
    }

    #[test]
    fn trimmed_job_forces_keyframes_at_cuts() {
        let spec = spec_with_trim(Some(TrimRange { start: 10.0, end: 20.0 }));
        let args = build_download_args(&spec, "/bin/ffmpeg", None);
        assert!(args.contains(&"--force-keyframes-at-cuts".to_string()));
    }

    // Regression guard for spec section 2.1: `-c copy` forces keyframe snapping,
    // which is what made trimmed output land seconds away from the selection.
    #[test]
    fn no_trim_path_ever_uses_stream_copy() {
        for trim in [None, Some(TrimRange { start: 1.0, end: 2.0 })] {
            let spec = spec_with_trim(trim);
            let args = build_download_args(&spec, "/bin/ffmpeg", None);
            let text = joined(&args);
            assert!(!text.contains("-c copy"), "stream copy must never appear: {}", text);
            assert!(
                !args.iter().any(|a| a == "copy"),
                "stream copy must never appear on a trim path: {:?}", args
            );
        }
    }

    // Spec section 5.2: sections are incompatible with an external downloader.
    #[test]
    fn trimmed_job_omits_aria2c() {
        let spec = spec_with_trim(Some(TrimRange { start: 10.0, end: 20.0 }));
        let args = build_download_args(&spec, "/bin/ffmpeg", None);
        assert!(!args.contains(&"--external-downloader".to_string()));
        assert!(!joined(&args).contains("aria2c"));
    }

    #[test]
    fn untrimmed_job_keeps_aria2c_acceleration() {
        let spec = spec_with_trim(None);
        let args = build_download_args(&spec, "/bin/ffmpeg", None);
        assert!(args.contains(&"--external-downloader".to_string()));
        assert!(args.contains(&"aria2c".to_string()));
        assert!(!args.contains(&"--download-sections".to_string()));
    }

    #[test]
    fn mp3_trim_uses_sections_and_audio_extraction() {
        let spec = DownloadSpec {
            format: FormatChoice::Quick { kind: MediaKind::Mp3, height: None },
            ..spec_with_trim(Some(TrimRange { start: 3.0, end: 9.0 }))
        };
        let args = build_download_args(&spec, "/bin/ffmpeg", None);
        assert!(args.contains(&"-x".to_string()));
        assert!(joined(&args).contains("--audio-format mp3"));
        assert!(joined(&args).contains("--download-sections *00:00:03.000-00:00:09.000"));
    }

    #[test]
    fn exact_video_only_format_is_paired_with_best_audio() {
        let spec = DownloadSpec {
            format: FormatChoice::Exact { format_id: "137".to_string() },
            ..spec_with_trim(None)
        };
        let args = build_download_args(&spec, "/bin/ffmpeg", None);
        assert!(joined(&args).contains("-f 137+bestaudio/137"));
    }

    #[test]
    fn quick_height_maps_to_capped_format_selector() {
        let spec = spec_with_trim(None);
        let args = build_download_args(&spec, "/bin/ffmpeg", None);
        assert!(joined(&args).contains("bestvideo[height<=720]+bestaudio/best[height<=720]"));
    }

    #[test]
    fn aria2c_connections_scale_down_as_concurrency_rises() {
        assert_eq!(aria2c_connections(1), 16);
        assert_eq!(aria2c_connections(2), 8);
        assert_eq!(aria2c_connections(4), 4);
        assert_eq!(aria2c_connections(5), 4); // clamped at the floor
    }

    // Proves the element-based stream-copy guard actually catches what the
    // substring check misses: `-c:v copy` re-encodes audio while silently
    // stream-copying video, which still keyframe-snaps the cut. The old
    // `!text.contains("-c copy")` check does not see this because of the
    // colon; the element-based predicate must.
    #[test]
    fn stream_copy_guard_detects_scoped_copy_flag() {
        let hypothetical_bad_args: Vec<String> = vec!["-c:v".into(), "copy".into()];
        assert!(
            !hypothetical_bad_args.join(" ").contains("-c copy"),
            "sanity check: the colon must defeat the substring check"
        );
        assert!(
            hypothetical_bad_args.iter().any(|a| a == "copy"),
            "the element-based guard must flag a scoped copy flag that the substring check misses"
        );
    }

    fn index_of(args: &[String], needle: &str) -> usize {
        args.iter().position(|a| a == needle).expect("argument missing")
    }

    // Spec section 5.3: `-ss` after `-i` makes ffmpeg decode from zero and, with
    // stream copy, snap to a keyframe. Placement is load-bearing, not cosmetic.
    #[test]
    fn seek_flag_precedes_input_flag() {
        let args = build_ffmpeg_trim_args("in.mp4", "out.mp4", TrimRange { start: 10.0, end: 20.0 }, false);
        assert!(index_of(&args, "-ss") < index_of(&args, "-i"));
    }

    #[test]
    fn fallback_reencodes_rather_than_copying() {
        let args = build_ffmpeg_trim_args("in.mp4", "out.mp4", TrimRange { start: 10.0, end: 20.0 }, false);
        let text = args.join(" ");
        assert!(!text.contains("-c copy"));
        assert!(
            !args.iter().any(|a| a == "copy"),
            "stream copy must never appear on a trim path: {:?}", args
        );
        assert!(text.contains("-c:v libx264"));
        assert!(text.contains("-c:a aac"));
    }

    #[test]
    fn duration_is_the_span_not_the_end_timestamp() {
        let args = build_ffmpeg_trim_args("in.mp4", "out.mp4", TrimRange { start: 10.0, end: 25.5 }, false);
        let t = index_of(&args, "-t");
        assert_eq!(args[t + 1], "00:00:15.500");
    }

    #[test]
    fn audio_only_fallback_omits_video_encoder() {
        let args = build_ffmpeg_trim_args("in.mp3", "out.mp3", TrimRange { start: 1.0, end: 4.0 }, true);
        let text = args.join(" ");
        assert!(!text.contains("libx264"));
        assert!(text.contains("-c:a"));
    }

    #[test]
    fn inverted_range_yields_zero_duration_rather_than_negative() {
        let args = build_ffmpeg_trim_args("in.mp4", "out.mp4", TrimRange { start: 20.0, end: 10.0 }, false);
        let t = index_of(&args, "-t");
        assert_eq!(args[t + 1], "00:00:00.000");
    }

    #[test]
    fn parses_a_standard_ytdlp_progress_line() {
        let line = "[download]  42.5% of  120.50MiB at   3.20MiB/s ETA 00:22";
        let p = parse_progress_line(line).expect("should parse");
        assert!((p.percentage - 42.5).abs() < 0.01);
        assert_eq!(p.eta_seconds, Some(22));
        assert!(p.speed_bytes_per_sec.unwrap() > 3_000_000);
    }

    #[test]
    fn parses_eta_with_hours() {
        let line = "[download]   1.0% of  4.00GiB at 500.00KiB/s ETA 01:02:03";
        let p = parse_progress_line(line).expect("should parse");
        assert_eq!(p.eta_seconds, Some(3723));
    }

    #[test]
    fn parses_line_with_unknown_speed() {
        let line = "[download]   0.0% of ~ 10.00MiB at    Unknown B/s ETA Unknown";
        let p = parse_progress_line(line).expect("should parse");
        assert!((p.percentage - 0.0).abs() < 0.01);
        assert_eq!(p.speed_bytes_per_sec, None);
        assert_eq!(p.eta_seconds, None);
    }

    // Spec: an untrimmed job runs through aria2c, and yt-dlp then prints no
    // `[download] N%` lines of its own.
    #[test]
    fn parses_an_aria2c_readout_line() {
        let line = "[#f1a2b3 1.2MiB/10MiB(12%) CN:16 DL:2.1MiB ETA:4s]";
        let p = parse_progress_line(line).expect("should parse");
        assert_eq!(p.percentage, 12.0);
        assert_eq!(p.total_bytes, Some(10 * 1024 * 1024));
        assert_eq!(p.speed_bytes_per_sec, Some((2.1 * 1024.0 * 1024.0) as u64));
        assert_eq!(p.eta_seconds, Some(4));
    }

    #[test]
    fn aria2c_line_without_eta_still_reports_progress() {
        let line = "[#f1a2b3 1.2MiB/10MiB(12%) CN:16 DL:2.1MiB]";
        let p = parse_progress_line(line).expect("should parse");
        assert_eq!(p.percentage, 12.0);
        assert_eq!(p.total_bytes, Some(10 * 1024 * 1024));
        assert_eq!(p.speed_bytes_per_sec, Some((2.1 * 1024.0 * 1024.0) as u64));
        assert_eq!(p.eta_seconds, None);
    }

    #[test]
    fn parses_aria2c_eta_with_minutes_and_hours() {
        let minutes = parse_progress_line("[#a 1MiB/10MiB(10%) CN:8 DL:500KiB ETA:1m4s]")
            .expect("should parse");
        assert_eq!(minutes.eta_seconds, Some(64));

        let hours = parse_progress_line("[#a 1MiB/10MiB(10%) CN:8 DL:500KiB ETA:1h2m3s]")
            .expect("should parse");
        assert_eq!(hours.eta_seconds, Some(3723));
    }

    #[test]
    fn aria2c_speed_units_are_scaled() {
        let p = parse_progress_line("[#a 1MiB/10MiB(10%) CN:8 DL:512KiB ETA:9s]")
            .expect("should parse");
        assert_eq!(p.speed_bytes_per_sec, Some(512 * 1024));
    }

    // aria2c prints this before it knows the size; 0% would be a lie, and the
    // runner would emit a progress event that undoes a real one.
    #[test]
    fn ignores_aria2c_startup_line_with_no_percentage() {
        assert!(parse_progress_line("[#f1a2b3 0B/0B CN:1 DL:0B]").is_none());
    }

    #[test]
    fn ignores_non_progress_output() {
        assert!(parse_progress_line("[info] Downloading format 137").is_none());
        assert!(parse_progress_line("").is_none());
        assert!(parse_progress_line("[Merger] Merging formats into \"out.mp4\"").is_none());
    }

    #[test]
    fn parses_completed_line() {
        let line = "[download] 100% of  120.50MiB in 00:38";
        let p = parse_progress_line(line).expect("should parse");
        assert!((p.percentage - 100.0).abs() < 0.01);
    }

    // A trimmed job's only progress output. Without this branch every trimmed
    // download — the whole point of the feature — sat at 0% until it finished.
    #[test]
    fn parses_an_ffmpeg_status_line() {
        let line = "frame=  150 fps= 36 q=32.0 Lsize=     358KiB time=00:00:05.00 bitrate= 586.2kbits/s speed=1.21x";
        let p = parse_progress_line(line).expect("should parse");
        assert_eq!(p.out_time_seconds, Some(5.0));
        // The section length is unknown here, so the parser must not invent a
        // percentage; the runner derives it from the job's trim range.
        assert_eq!(p.percentage, 0.0);
    }

    // Captured verbatim from the bundled ffmpeg 9.0.1, trailing padding and
    // all. It carries a second clock field, `elapsed=`, which must not be
    // mistaken for the media position.
    #[test]
    fn parses_a_verbatim_line_from_the_bundled_ffmpeg() {
        let line = "frame=   26 fps=0.0 q=-0.0 size=N/A time=00:00:01.04 bitrate=N/A speed=2.06x elapsed=0:00:00.50    ";
        let p = parse_progress_line(line).expect("should parse");
        assert_eq!(p.out_time_seconds, Some(1.04));
    }

    #[test]
    fn parses_an_audio_only_ffmpeg_status_line() {
        let line = "size=     358KiB time=00:01:30.25 bitrate= 586.2kbits/s speed=1.21x";
        let p = parse_progress_line(line).expect("should parse");
        assert_eq!(p.out_time_seconds, Some(90.25));
    }

    #[test]
    fn parses_an_ffmpeg_position_past_one_hour() {
        let line = "frame=  150 fps= 36 q=32.0 size=N/A time=01:02:03.50 bitrate=N/A speed= 448x";
        let p = parse_progress_line(line).expect("should parse");
        assert_eq!(p.out_time_seconds, Some(3723.5));
    }

    // Seen in live output. A 0% update here would overwrite a real one, so
    // this yields nothing at all — the same rule the aria2c startup line follows.
    #[test]
    fn ignores_an_ffmpeg_line_with_an_unknown_time() {
        assert!(parse_progress_line("frame=    0 fps=0.0 q=0.0 size=N/A time=N/A bitrate=N/A speed=N/A").is_none());
        assert!(parse_progress_line("size=N/A time=N/A bitrate=N/A speed=N/A").is_none());
    }

    #[test]
    fn clamps_a_negative_ffmpeg_position_to_zero() {
        let line = "frame=    0 fps=0.0 q=0.0 size=N/A time=-00:00:00.02 bitrate=N/A speed=N/A";
        let p = parse_progress_line(line).expect("should parse");
        assert_eq!(p.out_time_seconds, Some(0.0));
    }

    // The ffmpeg branch keys off a line prefix; nothing that merely mentions a
    // time may be dragged into it.
    #[test]
    fn ffmpeg_branch_does_not_capture_other_output() {
        assert!(parse_progress_line("[info] Downloading 1 format(s): 137+140").is_none());
        assert!(parse_progress_line("ERROR: unable to download video data time=00:00:01.00").is_none());
    }

    // Without this flag, current yt-dlp resolves no YouTube format at all and
    // every download fails as "Requested format is not available".
    #[test]
    fn js_runtime_is_passed_through_with_its_name_and_path() {
        let spec = spec_with_trim(None);
        let args = build_download_args(&spec, "/bin/ffmpeg", Some("node:/usr/local/bin/node"));
        let i = index_of(&args, "--js-runtimes");
        assert_eq!(args[i + 1], "node:/usr/local/bin/node");
    }

    #[test]
    fn bundled_deno_runtime_keeps_its_own_name() {
        let spec = spec_with_trim(Some(TrimRange { start: 1.0, end: 2.0 }));
        let args = build_download_args(&spec, "/bin/ffmpeg", Some("deno:/app/binaries/deno"));
        let i = index_of(&args, "--js-runtimes");
        assert_eq!(args[i + 1], "deno:/app/binaries/deno");
    }

    // A machine with no runtime must still get a command line yt-dlp accepts:
    // degrade, never fail.
    #[test]
    fn absent_js_runtime_adds_no_argument() {
        for trim in [None, Some(TrimRange { start: 1.0, end: 2.0 })] {
            let spec = spec_with_trim(trim);
            let args = build_download_args(&spec, "/bin/ffmpeg", None);
            assert!(
                !args.iter().any(|a| a == "--js-runtimes"),
                "no runtime must mean no flag: {:?}", args
            );
            assert!(!joined(&args).contains("--js-runtimes"));
        }
    }

    // Regression guard: the byte-oriented parsers must be untouched by the
    // timeline field the ffmpeg branch introduced.
    #[test]
    fn byte_based_progress_reports_no_timeline_position() {
        let ytdlp = parse_progress_line("[download]  42.5% of  120.50MiB at   3.20MiB/s ETA 00:22")
            .expect("should parse");
        assert_eq!(ytdlp.out_time_seconds, None);

        let aria = parse_progress_line("[#f1a2b3 1.2MiB/10MiB(12%) CN:16 DL:2.1MiB ETA:4s]")
            .expect("should parse");
        assert_eq!(aria.out_time_seconds, None);
    }
}
