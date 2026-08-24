# Trim Rework and Queue Foundation — Implementation Plan (Plan A)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace U-Download's broken trim implementation with accurate range-based downloading, and replace the single global download state with a job registry and queue that makes it possible.

**Architecture:** Trimming moves out of a post-hoc FFmpeg pass and into yt-dlp's `--download-sections` with `--force-keyframes-at-cuts`, so only the requested range is fetched and cuts land exactly where asked. The single `Arc<Mutex<DownloadProgress>>` becomes a `HashMap<JobId, JobHandle>` registry with job-scoped Tauri events. The preview is rebuilt around a real `<video>` element whose own `duration` is the source of truth, which structurally removes the dead-slider bug.

**Tech Stack:** Rust (Tauri v2, tokio, serde, regex), React 19, Vite 7, Tailwind 3, yt-dlp, FFmpeg, aria2c.

**Spec:** `docs/superpowers/specs/2026-08-24-stacher-parity-and-trim-design.md`

## Global Constraints

- **`-c copy` must never appear on any trim path.** It is the direct cause of off-target cuts (spec §2.1). Task 2 and Task 3 both assert its absence.
- **`-ss` must precede `-i`** in every FFmpeg trim invocation (spec §5.3).
- **Section timestamps are formatted `HH:MM:SS.mmm`** (spec §5.1).
- **Trimmed jobs must omit `--external-downloader aria2c`**; untrimmed jobs keep it (spec §5.2, pending Task 0 confirmation).
- **aria2c connections scale with concurrency:** `-x`/`-s` = `clamp(16 / concurrency, 4, 16)` (spec §4.4).
- **Duration is `Option<f64>` in Rust and comes from the `<video>` element in the frontend.** No code path may substitute `0.0` for unknown duration (spec §2.4, §6.1).
- **Pause is non-suspending.** Pausing a downloading job kills the process and restarts from zero on resume (spec §4.4).
- **Rust edition 2021**, existing `Cargo.toml` dependency set. No new Rust crates except `uuid`.
- **Every task ends with a commit.** Commit signing is enabled globally via SSH key `~/.ssh/fingo_ed25519`; run `ssh-add ~/.ssh/fingo_ed25519` once before starting or every commit step will fail.

---

### Task 0: Environment prep and empirical verification of yt-dlp constraints

The entire trim strategy rests on two assumptions about yt-dlp that the spec deliberately refuses to assume. Confirm them against the real binary before any code depends on them.

**Files:**
- Create: `docs/superpowers/notes/2026-08-24-ytdlp-capability-findings.md`

**Interfaces:**
- Consumes: nothing
- Produces: a documented yes/no on (a) `--download-sections` + `--external-downloader` compatibility, (b) `--force-keyframes-at-cuts` availability in the bundled version. Task 2 branches on both.

- [ ] **Step 1: Pull the LFS binaries**

The bundled binaries are unpulled LFS pointers — `src-tauri/binaries/macos-arm64/yt-dlp` is a 3-line text stub and executing it fails with `line 1: version: command not found`.

```bash
git lfs pull
```

- [ ] **Step 2: Verify the binaries are now real executables**

```bash
file src-tauri/binaries/macos-arm64/yt-dlp
./src-tauri/binaries/macos-arm64/yt-dlp --version
```

Expected: `file` reports an executable (not ASCII text), and `--version` prints a date-style version such as `2025.09.26`.

- [ ] **Step 3: Confirm `--force-keyframes-at-cuts` exists**

```bash
./src-tauri/binaries/macos-arm64/yt-dlp --help | grep -A2 "force-keyframes-at-cuts"
./src-tauri/binaries/macos-arm64/yt-dlp --help | grep -A3 "download-sections"
```

Expected: both flags are listed. If `--force-keyframes-at-cuts` is absent, the bundled yt-dlp is too old — stop and bump the binary before continuing.

- [ ] **Step 4: Test sections combined with aria2c**

Use a short Creative Commons video to keep the test cheap.

```bash
cd /tmp && mkdir -p udl-probe && cd udl-probe
BIN=/Users/okwared/Softwares/Mine/U-Download/src-tauri/binaries/macos-arm64
PATH="$BIN:$PATH" "$BIN/yt-dlp" \
  --download-sections "*00:00:05.000-00:00:10.000" \
  --force-keyframes-at-cuts \
  --external-downloader aria2c \
  --external-downloader-args "-x 16 -s 16 -k 1M" \
  --ffmpeg-location "$BIN/ffmpeg" \
  -f "bestvideo[height<=480]+bestaudio/best[height<=480]" \
  -o "with-aria2c.%(ext)s" \
  "https://www.youtube.com/watch?v=aqz-KE-bpKQ" 2>&1 | tail -25
```

Record whether yt-dlp errors, warns and ignores the sections, or succeeds. **This is the finding that matters most.**

- [ ] **Step 5: Test sections without aria2c, and measure cut accuracy**

```bash
BIN=/Users/okwared/Softwares/Mine/U-Download/src-tauri/binaries/macos-arm64
PATH="$BIN:$PATH" "$BIN/yt-dlp" \
  --download-sections "*00:00:05.000-00:00:10.000" \
  --force-keyframes-at-cuts \
  --ffmpeg-location "$BIN/ffmpeg" \
  -f "bestvideo[height<=480]+bestaudio/best[height<=480]" \
  -o "no-aria2c.%(ext)s" \
  "https://www.youtube.com/watch?v=aqz-KE-bpKQ" 2>&1 | tail -25

"$BIN/ffprobe" -v error -show_entries format=duration -of csv=p=0 no-aria2c.mp4 2>/dev/null \
  || "$BIN/ffmpeg" -i no-aria2c.mp4 2>&1 | grep Duration
```

Expected: duration is approximately 5.0s. **A result within 0.5s of 5.0s confirms the core fix works.** Anything wildly off (e.g. 12s) means keyframe snapping is still occurring and Task 2's approach needs revisiting.

- [ ] **Step 6: Record the findings**

Write `docs/superpowers/notes/2026-08-24-ytdlp-capability-findings.md` containing: the yt-dlp version string, whether both flags exist, the exact observed behaviour of sections+aria2c (verbatim error/warning text if any), and the measured duration from Step 5. Task 2 reads this file.

- [ ] **Step 7: Clean up and commit**

```bash
rm -rf /tmp/udl-probe
cd /Users/okwared/Softwares/Mine/U-Download
git add docs/superpowers/notes/2026-08-24-ytdlp-capability-findings.md
git commit -m "docs: record empirical yt-dlp section-download capability findings"
```

---

### Task 1: Section timestamp formatting

Establishes that `cargo test` works in this repo (it has never been run) and delivers the first pure function.

**Files:**
- Create: `src-tauri/src/ytdlp.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod ytdlp;`)

**Interfaces:**
- Consumes: nothing
- Produces: `pub fn format_section_timestamp(seconds: f64) -> String` returning `HH:MM:SS.mmm`. Task 2 uses it to build the `--download-sections` argument.

- [ ] **Step 1: Write the failing test**

Create `src-tauri/src/ytdlp.rs` containing only the test module and a stub:

```rust
pub fn format_section_timestamp(_seconds: f64) -> String {
    unimplemented!()
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
}
```

Add to `src-tauri/src/lib.rs` near the other module declarations (it currently declares `mod binary_manager;`):

```rust
mod ytdlp;
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd src-tauri && cargo test ytdlp:: 2>&1 | tail -20
```

Expected: tests compile and fail with `not implemented`.

- [ ] **Step 3: Write the implementation**

Replace the stub in `src-tauri/src/ytdlp.rs`:

```rust
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
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd src-tauri && cargo test ytdlp:: 2>&1 | tail -20
```

Expected: `test result: ok. 6 passed`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/ytdlp.rs src-tauri/src/lib.rs
git commit -m "feat(ytdlp): add section timestamp formatting with rounding carry"
```

---

### Task 2: Trim-aware download argument construction

This is the regression test for the reported off-target-cut bug. The assertions here are the guarantee that the defect cannot silently return.

**Files:**
- Modify: `src-tauri/src/ytdlp.rs`

**Interfaces:**
- Consumes: `format_section_timestamp` (Task 1)
- Produces:
  - `pub struct TrimRange { pub start: f64, pub end: f64 }`
  - `pub enum MediaKind { Mp4, Mp3 }`
  - `pub enum FormatChoice { Quick { kind: MediaKind, height: Option<u32> }, Exact { format_id: String } }`
  - `pub struct DownloadSpec { pub url: String, pub format: FormatChoice, pub trim: Option<TrimRange>, pub output_template: String, pub concurrency: u32 }`
  - `pub fn build_download_args(spec: &DownloadSpec, ffmpeg_path: &str) -> Vec<String>`
  - `pub fn aria2c_connections(concurrency: u32) -> u32`

  Task 7 calls `build_download_args` to spawn yt-dlp.

- [ ] **Step 1: Write the failing tests**

Append to `src-tauri/src/ytdlp.rs`, inside the existing `mod tests`:

```rust
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
        let args = build_download_args(&spec, "/bin/ffmpeg");
        assert!(joined(&args).contains("--download-sections *00:00:10.000-00:00:20.000"));
    }

    #[test]
    fn trimmed_job_forces_keyframes_at_cuts() {
        let spec = spec_with_trim(Some(TrimRange { start: 10.0, end: 20.0 }));
        let args = build_download_args(&spec, "/bin/ffmpeg");
        assert!(args.contains(&"--force-keyframes-at-cuts".to_string()));
    }

    // Regression guard for spec section 2.1: `-c copy` forces keyframe snapping,
    // which is what made trimmed output land seconds away from the selection.
    #[test]
    fn no_trim_path_ever_uses_stream_copy() {
        for trim in [None, Some(TrimRange { start: 1.0, end: 2.0 })] {
            let spec = spec_with_trim(trim);
            let args = build_download_args(&spec, "/bin/ffmpeg");
            let text = joined(&args);
            assert!(!text.contains("-c copy"), "stream copy must never appear: {}", text);
        }
    }

    // Spec section 5.2: sections are incompatible with an external downloader.
    #[test]
    fn trimmed_job_omits_aria2c() {
        let spec = spec_with_trim(Some(TrimRange { start: 10.0, end: 20.0 }));
        let args = build_download_args(&spec, "/bin/ffmpeg");
        assert!(!args.contains(&"--external-downloader".to_string()));
        assert!(!joined(&args).contains("aria2c"));
    }

    #[test]
    fn untrimmed_job_keeps_aria2c_acceleration() {
        let spec = spec_with_trim(None);
        let args = build_download_args(&spec, "/bin/ffmpeg");
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
        let args = build_download_args(&spec, "/bin/ffmpeg");
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
        let args = build_download_args(&spec, "/bin/ffmpeg");
        assert!(joined(&args).contains("-f 137+bestaudio/137"));
    }

    #[test]
    fn quick_height_maps_to_capped_format_selector() {
        let spec = spec_with_trim(None);
        let args = build_download_args(&spec, "/bin/ffmpeg");
        assert!(joined(&args).contains("bestvideo[height<=720]+bestaudio/best[height<=720]"));
    }

    #[test]
    fn aria2c_connections_scale_down_as_concurrency_rises() {
        assert_eq!(aria2c_connections(1), 16);
        assert_eq!(aria2c_connections(2), 8);
        assert_eq!(aria2c_connections(4), 4);
        assert_eq!(aria2c_connections(5), 4); // clamped at the floor
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd src-tauri && cargo test ytdlp:: 2>&1 | tail -20
```

Expected: compilation errors — `DownloadSpec` and `build_download_args` are undefined.

- [ ] **Step 3: Write the implementation**

Add above the test module in `src-tauri/src/ytdlp.rs`:

```rust
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
pub fn build_download_args(spec: &DownloadSpec, ffmpeg_path: &str) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    let trimming = spec.trim.is_some();

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
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd src-tauri && cargo test ytdlp:: 2>&1 | tail -20
```

Expected: all tests pass, including `no_trim_path_ever_uses_stream_copy`.

- [ ] **Step 5: Reconcile with the Task 0 findings**

Open `docs/superpowers/notes/2026-08-24-ytdlp-capability-findings.md`. If Step 4 of Task 0 found that sections and aria2c **do** work together, delete the `trimmed_job_omits_aria2c` test and the `if !trimming` guard, and note the change in the commit message. If it found they conflict (the expected result), leave the code as written.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/ytdlp.rs
git commit -m "feat(ytdlp): build trim args via --download-sections, never stream copy

Replaces the post-hoc FFmpeg approach whose '-c copy' forced cuts onto
keyframes. Includes a regression test asserting '-c copy' appears on no
trim path."
```

---

### Task 3: FFmpeg fallback trim arguments

For extractors that do not support `--download-sections`. Correct by construction: fast seek before input, genuine re-encode.

**Files:**
- Modify: `src-tauri/src/ytdlp.rs`

**Interfaces:**
- Consumes: `TrimRange`, `format_section_timestamp` (Tasks 1–2)
- Produces: `pub fn build_ffmpeg_trim_args(input: &str, output: &str, range: TrimRange, audio_only: bool) -> Vec<String>`. Task 7 calls this on the fallback path.

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests`:

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd src-tauri && cargo test ytdlp:: 2>&1 | tail -20
```

Expected: `build_ffmpeg_trim_args` is undefined.

- [ ] **Step 3: Write the implementation**

```rust
/// Fallback trim for extractors that do not support `--download-sections`.
///
/// `-ss` is placed *before* `-i` so ffmpeg seeks by index instead of decoding
/// from the start, and the streams are genuinely re-encoded. Stream copy is
/// deliberately absent: it can only cut on keyframes.
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
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd src-tauri && cargo test ytdlp:: 2>&1 | tail -20
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/ytdlp.rs
git commit -m "feat(ytdlp): add correct-by-construction ffmpeg fallback trim args"
```

---

### Task 4: Progress line parsing

The existing parsing logic is inline in `perform_download` and has never been tested. Extract it so Task 7 can delete that function safely.

**Files:**
- Modify: `src-tauri/src/ytdlp.rs`
- Reference: `src-tauri/src/lib.rs:800-860` (existing inline regex logic)

**Interfaces:**
- Consumes: nothing
- Produces: `pub struct ProgressLine { pub percentage: f64, pub total_bytes: Option<u64>, pub speed_bytes_per_sec: Option<u64>, pub eta_seconds: Option<u64> }` and `pub fn parse_progress_line(line: &str) -> Option<ProgressLine>`. Task 7 calls it per stdout line.

- [ ] **Step 1: Write the failing tests**

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd src-tauri && cargo test ytdlp:: 2>&1 | tail -20
```

Expected: `parse_progress_line` is undefined.

- [ ] **Step 3: Write the implementation**

```rust
use regex::Regex;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProgressLine {
    pub percentage: f64,
    pub total_bytes: Option<u64>,
    pub speed_bytes_per_sec: Option<u64>,
    pub eta_seconds: Option<u64>,
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
pub fn parse_progress_line(line: &str) -> Option<ProgressLine> {
    if !line.starts_with("[download]") {
        return None;
    }

    let pct_re = Regex::new(r"(\d+(?:\.\d+)?)%").ok()?;
    let percentage = pct_re
        .captures(line)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<f64>().ok())?;

    let size_re = Regex::new(r"of\s+~?\s*(\d+(?:\.\d+)?)([KMG]i?B)").ok()?;
    let total_bytes = size_re
        .captures(line)
        .and_then(|c| parse_size(c.get(1)?.as_str(), c.get(2)?.as_str()));

    let speed_re = Regex::new(r"at\s+(\d+(?:\.\d+)?)([KMG]i?B)/s").ok()?;
    let speed_bytes_per_sec = speed_re
        .captures(line)
        .and_then(|c| parse_size(c.get(1)?.as_str(), c.get(2)?.as_str()));

    let eta_re = Regex::new(r"ETA\s+(\d{1,2}(?::\d{2})+)").ok()?;
    let eta_seconds = eta_re
        .captures(line)
        .and_then(|c| c.get(1))
        .and_then(|m| parse_clock(m.as_str()));

    Some(ProgressLine { percentage, total_bytes, speed_bytes_per_sec, eta_seconds })
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd src-tauri && cargo test ytdlp:: 2>&1 | tail -20
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/ytdlp.rs
git commit -m "feat(ytdlp): extract and test yt-dlp progress line parsing"
```

---

### Task 5: Job model and registry

Replaces the single global `ProgressState` that blocks every queue feature.

**Files:**
- Create: `src-tauri/src/jobs.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod jobs;`)
- Modify: `src-tauri/Cargo.toml` (add `uuid`)

**Interfaces:**
- Consumes: `FormatChoice`, `TrimRange` (Task 2)
- Produces:
  - `pub type JobId = String`
  - `pub struct Job` / `pub struct JobProgress` / `pub enum JobStatus`
  - `pub struct JobRegistry` with `new()`, `insert(Job) -> JobId`, `get(&JobId) -> Option<Job>`, `update_progress(&JobId, JobProgress)`, `set_status(&JobId, JobStatus)`, `list() -> Vec<Job>`, `queued_ids() -> Vec<JobId>`, `active_count() -> usize`
  - `pub type SharedJobs = Arc<Mutex<JobRegistry>>`

  Task 6 drives the registry; Task 7 exposes it via commands.

- [ ] **Step 1: Add the uuid dependency**

```bash
cd src-tauri && cargo add uuid --features v4
```

- [ ] **Step 2: Write the failing tests**

Create `src-tauri/src/jobs.rs` with the test module and stubs. Tests:

```rust
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
```

- [ ] **Step 3: Run the tests to verify they fail**

```bash
cd src-tauri && cargo test jobs:: 2>&1 | tail -20
```

Expected: compilation errors — `Job`, `JobRegistry` undefined.

- [ ] **Step 4: Write the implementation**

Add above the test module in `src-tauri/src/jobs.rs`:

```rust
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

    /// Kills the running process, if any, and marks the job cancelled.
    pub fn cancel(&mut self, id: &JobId) {
        if let Some(h) = self.handles.get_mut(id) {
            if let Some(child) = h.child.as_mut() {
                let _ = child.kill();
            }
            h.child = None;
            h.job.status = JobStatus::Cancelled;
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
```

Add `mod jobs;` to `src-tauri/src/lib.rs`.

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cd src-tauri && cargo test jobs:: 2>&1 | tail -20
```

Expected: all 8 pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/jobs.rs src-tauri/src/lib.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat(jobs): add job model and registry replacing global progress state"
```

---

### Task 6: Queue scheduler

**Files:**
- Create: `src-tauri/src/queue.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod queue;`)

**Interfaces:**
- Consumes: `JobRegistry`, `JobStatus`, `JobId` (Task 5)
- Produces: `pub fn next_dispatchable(reg: &JobRegistry, concurrency: usize) -> Vec<JobId>` and `pub fn pause(reg: &mut JobRegistry, id: &JobId)` / `pub fn resume(reg: &mut JobRegistry, id: &JobId)`. Task 7 calls `next_dispatchable` after every job state change.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{Job, JobRegistry, JobStatus};
    use crate::ytdlp::{FormatChoice, MediaKind};

    fn reg_with(n: usize) -> (JobRegistry, Vec<String>) {
        let mut reg = JobRegistry::new();
        let ids = (0..n)
            .map(|_| {
                reg.insert(Job::new(
                    "https://example.com/v".to_string(),
                    FormatChoice::Quick { kind: MediaKind::Mp4, height: None },
                    None,
                    "/out".to_string(),
                ))
            })
            .collect();
        (reg, ids)
    }

    #[test]
    fn dispatches_up_to_the_concurrency_limit() {
        let (reg, _) = reg_with(5);
        assert_eq!(next_dispatchable(&reg, 2).len(), 2);
    }

    #[test]
    fn dispatches_in_fifo_order() {
        let (reg, ids) = reg_with(3);
        assert_eq!(next_dispatchable(&reg, 2), vec![ids[0].clone(), ids[1].clone()]);
    }

    #[test]
    fn accounts_for_already_active_jobs() {
        let (mut reg, ids) = reg_with(4);
        reg.set_status(&ids[0], JobStatus::Downloading);
        // One slot of two is taken, so only one more may start.
        assert_eq!(next_dispatchable(&reg, 2).len(), 1);
    }

    #[test]
    fn dispatches_nothing_when_saturated() {
        let (mut reg, ids) = reg_with(4);
        reg.set_status(&ids[0], JobStatus::Downloading);
        reg.set_status(&ids[1], JobStatus::Processing);
        assert!(next_dispatchable(&reg, 2).is_empty());
    }

    #[test]
    fn skips_paused_jobs() {
        let (mut reg, ids) = reg_with(3);
        reg.set_status(&ids[0], JobStatus::Paused);
        assert_eq!(next_dispatchable(&reg, 1), vec![ids[1].clone()]);
    }

    #[test]
    fn pausing_a_queued_job_removes_it_from_dispatch() {
        let (mut reg, ids) = reg_with(2);
        pause(&mut reg, &ids[0]);
        assert_eq!(reg.get(&ids[0]).unwrap().status, JobStatus::Paused);
        assert_eq!(next_dispatchable(&reg, 4), vec![ids[1].clone()]);
    }

    // Spec section 4.4: pause is non-suspending — an in-flight job is killed.
    #[test]
    fn pausing_a_downloading_job_cancels_it_and_resets_progress() {
        let (mut reg, ids) = reg_with(1);
        reg.set_status(&ids[0], JobStatus::Downloading);
        reg.update_progress(&ids[0], crate::jobs::JobProgress { percentage: 55.0, ..Default::default() });

        pause(&mut reg, &ids[0]);

        let job = reg.get(&ids[0]).unwrap();
        assert_eq!(job.status, JobStatus::Paused);
        assert_eq!(job.progress.percentage, 0.0, "progress is not preserved across pause");
    }

    #[test]
    fn resuming_returns_a_job_to_the_queue() {
        let (mut reg, ids) = reg_with(1);
        pause(&mut reg, &ids[0]);
        resume(&mut reg, &ids[0]);
        assert_eq!(reg.get(&ids[0]).unwrap().status, JobStatus::Queued);
    }

    #[test]
    fn terminal_jobs_are_never_dispatched() {
        let (mut reg, ids) = reg_with(3);
        reg.set_status(&ids[0], JobStatus::Done);
        reg.set_status(&ids[1], JobStatus::Failed);
        assert_eq!(next_dispatchable(&reg, 4), vec![ids[2].clone()]);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd src-tauri && cargo test queue:: 2>&1 | tail -20
```

Expected: `next_dispatchable`, `pause`, `resume` undefined.

- [ ] **Step 3: Write the implementation**

```rust
use crate::jobs::{JobId, JobProgress, JobRegistry, JobStatus};

/// Returns the job ids that may start right now, respecting the concurrency
/// limit and the slots already occupied by in-flight work.
pub fn next_dispatchable(reg: &JobRegistry, concurrency: usize) -> Vec<JobId> {
    let limit = concurrency.max(1);
    let free = limit.saturating_sub(reg.active_count());
    reg.queued_ids().into_iter().take(free).collect()
}

/// Pauses a job. There is no process-level suspend: a job that is already
/// downloading is killed outright and its progress discarded, so resuming
/// restarts it from zero. The UI must present it that way.
pub fn pause(reg: &mut JobRegistry, id: &JobId) {
    let status = match reg.get(id) {
        Some(job) => job.status,
        None => return,
    };

    if status.is_active() {
        reg.cancel(id);
        reg.update_progress(id, JobProgress::default());
    }

    reg.set_status(id, JobStatus::Paused);
}

pub fn resume(reg: &mut JobRegistry, id: &JobId) {
    if reg.get(id).map(|j| j.status) == Some(JobStatus::Paused) {
        reg.set_status(id, JobStatus::Queued);
    }
}
```

Add `mod queue;` to `src-tauri/src/lib.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd src-tauri && cargo test queue:: 2>&1 | tail -20
```

Expected: all 9 pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/queue.rs src-tauri/src/lib.rs
git commit -m "feat(queue): add concurrency-limited FIFO scheduler with non-suspending pause"
```

---

### Task 7: Wire the runner and delete `perform_trimming`

The integration task. After this, the old broken trim code is gone from the repository.

**Files:**
- Create: `src-tauri/src/runner.rs`
- Modify: `src-tauri/src/lib.rs` — **delete** `perform_trimming` (lines 891-980) and `perform_download`'s trim branches (lines 456-467, 513-519, 869-872); replace `ProgressState` (line 41) usage; update `invoke_handler` (lines 1001-1009)

**Interfaces:**
- Consumes: `build_download_args`, `parse_progress_line` (Tasks 2, 4); `JobRegistry`, `SharedJobs` (Task 5); `next_dispatchable` (Task 6)
- Produces: Tauri commands `enqueue_job(url, format, trim, output_folder) -> JobId`, `list_jobs() -> Vec<Job>`, `cancel_job(job_id)`, `pause_job(job_id)`, `resume_job(job_id)`; events `job-updated`, `job-done`, `job-failed`. Task 10's `useJobs` hook consumes these.

- [ ] **Step 1: Confirm the current broken code is present before removing it**

```bash
grep -n "perform_trimming\|_temp\|\"-c\"" src-tauri/src/lib.rs
```

Expected: matches at the `perform_trimming` definition, its call site, the `_temp` output pattern, and the `-c copy` args. Record the line numbers — these are what Step 4 removes.

- [ ] **Step 2: Write the runner**

Create `src-tauri/src/runner.rs`:

```rust
use crate::jobs::{JobId, JobProgress, JobStatus, SharedJobs};
use crate::queue;
use crate::ytdlp::{build_download_args, parse_progress_line, DownloadSpec};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use tauri::{Emitter, Runtime, Window};

/// Emits at most one progress event per job per interval, so several concurrent
/// downloads cannot flood the webview bridge.
const PROGRESS_EMIT_INTERVAL_MS: u128 = 500;

fn emit_job<R: Runtime>(window: &Window<R>, jobs: &SharedJobs, id: &JobId) {
    let job = { jobs.lock().unwrap().get(id) };
    if let Some(job) = job {
        let _ = window.emit("job-updated", job);
    }
}

/// Runs one job to completion on a blocking thread.
pub fn run_job<R: Runtime>(
    window: Window<R>,
    jobs: SharedJobs,
    id: JobId,
    yt_dlp: std::path::PathBuf,
    ffmpeg: std::path::PathBuf,
    binaries_dir: std::path::PathBuf,
    concurrency: u32,
) {
    let job = match jobs.lock().unwrap().get(&id) {
        Some(j) => j,
        None => return,
    };

    let spec = DownloadSpec {
        url: job.url.clone(),
        format: job.format.clone(),
        trim: job.trim,
        output_template: format!("{}/%(title)s.%(ext)s", job.output_folder),
        concurrency,
    };

    let args = build_download_args(&spec, &ffmpeg.to_string_lossy());

    let mut cmd = Command::new(&yt_dlp);
    crate::binary_manager::augment_path_env(&mut cmd, &binaries_dir);
    cmd.args(&args).stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            jobs.lock().unwrap().set_error(&id, format!("Failed to start yt-dlp: {e}"));
            let _ = window.emit("job-failed", serde_json::json!({ "job_id": id, "error": e.to_string() }));
            return;
        }
    };

    let stdout = child.stdout.take();
    {
        let mut reg = jobs.lock().unwrap();
        reg.set_status(&id, JobStatus::Downloading);
    }
    emit_job(&window, &jobs, &id);

    if let Some(stdout) = stdout {
        let reader = BufReader::new(stdout);
        let mut last_emit = std::time::Instant::now();

        for line in reader.lines().map_while(Result::ok) {
            if let Some(p) = parse_progress_line(&line) {
                {
                    let mut reg = jobs.lock().unwrap();
                    reg.update_progress(
                        &id,
                        JobProgress {
                            percentage: p.percentage,
                            speed_bytes_per_sec: p.speed_bytes_per_sec.unwrap_or(0),
                            eta_seconds: p.eta_seconds,
                            bytes_downloaded: 0,
                            total_bytes: p.total_bytes.unwrap_or(0),
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
    }

    let mut stderr_text = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        use std::io::Read;
        let _ = stderr.read_to_string(&mut stderr_text);
    }

    let status = child.wait();

    let cancelled = jobs.lock().unwrap().get(&id).map(|j| j.status) == Some(JobStatus::Cancelled);
    if cancelled {
        return;
    }

    match status {
        Ok(s) if s.success() => {
            {
                let mut reg = jobs.lock().unwrap();
                reg.set_status(&id, JobStatus::Done);
                reg.update_progress(&id, JobProgress { percentage: 100.0, ..Default::default() });
            }
            emit_job(&window, &jobs, &id);
            let title = jobs.lock().unwrap().get(&id).map(|j| j.title).unwrap_or_default();
            let _ = window.emit("job-done", serde_json::json!({ "job_id": id, "title": title }));
        }
        Ok(s) => {
            let msg = format!("yt-dlp exited with {}: {}", s.code().unwrap_or(-1), stderr_text.trim());
            jobs.lock().unwrap().set_error(&id, msg.clone());
            emit_job(&window, &jobs, &id);
            let _ = window.emit("job-failed", serde_json::json!({ "job_id": id, "error": msg }));
        }
        Err(e) => {
            let msg = format!("Process error: {e}");
            jobs.lock().unwrap().set_error(&id, msg.clone());
            emit_job(&window, &jobs, &id);
            let _ = window.emit("job-failed", serde_json::json!({ "job_id": id, "error": msg }));
        }
    }
}

/// Starts every job the scheduler currently permits.
pub fn pump<R: Runtime>(
    window: Window<R>,
    jobs: SharedJobs,
    yt_dlp: std::path::PathBuf,
    ffmpeg: std::path::PathBuf,
    binaries_dir: std::path::PathBuf,
    concurrency: u32,
) {
    let ready = {
        let reg = jobs.lock().unwrap();
        queue::next_dispatchable(&reg, concurrency as usize)
    };

    for id in ready {
        let (w, j, y, f, b) = (
            window.clone(),
            jobs.clone(),
            yt_dlp.clone(),
            ffmpeg.clone(),
            binaries_dir.clone(),
        );
        std::thread::spawn(move || {
            run_job(w.clone(), j.clone(), id, y.clone(), f.clone(), b.clone(), concurrency);
            // A finished job frees a slot; start whatever is next.
            pump(w, j, y, f, b, concurrency);
        });
    }
}
```

- [ ] **Step 3: Add the commands**

Add to `src-tauri/src/lib.rs`:

```rust
mod runner;

#[tauri::command]
async fn enqueue_job<R: Runtime>(
    window: Window<R>,
    jobs: tauri::State<'_, jobs::SharedJobs>,
    url: String,
    format: ytdlp::FormatChoice,
    trim: Option<ytdlp::TrimRange>,
    output_folder: String,
    concurrency: Option<u32>,
) -> Result<String, String> {
    let app_handle = window.app_handle();
    let paths = binary_manager::resolve_paths(&app_handle)?;
    binary_manager::ensure_executable(&paths)?;

    let job = jobs::Job::new(url, format, trim, output_folder);
    let id = jobs.lock().unwrap().insert(job);

    runner::pump(
        window.clone(),
        jobs.inner().clone(),
        paths.yt_dlp.clone(),
        paths.ffmpeg.clone(),
        paths.dir.clone(),
        concurrency.unwrap_or(2),
    );

    Ok(id)
}

#[tauri::command]
fn list_jobs(jobs: tauri::State<'_, jobs::SharedJobs>) -> Vec<jobs::Job> {
    jobs.lock().unwrap().list()
}

#[tauri::command]
fn cancel_job(jobs: tauri::State<'_, jobs::SharedJobs>, job_id: String) {
    jobs.lock().unwrap().cancel(&job_id);
}

#[tauri::command]
fn pause_job(jobs: tauri::State<'_, jobs::SharedJobs>, job_id: String) {
    queue::pause(&mut jobs.lock().unwrap(), &job_id);
}

#[tauri::command]
fn resume_job(jobs: tauri::State<'_, jobs::SharedJobs>, job_id: String) {
    queue::resume(&mut jobs.lock().unwrap(), &job_id);
}
```

Replace the managed state in `run()` — the `ProgressState` block at `lib.rs:983-993` becomes:

```rust
let jobs: jobs::SharedJobs = Arc::new(Mutex::new(jobs::JobRegistry::new()));
```

and `.manage(progress_state)` becomes `.manage(jobs)`.

Update `invoke_handler` (lines 1001-1009) to:

```rust
.invoke_handler(tauri::generate_handler![
    select_output_folder,
    test_dependencies,
    get_video_metadata,
    check_ffmpeg,
    get_shared_url,
    get_android_videos_dir,
    enqueue_job,
    list_jobs,
    cancel_job,
    pause_job,
    resume_job
])
```

- [ ] **Step 4: Delete the broken trim implementation**

Remove from `src-tauri/src/lib.rs`:
- The entire `perform_trimming` function (lines 891-980) — the `-c copy` keyframe-snapping cut.
- Its call site (lines 869-872).
- The `_temp` output pattern branch (lines 513-519) and the `trimming_enabled` FFmpeg check (lines 456-467).
- The old `start_download` command and `perform_download`, now superseded by `enqueue_job` and `runner::run_job`.
- The `DownloadProgress` struct and `ProgressState` type alias (lines 20-30, 41).

- [ ] **Step 5: Verify the broken code is gone and the build is clean**

```bash
grep -n "perform_trimming\|_temp\|ProgressState" src-tauri/src/lib.rs
```

Expected: **no matches.**

```bash
cd src-tauri && cargo build 2>&1 | tail -30
```

Expected: compiles. Resolve any residual references to the removed symbols.

- [ ] **Step 6: Run the full test suite**

```bash
cd src-tauri && cargo test 2>&1 | tail -20
```

Expected: all tests from Tasks 1-6 still pass.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/runner.rs src-tauri/src/lib.rs
git commit -m "feat(runner): run jobs through the queue and delete perform_trimming

Removes the FFmpeg post-pass whose '-c copy' snapped cuts to keyframes,
along with the fragile '_temp' file discovery that could trim the wrong
file. Trimming is now yt-dlp's --download-sections."
```

---

### Task 8: Preview source resolution

Backend half of the hybrid preview. Picks a directly-playable muxed stream, or reports that none exists so the frontend can fall back to a proxy.

**Files:**
- Create: `src-tauri/src/probe.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod probe;`, register command)

**Interfaces:**
- Consumes: nothing
- Produces: `pub struct PreviewSource { pub kind: String, pub url: Option<String>, pub duration: Option<f64>, pub title: String, pub thumbnail: String }` where `kind` is `"stream"` or `"needs_proxy"`; command `resolve_preview(url) -> PreviewSource`; `pub fn pick_muxed_format(formats: &serde_json::Value) -> Option<serde_json::Value>`. Task 11's `TrimWorkbench` consumes the command.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn picks_a_muxed_format_over_video_only() {
        let formats = json!([
            { "format_id": "137", "vcodec": "avc1", "acodec": "none",  "height": 1080 },
            { "format_id": "18",  "vcodec": "avc1", "acodec": "mp4a",  "height": 360, "url": "https://cdn/18" }
        ]);
        let picked = pick_muxed_format(&formats).expect("a muxed format exists");
        assert_eq!(picked["format_id"], "18");
    }

    #[test]
    fn prefers_the_highest_muxed_format_at_or_below_480p() {
        let formats = json!([
            { "format_id": "18",  "vcodec": "avc1", "acodec": "mp4a", "height": 360, "url": "https://cdn/18" },
            { "format_id": "22",  "vcodec": "avc1", "acodec": "mp4a", "height": 720, "url": "https://cdn/22" },
            { "format_id": "59",  "vcodec": "avc1", "acodec": "mp4a", "height": 480, "url": "https://cdn/59" }
        ]);
        let picked = pick_muxed_format(&formats).expect("a muxed format exists");
        assert_eq!(picked["format_id"], "59");
    }

    // DASH-only videos, common for 4K and long uploads, have no muxed format at
    // all — this is the case the proxy fallback exists for.
    #[test]
    fn returns_none_when_every_format_is_split() {
        let formats = json!([
            { "format_id": "137", "vcodec": "avc1", "acodec": "none", "height": 1080 },
            { "format_id": "140", "vcodec": "none", "acodec": "mp4a" }
        ]);
        assert!(pick_muxed_format(&formats).is_none());
    }

    #[test]
    fn ignores_muxed_entries_that_carry_no_url() {
        let formats = json!([
            { "format_id": "18", "vcodec": "avc1", "acodec": "mp4a", "height": 360 }
        ]);
        assert!(pick_muxed_format(&formats).is_none());
    }

    #[test]
    fn handles_an_empty_format_list() {
        assert!(pick_muxed_format(&json!([])).is_none());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd src-tauri && cargo test probe:: 2>&1 | tail -20
```

Expected: `pick_muxed_format` undefined.

- [ ] **Step 3: Write the implementation**

```rust
use serde::{Deserialize, Serialize};
use std::process::Command;
use tauri::{AppHandle, Runtime};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewSource {
    /// "stream" when a directly playable URL was found, "needs_proxy" otherwise.
    pub kind: String,
    pub url: Option<String>,
    pub duration: Option<f64>,
    pub title: String,
    pub thumbnail: String,
}

/// Selects a format the webview's <video> element can play on its own: both a
/// video and an audio codec in one stream, with a resolvable URL. Prefers the
/// best such format at or below 480p — preview quality is irrelevant, and small
/// streams seek faster.
pub fn pick_muxed_format(formats: &serde_json::Value) -> Option<serde_json::Value> {
    let arr = formats.as_array()?;

    let mut candidates: Vec<&serde_json::Value> = arr
        .iter()
        .filter(|f| {
            let v = f["vcodec"].as_str().unwrap_or("none");
            let a = f["acodec"].as_str().unwrap_or("none");
            v != "none" && a != "none" && f["url"].as_str().is_some()
        })
        .collect();

    if candidates.is_empty() {
        return None;
    }

    // Highest height <= 480; if none qualify, the smallest available.
    candidates.sort_by_key(|f| f["height"].as_u64().unwrap_or(0));

    let best = candidates
        .iter()
        .rev()
        .find(|f| f["height"].as_u64().unwrap_or(0) <= 480)
        .or_else(|| candidates.first())?;

    Some((*best).clone())
}

#[tauri::command]
pub async fn resolve_preview<R: Runtime>(
    app_handle: AppHandle<R>,
    url: String,
) -> Result<PreviewSource, String> {
    let paths = crate::binary_manager::resolve_paths(&app_handle)?;
    crate::binary_manager::ensure_executable(&paths)?;

    // --dump-single-json emits exactly one object even for playlists, unlike
    // --dump-json which emits one per entry and breaks JSON parsing.
    let output = Command::new(&paths.yt_dlp)
        .arg("--dump-single-json")
        .arg("--no-playlist")
        .arg("--no-download")
        .arg(&url)
        .output()
        .map_err(|e| format!("Failed to probe video: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "Could not read video info: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let meta: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))
        .map_err(|e| format!("Failed to parse video info: {e}"))?;

    let title = meta["title"].as_str().unwrap_or("Unknown Title").to_string();
    let thumbnail = meta["thumbnail"].as_str().unwrap_or("").to_string();
    // Absent duration stays None. Substituting 0.0 here is what previously
    // collapsed the frontend scrub control to two positions.
    let duration = meta["duration"].as_f64();

    match pick_muxed_format(&meta["formats"]) {
        Some(f) => Ok(PreviewSource {
            kind: "stream".to_string(),
            url: f["url"].as_str().map(|s| s.to_string()),
            duration,
            title,
            thumbnail,
        }),
        None => Ok(PreviewSource {
            kind: "needs_proxy".to_string(),
            url: None,
            duration,
            title,
            thumbnail,
        }),
    }
}
```

Add `mod probe;` to `lib.rs` and `probe::resolve_preview` to `invoke_handler`.

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd src-tauri && cargo test probe:: 2>&1 | tail -20
```

Expected: all 5 pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/probe.rs src-tauri/src/lib.rs
git commit -m "feat(probe): resolve a playable preview stream or signal proxy fallback"
```

---

### Task 9: Frontend time utilities with a test runner

The repo has no JS test runner. Add Vitest and extract the time helpers, whose `formatTime` currently cannot render durations over an hour.

**Files:**
- Create: `src/lib/time.js`, `src/lib/time.test.js`
- Modify: `package.json` (Vitest reads the existing `vite.config.js` as-is; no change needed there)

**Interfaces:**
- Consumes: nothing
- Produces: `parseTimeToSeconds(value: string) -> number | NaN` and `formatTime(seconds: number) -> string`. Task 11 imports both.

- [ ] **Step 1: Install Vitest**

```bash
npm install -D vitest
```

Add to `package.json` scripts:

```json
"test": "vitest run",
"test:watch": "vitest"
```

- [ ] **Step 2: Write the failing tests**

Create `src/lib/time.test.js`:

```js
import { describe, it, expect } from 'vitest';
import { parseTimeToSeconds, formatTime } from './time';

describe('parseTimeToSeconds', () => {
  it('parses bare seconds', () => {
    expect(parseTimeToSeconds('45')).toBe(45);
  });

  it('parses MM:SS', () => {
    expect(parseTimeToSeconds('2:30')).toBe(150);
  });

  it('parses HH:MM:SS', () => {
    expect(parseTimeToSeconds('1:02:03')).toBe(3723);
  });

  it('tolerates surrounding whitespace', () => {
    expect(parseTimeToSeconds('  2:30  ')).toBe(150);
  });

  it('returns NaN for non-numeric input', () => {
    expect(parseTimeToSeconds('abc')).toBeNaN();
    expect(parseTimeToSeconds('1:xx')).toBeNaN();
  });

  it('returns NaN for empty input', () => {
    expect(parseTimeToSeconds('')).toBeNaN();
    expect(parseTimeToSeconds(null)).toBeNaN();
  });

  it('returns NaN for too many segments', () => {
    expect(parseTimeToSeconds('1:2:3:4')).toBeNaN();
  });
});

describe('formatTime', () => {
  it('formats under a minute', () => {
    expect(formatTime(45)).toBe('0:45');
  });

  it('formats minutes and seconds', () => {
    expect(formatTime(150)).toBe('2:30');
  });

  // The previous implementation rendered 3723 as "62:03", which is unreadable
  // for the long videos trimming is most useful on.
  it('formats past an hour as H:MM:SS', () => {
    expect(formatTime(3723)).toBe('1:02:03');
  });

  it('clamps negatives to zero', () => {
    expect(formatTime(-5)).toBe('0:00');
  });

  it('handles null and undefined', () => {
    expect(formatTime(null)).toBe('0:00');
    expect(formatTime(undefined)).toBe('0:00');
  });

  it('round-trips with parseTimeToSeconds', () => {
    for (const secs of [0, 45, 150, 3723, 7199]) {
      expect(parseTimeToSeconds(formatTime(secs))).toBe(secs);
    }
  });
});
```

- [ ] **Step 3: Run the tests to verify they fail**

```bash
npm test
```

Expected: FAIL — cannot resolve `./time`.

- [ ] **Step 4: Write the implementation**

Create `src/lib/time.js`. `parseTimeToSeconds` is moved verbatim from `src/VideoPreview.jsx:56-73` — it is already correct. `formatTime` is extended to emit an hours component.

```js
/** Parses "SS", "MM:SS", or "HH:MM:SS" into seconds. Returns NaN if invalid. */
export function parseTimeToSeconds(value) {
  const s = String(value || '').trim();
  if (!s) return NaN;

  const parts = s.split(':').map((p) => p.trim());
  if (parts.some((p) => p === '' || isNaN(Number(p)))) return NaN;

  if (parts.length === 1) return Math.floor(Number(parts[0]));
  if (parts.length === 2) {
    const [m, sec] = parts.map((p) => Math.floor(Number(p)));
    return m * 60 + sec;
  }
  if (parts.length === 3) {
    const [h, m, sec] = parts.map((p) => Math.floor(Number(p)));
    return h * 3600 + m * 60 + sec;
  }
  return NaN;
}

/**
 * Formats seconds as "M:SS", or "H:MM:SS" once past an hour. The hours case
 * matters: long videos are exactly where trimming is most used.
 */
export function formatTime(seconds) {
  const t = Math.max(0, Math.floor(seconds || 0));
  const h = Math.floor(t / 3600);
  const m = Math.floor((t % 3600) / 60);
  const s = t % 60;

  if (h > 0) {
    return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
  }
  return `${m}:${String(s).padStart(2, '0')}`;
}
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
npm test
```

Expected: all pass, including the round-trip.

- [ ] **Step 6: Commit**

```bash
git add src/lib/time.js src/lib/time.test.js package.json package-lock.json
git commit -m "feat(time): extract time helpers with vitest coverage and H:MM:SS support"
```

---

### Task 10: Job event hook

**Files:**
- Create: `src/hooks/useJobs.js`

**Interfaces:**
- Consumes: commands and events from Task 7
- Produces: `useJobs()` returning `{ jobs, enqueue, cancel, pause, resume }` where `jobs` is an array ordered by `created_at`. Task 12 consumes it.

- [ ] **Step 1: Write the hook**

```js
import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

/**
 * Keeps a map of jobs keyed by id, updated from job-scoped events.
 *
 * Routing by job_id is what makes concurrent downloads possible: the previous
 * implementation wrote every progress event into one set of global useState
 * variables, so a second download would overwrite the first one's display.
 */
export function useJobs() {
  const [jobsById, setJobsById] = useState({});
  const unlistenRefs = useRef([]);

  useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        const initial = await invoke('list_jobs');
        if (!cancelled) {
          setJobsById(Object.fromEntries(initial.map((j) => [j.id, j])));
        }
      } catch (e) {
        console.error('Failed to load jobs:', e);
      }

      const unlistenUpdated = await listen('job-updated', (event) => {
        const job = event.payload;
        setJobsById((prev) => ({ ...prev, [job.id]: job }));
      });

      const unlistenFailed = await listen('job-failed', (event) => {
        const { job_id, error } = event.payload;
        setJobsById((prev) =>
          prev[job_id]
            ? { ...prev, [job_id]: { ...prev[job_id], status: 'failed', error } }
            : prev
        );
      });

      unlistenRefs.current = [unlistenUpdated, unlistenFailed];
    })();

    return () => {
      cancelled = true;
      unlistenRefs.current.forEach((fn) => fn && fn());
    };
  }, []);

  const enqueue = useCallback(async ({ url, format, trim, outputFolder, concurrency }) => {
    const id = await invoke('enqueue_job', {
      url,
      format,
      trim: trim ?? null,
      outputFolder,
      concurrency: concurrency ?? 2,
    });
    const fresh = await invoke('list_jobs');
    setJobsById(Object.fromEntries(fresh.map((j) => [j.id, j])));
    return id;
  }, []);

  const cancel = useCallback((jobId) => invoke('cancel_job', { jobId }), []);
  const pause = useCallback((jobId) => invoke('pause_job', { jobId }), []);
  const resume = useCallback((jobId) => invoke('resume_job', { jobId }), []);

  const jobs = Object.values(jobsById).sort((a, b) => a.created_at - b.created_at);

  return { jobs, enqueue, cancel, pause, resume };
}
```

- [ ] **Step 2: Verify it compiles**

```bash
npm run build
```

Expected: build succeeds.

- [ ] **Step 3: Commit**

```bash
git add src/hooks/useJobs.js
git commit -m "feat(hooks): add useJobs routing job-scoped events by id"
```

---

### Task 11: Trim workbench with real video playback

Replaces `src/VideoPreview.jsx`. This is where "cannot see what I'm cutting" and "cannot select start and end" are fixed.

**Files:**
- Create: `src/components/TrimWorkbench.jsx`
- Delete: `src/VideoPreview.jsx`
- Modify: `src/VideoPreview.css` → rename to `src/components/TrimWorkbench.css`

**Interfaces:**
- Consumes: `resolve_preview` (Task 8); `parseTimeToSeconds`, `formatTime` (Task 9)
- Produces: `<TrimWorkbench url onChange />` where `onChange({ start, end })` fires whenever the selection changes, and `null` clears it. Task 12 renders it.

- [ ] **Step 1: Write the component**

```jsx
import { useState, useRef, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { formatTime, parseTimeToSeconds } from '../lib/time';
import './TrimWorkbench.css';

/**
 * Trim workbench built around a real <video> element.
 *
 * The element's own `duration` is the source of truth. The previous
 * implementation derived the scrub bound from yt-dlp metadata, which defaults
 * to 0 whenever a probe fails — collapsing `Math.max(1, duration)` to 1 and
 * leaving a two-position slider. Reading duration from a stream the browser has
 * actually decoded makes that failure mode unreachable.
 */
export default function TrimWorkbench({ url, onChange }) {
  const videoRef = useRef(null);
  const trackRef = useRef(null);

  const [source, setSource] = useState(null);      // { kind, url, title }
  const [phase, setPhase] = useState('idle');      // idle|probing|proxying|ready|error
  const [error, setError] = useState('');
  const [duration, setDuration] = useState(0);     // from the <video> element
  const [current, setCurrent] = useState(0);
  const [inPoint, setInPoint] = useState(null);
  const [outPoint, setOutPoint] = useState(null);
  const [dragging, setDragging] = useState(null);  // 'in' | 'out' | null
  const [startInput, setStartInput] = useState('');
  const [endInput, setEndInput] = useState('');

  const ready = phase === 'ready' && duration > 0;

  // --- source resolution: stream first, proxy on failure -------------------

  const loadProxy = useCallback(async () => {
    setPhase('proxying');
    try {
      const path = await invoke('fetch_preview_proxy', { url });
      const { convertFileSrc } = await import('@tauri-apps/api/core');
      setSource((s) => ({ ...s, kind: 'proxy', url: convertFileSrc(path) }));
      setPhase('idle');
    } catch (e) {
      setError(`Could not prepare a preview: ${e}`);
      setPhase('error');
    }
  }, [url]);

  useEffect(() => {
    if (!url) return;
    let cancelled = false;

    (async () => {
      setPhase('probing');
      setError('');
      try {
        const result = await invoke('resolve_preview', { url });
        if (cancelled) return;
        setSource(result);
        if (result.kind === 'needs_proxy') {
          await loadProxy();
        } else {
          setPhase('idle');
        }
      } catch (e) {
        if (cancelled) return;
        // Errors are shown, not swallowed into console.warn as before.
        setError(String(e));
        setPhase('error');
      }
    })();

    return () => { cancelled = true; };
  }, [url, loadProxy]);

  // --- selection ------------------------------------------------------------

  const clamp = (t) => Math.max(0, Math.min(t, duration));

  // Report an ordered range so an out-point set before the in-point still works.
  const emit = useCallback((a, b) => {
    if (a == null || b == null) { onChange?.(null); return; }
    const [start, end] = a <= b ? [a, b] : [b, a];
    onChange?.(start === end ? null : { start, end });
  }, [onChange]);

  const setIn = (t) => { const v = clamp(t); setInPoint(v); emit(v, outPoint); };
  const setOut = (t) => { const v = clamp(t); setOutPoint(v); emit(inPoint, v); };
  const clearSelection = () => { setInPoint(null); setOutPoint(null); onChange?.(null); };

  useEffect(() => { setStartInput(inPoint != null ? formatTime(inPoint) : ''); }, [inPoint]);
  useEffect(() => { setEndInput(outPoint != null ? formatTime(outPoint) : ''); }, [outPoint]);

  const applyInput = (raw, apply) => {
    const secs = parseTimeToSeconds(raw);
    if (Number.isNaN(secs)) { setError('Invalid time. Use SS, MM:SS or HH:MM:SS'); return; }
    setError('');
    apply(secs);
  };

  // --- drag handling --------------------------------------------------------

  const timeFromClientX = (clientX) => {
    const rect = trackRef.current?.getBoundingClientRect();
    if (!rect || rect.width === 0) return 0;
    return clamp(((clientX - rect.left) / rect.width) * duration);
  };

  useEffect(() => {
    if (!dragging) return;

    const onMove = (e) => {
      const t = timeFromClientX(e.clientX);
      if (dragging === 'in') setIn(t); else setOut(t);
      if (videoRef.current) videoRef.current.currentTime = t;
    };
    const onUp = () => setDragging(null);

    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    return () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };
  });

  // --- keyboard -------------------------------------------------------------

  useEffect(() => {
    if (!ready) return;
    const onKey = (e) => {
      if (e.target.tagName === 'INPUT') return;
      const v = videoRef.current;
      if (!v) return;

      if (e.key === '[') { setIn(v.currentTime); e.preventDefault(); }
      else if (e.key === ']') { setOut(v.currentTime); e.preventDefault(); }
      else if (e.key === 'ArrowLeft')  { v.currentTime = clamp(v.currentTime - (e.shiftKey ? 0.1 : 1)); e.preventDefault(); }
      else if (e.key === 'ArrowRight') { v.currentTime = clamp(v.currentTime + (e.shiftKey ? 0.1 : 1)); e.preventDefault(); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  // --- geometry: guarded against a zero denominator -------------------------

  const pct = (t) => (duration > 0 ? Math.max(0, Math.min(100, (t / duration) * 100)) : 0);
  const selLeft = inPoint != null && outPoint != null ? pct(Math.min(inPoint, outPoint)) : null;
  const selWidth = inPoint != null && outPoint != null ? Math.abs(pct(outPoint) - pct(inPoint)) : 0;
  const clipLength = inPoint != null && outPoint != null ? Math.abs(outPoint - inPoint) : null;

  return (
    <div className="bg-gray-900 rounded-xl overflow-hidden shadow-2xl">
      <div className="bg-gray-800 p-4 border-b border-gray-700">
        <h3 className="text-white font-semibold">Trim</h3>
        <p className="text-gray-400 text-sm truncate">{source?.title || 'Loading…'}</p>
      </div>

      <div className="relative bg-black min-h-[16rem] flex items-center justify-center">
        {phase === 'error' ? (
          <div className="text-center p-6">
            <p className="text-red-400 mb-3">{error}</p>
            <button
              onClick={loadProxy}
              className="bg-gray-700 hover:bg-gray-600 text-white px-4 py-2 rounded-lg text-sm"
            >
              Try downloading a preview instead
            </button>
          </div>
        ) : phase === 'proxying' ? (
          <div className="text-center p-6 text-gray-300">
            <div className="animate-spin rounded-full h-10 w-10 border-b-2 border-red-500 mx-auto mb-3" />
            <p>Preparing preview…</p>
            <p className="text-gray-500 text-sm mt-1">This video has no directly playable stream.</p>
          </div>
        ) : source?.url ? (
          <video
            ref={videoRef}
            src={source.url}
            controls
            className="w-full max-h-96"
            onLoadedMetadata={(e) => {
              // Ground truth. Never yt-dlp's metadata.
              setDuration(e.currentTarget.duration || 0);
              setPhase('ready');
            }}
            onTimeUpdate={(e) => setCurrent(e.currentTarget.currentTime)}
            onError={() => {
              if (source.kind !== 'proxy') loadProxy();
              else { setError('Preview could not be played.'); setPhase('error'); }
            }}
          />
        ) : (
          <div className="text-gray-400 p-6">Loading preview…</div>
        )}
      </div>

      <div className="bg-gray-800 p-4">
        <div
          ref={trackRef}
          className={`relative h-8 bg-gray-700 rounded-lg ${ready ? 'cursor-pointer' : 'opacity-50 cursor-not-allowed'}`}
          onPointerDown={(e) => {
            if (!ready) return;
            const t = timeFromClientX(e.clientX);
            if (videoRef.current) videoRef.current.currentTime = t;
          }}
        >
          {selLeft != null && (
            <div
              className="absolute top-0 h-8 bg-green-500/30 border-x-2 border-green-400 pointer-events-none"
              style={{ left: `${selLeft}%`, width: `${selWidth}%` }}
            />
          )}

          <div className="absolute top-0 h-8 w-0.5 bg-white pointer-events-none" style={{ left: `${pct(current)}%` }} />

          {ready && inPoint != null && (
            <div
              role="slider" aria-label="Trim start" tabIndex={0}
              aria-valuemin={0} aria-valuemax={duration} aria-valuenow={inPoint}
              onPointerDown={(e) => { e.stopPropagation(); setDragging('in'); }}
              className="absolute -top-1 h-10 w-3 -ml-1.5 bg-green-400 rounded cursor-ew-resize"
              style={{ left: `${pct(inPoint)}%` }}
            />
          )}
          {ready && outPoint != null && (
            <div
              role="slider" aria-label="Trim end" tabIndex={0}
              aria-valuemin={0} aria-valuemax={duration} aria-valuenow={outPoint}
              onPointerDown={(e) => { e.stopPropagation(); setDragging('out'); }}
              className="absolute -top-1 h-10 w-3 -ml-1.5 bg-red-400 rounded cursor-ew-resize"
              style={{ left: `${pct(outPoint)}%` }}
            />
          )}
        </div>

        <div className="flex justify-between text-sm text-gray-400 mt-2">
          <span>{formatTime(current)}</span>
          <span>{ready ? formatTime(duration) : '—:—'}</span>
        </div>

        <div className="flex flex-wrap items-center gap-2 mt-4">
          <button disabled={!ready} onClick={() => setIn(videoRef.current?.currentTime ?? 0)}
            className="bg-green-600 hover:bg-green-500 disabled:opacity-40 text-white px-4 py-2 rounded-lg text-sm">
            Set Start <kbd className="ml-1 opacity-70">[</kbd>
          </button>
          <button disabled={!ready} onClick={() => setOut(videoRef.current?.currentTime ?? 0)}
            className="bg-red-600 hover:bg-red-500 disabled:opacity-40 text-white px-4 py-2 rounded-lg text-sm">
            Set End <kbd className="ml-1 opacity-70">]</kbd>
          </button>
          <button disabled={!ready} onClick={clearSelection}
            className="bg-gray-600 hover:bg-gray-500 disabled:opacity-40 text-white px-4 py-2 rounded-lg text-sm">
            Clear
          </button>
          {clipLength != null && (
            <span className="text-blue-300 text-sm ml-auto">Clip length: {formatTime(clipLength)}</span>
          )}
        </div>

        <div className="grid grid-cols-1 md:grid-cols-2 gap-4 mt-4">
          <div>
            <label className="block text-xs text-gray-400 mb-1">Start (SS, MM:SS or HH:MM:SS)</label>
            <input
              type="text" value={startInput} disabled={!ready}
              onChange={(e) => setStartInput(e.target.value)}
              onBlur={() => applyInput(startInput, setIn)}
              onKeyDown={(e) => { if (e.key === 'Enter') applyInput(startInput, setIn); }}
              className="w-full bg-gray-700 text-white px-3 py-2 rounded-lg border border-gray-600 focus:border-green-500 outline-none disabled:opacity-40"
            />
          </div>
          <div>
            <label className="block text-xs text-gray-400 mb-1">End (SS, MM:SS or HH:MM:SS)</label>
            <input
              type="text" value={endInput} disabled={!ready}
              onChange={(e) => setEndInput(e.target.value)}
              onBlur={() => applyInput(endInput, setOut)}
              onKeyDown={(e) => { if (e.key === 'Enter') applyInput(endInput, setOut); }}
              className="w-full bg-gray-700 text-white px-3 py-2 rounded-lg border border-gray-600 focus:border-red-500 outline-none disabled:opacity-40"
            />
          </div>
        </div>

        {error && phase !== 'error' && <p className="text-red-400 text-xs mt-2">{error}</p>}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Add the proxy-fetch command referenced by the component**

Add to `src-tauri/src/probe.rs` and register it in `invoke_handler`:

```rust
/// Downloads a small copy of the video for local scrubbing, for sources with no
/// directly playable stream. Cached by URL hash under the app cache directory.
#[tauri::command]
pub async fn fetch_preview_proxy<R: Runtime>(
    app_handle: AppHandle<R>,
    url: String,
) -> Result<String, String> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use tauri::Manager;

    let paths = crate::binary_manager::resolve_paths(&app_handle)?;
    crate::binary_manager::ensure_executable(&paths)?;

    let cache_dir = app_handle
        .path()
        .app_cache_dir()
        .map_err(|e| format!("No cache directory: {e}"))?
        .join("preview");
    std::fs::create_dir_all(&cache_dir).map_err(|e| format!("Cannot create cache dir: {e}"))?;

    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    let target = cache_dir.join(format!("{:x}.mp4", hasher.finish()));

    if target.exists() {
        return Ok(target.to_string_lossy().to_string());
    }

    let output = Command::new(&paths.yt_dlp)
        .arg("--no-playlist")
        .arg("-f")
        .arg("best[height<=360]/worst")
        .arg("--merge-output-format")
        .arg("mp4")
        .arg("--ffmpeg-location")
        .arg(&paths.ffmpeg)
        .arg("-o")
        .arg(&target)
        .arg(&url)
        .output()
        .map_err(|e| format!("Failed to fetch preview: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "Preview download failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(target.to_string_lossy().to_string())
}
```

- [ ] **Step 3: Move the stylesheet and delete the old component**

```bash
git mv src/VideoPreview.css src/components/TrimWorkbench.css
git rm src/VideoPreview.jsx
```

- [ ] **Step 4: Verify the build**

```bash
npm run build && cd src-tauri && cargo build 2>&1 | tail -20
```

Expected: both succeed. If `npm run build` reports a missing `VideoPreview` import, that is Task 12's `App.jsx` change — proceed to it.

- [ ] **Step 5: Commit**

```bash
git add src/components/TrimWorkbench.jsx src/components/TrimWorkbench.css src-tauri/src/probe.rs src-tauri/src/lib.rs
git rm --cached src/VideoPreview.jsx 2>/dev/null; true
git commit -m "feat(trim): rebuild preview as a real video workbench

Duration now comes from the <video> element rather than yt-dlp metadata,
which removes the two-position-slider failure. Adds draggable in/out
handles, [ and ] keys, and visible errors instead of a console.warn."
```

---

### Task 12: Wire `App.jsx` to the queue

**Files:**
- Modify: `src/App.jsx` — remove `isTrimMode`/`showVideoPreview` (lines 29-32, 201-207), the global progress state (lines 16-20), the event listeners (lines 110-148), and `startDownload` (lines 209-275)

**Interfaces:**
- Consumes: `useJobs` (Task 10), `TrimWorkbench` (Task 11)
- Produces: nothing — terminal UI wiring

- [ ] **Step 1: Replace the trim toggles and download handler**

In `src/App.jsx`:

- Delete `progress`, `speed`, `eta`, `status` state (lines 16-20) and the `download-progress` / `download-complete` / `download-error` listeners (lines 110-148). `useJobs` replaces all of it.
- Delete `isTrimMode`, `showVideoPreview`, `trimStartTime`, `trimEndTime`, `handleTimeSelect`, and `toggleTrimMode`. Two booleans toggled from one handler could desync and silently hide the panel.
- Replace with a single trim selection value:

```jsx
import { useJobs } from './hooks/useJobs';
import TrimWorkbench from './components/TrimWorkbench';

const { jobs, enqueue, cancel } = useJobs();
const [trim, setTrim] = useState(null);          // { start, end } | null
const [showTrim, setShowTrim] = useState(false); // panel visibility only
```

- Replace `startDownload` with:

```jsx
const startDownload = async () => {
  if (!url.trim()) {
    alert('Please enter a URL');
    return;
  }
  if (!outputFolder) {
    alert('Please select an output folder');
    return;
  }

  try {
    await enqueue({
      url,
      format: downloadType === 'mp3'
        ? { mode: 'quick', kind: 'mp3', height: null }
        : { mode: 'quick', kind: 'mp4', height: quality === 'best' ? null : Number(quality) },
      trim,
      outputFolder,
    });
  } catch (e) {
    alert(`Could not queue download: ${e}`);
  }
};
```

- Render the workbench and a minimal job list:

```jsx
{showTrim && <TrimWorkbench url={url} onChange={setTrim} />}

<div className="space-y-2">
  {jobs.map((job) => (
    <div key={job.id} className="flex items-center gap-3 bg-gray-800/50 p-3 rounded-lg">
      <div className="flex-1 min-w-0">
        <p className="text-sm truncate">{job.title || job.url}</p>
        <div className="h-1.5 bg-gray-700 rounded mt-1">
          <div className="h-1.5 bg-green-500 rounded" style={{ width: `${job.progress.percentage}%` }} />
        </div>
      </div>
      <span className="text-xs text-gray-400 w-20 text-right">{job.status}</span>
      {!['done', 'failed', 'cancelled'].includes(job.status) && (
        <button onClick={() => cancel(job.id)} className="text-xs text-red-400 hover:text-red-300">Cancel</button>
      )}
    </div>
  ))}
</div>
```

- [ ] **Step 2: Replace every remaining `isValidYouTubeUrl` and `status` reference**

`isValidYouTubeUrl` is used in **13 places**, and the deleted `status` state is referenced in the download button. Leaving any of them breaks the build. Enumerate them first:

```bash
grep -n "isValidYouTubeUrl\|status ===" src/App.jsx
```

Expected matches at lines 83, 152, 202, 211, 399, 402, 404, 407, 697, 699, 707, 727, 737, 739.

Replace the definition at line 152 with a host-agnostic check — the format picker in Plan B targets every yt-dlp site, and the YouTube-only regex blocks all of them:

```jsx
// yt-dlp supports 1000+ sites; the extractor decides what is downloadable, and
// reports back through the normal job error path. This only rejects input that
// is not a URL at all.
const isValidUrl = (value) => {
  try {
    const parsed = new URL(String(value).trim());
    return parsed.protocol === 'http:' || parsed.protocol === 'https:';
  } catch {
    return false;
  }
};
```

Then apply these replacements:

| Lines | Current | Replace with |
|---|---|---|
| 83 | `isValidYouTubeUrl(shared)` | `isValidUrl(shared)` |
| 202, 211 | inside `toggleTrimMode` / `startDownload` | deleted with those handlers in Step 1 |
| 399, 402, 404, 407 | URL-field validity styling and the "Please enter a valid YouTube URL" message | `isValidUrl(url)`; change the message text to `Please enter a valid URL` |
| 697, 699, 707 | `status === "downloading" \|\| !isValidYouTubeUrl(url) \|\| !outputFolder` | `!isValidUrl(url) \|\| !outputFolder` — the button no longer disables during download, because the queue accepts more jobs while one runs |
| 727, 737, 739 | readiness hints | `isValidUrl(url)` |

- [ ] **Step 3: Verify no references to removed symbols remain**

```bash
grep -n "isTrimMode\|showVideoPreview\|trimStartTime\|trimEndTime\|VideoPreview\|download-progress\|isValidYouTubeUrl\|status ===" src/App.jsx
```

Expected: **no matches.**

- [ ] **Step 4: Build and run the test suites**

```bash
npm test && npm run build && cd src-tauri && cargo test 2>&1 | tail -10
```

Expected: all green.

- [ ] **Step 5: Commit**

```bash
git add src/App.jsx
git commit -m "refactor(app): drive downloads through the job queue

Removes the desynchronising isTrimMode/showVideoPreview pair and the
global progress state that made concurrent downloads impossible."
```

---

### Task 13: Manual verification

Automated tests cover the pure logic. The download path needs network and real binaries, so it is verified by hand. **No completion claim may be made before this task passes.**

**Files:**
- Create: `docs/superpowers/notes/2026-08-24-manual-verification.md`

**Interfaces:**
- Consumes: everything
- Produces: a recorded pass/fail per check

- [ ] **Step 1: Launch the app**

```bash
npm run tauri:dev
```

- [ ] **Step 2: Work through the checks, recording the actual result for each**

| # | Check | Pass condition |
|---|---|---|
| 1 | Untrimmed mp4 download | Completes; aria2c multi-connection visible in logs |
| 2 | **Trimmed mp4** | **Output duration is within 0.5s of the selection — the spec §2.1 regression** |
| 3 | Trimmed mp3 | Audio clip matches the selected range |
| 4 | Preview on a standard video | Video plays and scrubs without a proxy download |
| 5 | Preview on a DASH-only/4K video | Falls back to proxy, shows "Preparing preview…", then scrubs |
| 6 | Drag the in/out handles | Selection follows the pointer; clip length updates live |
| 7 | Three queued jobs | At most 2 run concurrently; the third starts as one finishes |
| 8 | Cancel mid-download | Process dies; no orphaned `.part` files remain |
| 9 | Non-YouTube URL | Accepted; extractor errors surface in the UI |
| 10 | Metadata failure | Panel shows a visible error with a retry, not a dead slider |

For check 2, verify objectively rather than by eye:

```bash
BIN=src-tauri/binaries/macos-arm64
"$BIN/ffmpeg" -i "<downloaded file>" 2>&1 | grep Duration
```

- [ ] **Step 3: Record results and commit**

Write each check's actual observed result — including any failures — to `docs/superpowers/notes/2026-08-24-manual-verification.md`. Do not record a check as passing without having run it.

```bash
git add docs/superpowers/notes/2026-08-24-manual-verification.md
git commit -m "docs: record manual verification results for trim and queue rework"
```

---

## Plan B (deferred to a separate plan)

Phases 4–5 of the spec, to be planned after Plan A lands:

- Format picker (`list_formats`, Quick/Advanced modes, video-only + bestaudio pairing)
- Relaxed URL validation and playlist expansion into multiple jobs
- Metadata hardening in `get_video_metadata`
- Sidebar layout, history persistence via `plugin-store`, settings migration from `localStorage`
