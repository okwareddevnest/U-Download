# U-Download: Stacher7-class Downloader with Correct Trimming

**Date:** 2026-08-24
**Status:** Approved design, ready for implementation planning
**Applies to:** U-Download v2.2.5 → v3.0.0

---

## 1. Motivation

U-Download today downloads one video at a time with fixed quality presets, and its
trim feature is unreliable in three distinct ways that the user confirmed by testing:

1. **Cuts land off-target.** The trimmed file starts seconds away from the selected point.
2. **You cannot see what you are cutting.** The preview is a static thumbnail, not video.
3. **You cannot select start and end.** The scrub control frequently offers only two positions.

These are not three bugs; they are two. Symptoms 2 and 3 share a single root cause,
and symptom 1 is independent and lives in the backend.

The goal is to reach functional parity with Stacher7 for the capabilities the user
prioritised — a real download queue with history, a real format picker, and a trim
workbench that works — while fixing the trim defects at the correct layer rather
than patching around them.

---

## 2. Root cause analysis

### 2.1 Off-target cuts — `src-tauri/src/lib.rs:891-980`

`perform_trimming` builds this FFmpeg invocation:

```rust
ffmpeg_cmd.arg("-i").arg(&temp_path);          // line 925 — input FIRST
if let Some(start) = start_time {
    ffmpeg_cmd.arg("-ss").arg(format!("{}", start));   // line 929 — seek AFTER input
}
if let Some(end) = end_time {
    ffmpeg_cmd.arg("-t").arg(format!("{}", end - start_time.unwrap_or(0.0)));
}
ffmpeg_cmd.arg("-c").arg("copy");              // line 936 — stream copy
```

`-c copy` cannot re-encode, so FFmpeg can only cut on an existing keyframe. It
silently snaps the requested cut to the nearest keyframe, which on typical YouTube
encodes is 2–10 seconds away. The trim "succeeds" and produces a file at the wrong
offset. This is the sole cause of symptom 1.

### 2.2 Whole-video download before trimming — `src-tauri/src/lib.rs:513-519`

```rust
let temp_output_pattern = if trimming_enabled {
    format!("{}/%(title)s_temp.%(ext)s", output_folder)
} else {
    format!("{}/%(title)s.%(ext)s", output_folder)
};
```

Extracting 30 seconds from a 40-minute video downloads all 40 minutes first.

### 2.3 Fragile temp-file discovery — `src-tauri/src/lib.rs:905-918`

```rust
.filter(|entry| entry.file_name().to_string_lossy().contains("_temp"))
```

then unconditionally `&temp_files[0]`. A leftover `_temp` file from a crashed run, a
`.part` fragment, or a second concurrent download causes the wrong file to be trimmed.
The final filename is produced by `file_name_str.replace("_temp", "")`, which also
corrupts any title legitimately containing that substring.

### 2.4 Dead scrub control — `src/VideoPreview.jsx:225` with `src-tauri/src/lib.rs:228`

The backend defaults duration to zero on any metadata miss:

```rust
let duration = metadata["duration"].as_f64().unwrap_or(0.0);   // lib.rs:228
```

The frontend then derives the slider bound from that value:

```jsx
max={Math.max(1, Math.floor(duration || 0))}                   // VideoPreview.jsx:225
```

When `duration` is `0.0`, `max` becomes `1`. **The scrubber has exactly two positions,
0s and 1s.** This is symptom 3, exactly as reported.

Metadata legitimately fails in several common cases:

- **Playlist URLs.** `--dump-json` (line 208) emits one JSON object *per entry*.
  `serde_json::from_str` at line 220 rejects concatenated objects and returns `Err`.
- **Age-gated / bot-checked videos**, where yt-dlp exits non-zero (line 214).
- **Any extractor** that omits a `duration` field.

The failure is invisible. `loadVideoData` swallows it into a `console.warn`
(`src/VideoPreview.jsx:31-44`) and renders a thumbnail over a dead slider.

`duration = 0` additionally poisons the marker geometry at `VideoPreview.jsx:227-228`:
`(startTime / duration) * 100` evaluates to `NaN%`, so the green selection bar never
paints — contributing to symptom 2.

### 2.5 No real preview — `src/VideoPreview.jsx:183-205`

The component renders `<img src={videoData.thumbnail_url}>`. There is no `<video>`
element anywhere in the codebase. Cut points are chosen blind. This is symptom 2.

### 2.6 Desynchronising trim toggles — `src/App.jsx:201-207`

```jsx
setIsTrimMode(!isTrimMode);
setShowVideoPreview(!showVideoPreview);
```

Two independent booleans toggled from one handler. Any path that changes one without
the other leaves `isTrimMode === true` while `isVisible === false`, at which point
`VideoPreview` early-returns `null` (line 135) and the trim panel silently vanishes.

### 2.7 Single global progress state — `src-tauri/src/lib.rs:41`

```rust
type ProgressState = Arc<Mutex<DownloadProgress>>;
```

One mutex, one `download-progress` event, no job identity. This blocks every queue,
history, and per-item feature in this spec. It is the first thing that must change.

---

## 3. Scope

**In scope (this milestone):**

- Job registry and download queue with history
- Real yt-dlp format picker
- Trim workbench with real video playback and accurate cuts

**Explicitly deferred:**

- Subtitles, embedded thumbnail/metadata/chapters
- SponsorBlock segment removal
- `--cookies-from-browser`

> **Deferral consequence:** cookies-from-browser is also the fix for metadata failing
> on age-gated and bot-checked videos (§2.4). Some "preview will not load" cases will
> persist until it lands. The proxy fallback (§5.3) mitigates but does not eliminate this.

---

## 4. Architecture

### 4.1 Job model

Replaces `DownloadProgress` as the unit of state.

```rust
pub type JobId = String;                    // uuid v4

pub struct Job {
    pub id: JobId,
    pub url: String,
    pub title: String,
    pub thumbnail: String,
    pub duration: Option<f64>,
    pub format: FormatChoice,
    pub trim: Option<TrimRange>,
    pub status: JobStatus,
    pub progress: JobProgress,
    pub output_path: Option<PathBuf>,
    pub error: Option<String>,
    pub created_at: u64,                    // epoch millis
}

pub enum FormatChoice {
    Quick { kind: MediaKind, height: Option<u32> },   // mp4/mp3 + 360..1080/best
    Exact { format_id: String },                      // from the format table
}

pub struct TrimRange { pub start: f64, pub end: f64 }

pub enum JobStatus {
    Queued, Probing, Downloading, Processing,
    Done, Failed, Cancelled, Paused,
}

pub struct JobProgress {
    pub percentage: f64,
    pub speed_bytes_per_sec: u64,
    pub eta_seconds: Option<u64>,
    pub bytes_downloaded: u64,
    pub total_bytes: u64,
}
```

`TrimRange` is a **field on a job**, not an application mode. `isTrimMode` and
`showVideoPreview` (§2.6) are both deleted.

### 4.2 Registry and cancellation

```rust
pub struct JobHandle {
    pub job: Job,
    pub child: Option<std::process::Child>,   // for kill-on-cancel
}

pub type Jobs = Arc<Mutex<HashMap<JobId, JobHandle>>>;
```

Cancelling kills the child process, marks the job `Cancelled`, and removes partial
output plus any `.part` fragments.

### 4.3 Events

`download-progress`, `download-complete`, and `download-error` are replaced by
job-scoped events. Every payload carries `job_id`.

| Event | Payload |
|---|---|
| `job-updated` | full `Job` (status changes, progress ticks) |
| `job-done` | `{ job_id, output_path, title }` |
| `job-failed` | `{ job_id, error }` |

The frontend routes by `job_id` into a keyed map. Global `useState` progress
variables in `App.jsx:16-20` are removed.

> Progress ticks are coalesced to at most one `job-updated` per job per 500ms to avoid
> flooding the webview bridge when several jobs run concurrently.

### 4.4 Scheduler

- Default **2** concurrent jobs, user-settable 1–5 in Settings.
- aria2c connection count scales inversely with concurrency so the link is not
  saturated: `-x`/`-s` = `clamp(16 / concurrency, 4, 16)`.
- FIFO with manual reorder.

**Pause semantics are explicitly non-suspending.** There is no process-level suspend
and no resume-from-byte-offset in this milestone:

| Action on a job in state | Effect |
|---|---|
| `Queued` → pause | Becomes `Paused`; scheduler skips it; no process involved |
| `Paused` → resume | Returns to `Queued`; scheduled normally |
| `Downloading` → pause | **Child process is killed**; partial output discarded; job becomes `Paused` and restarts from zero when resumed |

Resuming a partially-downloaded job from its byte offset is deliberately out of scope.
The UI must label the in-flight case as such rather than implying progress is preserved.

### 4.5 Module layout

`App.jsx` is 759 lines and `lib.rs` is 1844. Both already exceed what is comfortably
maintainable and this work would worsen that. The split is part of this change, not a
separate refactor. No unrelated restructuring is included.

**Backend — `src-tauri/src/`**

| Module | Responsibility |
|---|---|
| `lib.rs` | Tauri setup, tray, command registration only |
| `jobs.rs` | `Job`, `JobStatus`, registry, event emission |
| `queue.rs` | Scheduler, concurrency, cancel/retry/reorder |
| `ytdlp.rs` | Argument construction, progress-line parsing |
| `probe.rs` | Metadata, format listing, preview-URL resolution |
| `binary_manager.rs` | Unchanged |

**Frontend — `src/`**

| Path | Responsibility |
|---|---|
| `App.jsx` | Shell, sidebar routing |
| `components/Sidebar.jsx` | Download · Queue · History · Settings |
| `components/AddUrlBar.jsx` | URL entry, paste-many, playlist expansion |
| `components/QueueList.jsx` · `QueueItem.jsx` | Queue rendering, per-item controls |
| `components/FormatPicker.jsx` | Quick/Advanced format selection |
| `components/TrimWorkbench.jsx` | Video playback, in/out handles |
| `components/HistoryList.jsx` · `Settings.jsx` | History, preferences |
| `hooks/useJobs.js` | Event subscription, job map, routing by `job_id` |
| `lib/time.js` · `lib/format.js` | Time parse/format, format-table helpers |

`lib/time.js` reuses the existing `parseTimeToSeconds` implementation
(`VideoPreview.jsx:56-73`) unchanged — it is correct and already handles SS, MM:SS,
and HH:MM:SS. `formatTime` (line 47) is extended to emit `H:MM:SS` for durations of
an hour or more, which it currently does not.

---

## 5. Trimming

### 5.1 Primary strategy

`perform_trimming` (`lib.rs:891-980`) is **deleted**. Trimming moves into yt-dlp:

```
yt-dlp --download-sections "*START-END" --force-keyframes-at-cuts
```

- Fetches only the requested byte range, eliminating the full-download wait (§2.2).
- `--force-keyframes-at-cuts` re-encodes only the boundary GOPs and stream-copies the
  interior, so the cut lands exactly where requested while remaining fast (§2.1).
- Output goes directly to the final filename. The `_temp` discovery scheme (§2.3) is
  removed entirely, along with its whole class of wrong-file bugs.

Timestamps are emitted as `HH:MM:SS.mmm`.

### 5.2 Known constraints

**These must be verified against the bundled yt-dlp before implementation, not assumed.**

1. **`--download-sections` is expected to be incompatible with
   `--external-downloader aria2c`.** A trimmed job therefore omits the aria2c flags
   (`lib.rs:470-474`) and uses yt-dlp's native ranged fetch. Untrimmed jobs keep aria2c.
2. **Not every extractor supports sections.**

Verification is Task 0 of the implementation plan, run against the real binary, with
the observed behaviour recorded before any code depends on it.

### 5.3 Fallback

Where `--download-sections` is unsupported, fall back to full download followed by an
FFmpeg pass that is correct by construction:

```
ffmpeg -ss <start> -i <input> -t <duration> -c:v libx264 -c:a aac <output>
```

`-ss` **before** `-i` for fast seeking, and a genuine re-encode. `-c copy` is never
used for trimming anywhere in the codebase after this change.

### 5.4 Audio

`mp3` jobs combine `-x --audio-format mp3` with the same `--download-sections`.
Audio has no keyframe constraint, so cuts are sample-accurate.

---

## 6. Preview: hybrid stream-then-proxy

Chosen by the user over stream-only, proxy-only, and storyboard-only alternatives.

```
probe_preview(url)
  └─ pick muxed format: vcodec != "none" && acodec != "none", prefer height <= 480
       │
       ├─ found → <video src="https://...googlevideo.com/...">
       │            ├─ onLoadedMetadata → ready, scrub immediately
       │            └─ onError ─────────────┐
       │                                     │
       └─ no muxed format ──────────────────┤
                                             ▼
                                    fetch_proxy(url)
                                      yt-dlp -f "best[height<=360]"
                                        -o <cache_dir>/preview/<hash>.mp4
                                      → convertFileSrc()
                                      → "Preparing preview…" (30s–2min)
                                      → exact local scrubbing
```

The fallback covers DASH-only videos (common for 4K and long uploads, which have no
muxed format at all), expired or IP-bound stream URLs, and non-YouTube extractors.

Remote media loads unblocked because `app.security.csp` is `null`
(`src-tauri/tauri.conf.json:29`). **This is a deliberate dependency.** If a CSP is
introduced later it must include `media-src` for the extractor's CDN hosts, or
streaming preview breaks and every video silently takes the slow proxy path.

Proxy files live under the app cache directory, keyed by a hash of the URL, and are
swept on startup with a 24-hour age cutoff.

### 6.1 Duration is taken from the video element

**The `<video>` element's own `duration` property is the source of truth, not yt-dlp
metadata.** yt-dlp's duration becomes an optional hint used only for the pre-load
placeholder.

This structurally eliminates §2.4: the scrubber is disabled until `loadedmetadata`
fires, and its bound then comes from a value the browser derived by actually decoding
the stream. The `Math.max(1, ...)` collapse cannot recur, because there is no longer a
code path where a zero from the backend reaches the slider bound.

Marker geometry is guarded against a zero or `NaN` denominator, fixing the invisible
selection bar at `VideoPreview.jsx:227-228`.

### 6.2 Workbench interaction

- Draggable **in** and **out** handles on the scrub track. The current
  "Set Start"/"Set End" buttons (`VideoPreview.jsx:250-277`) are retained as a
  secondary path, not the only one.
- `[` sets in-point at playhead, `]` sets out-point.
- `←`/`→` nudge by 1s; `Shift+←`/`Shift+→` by 0.1s.
- The existing HH:MM:SS text inputs are kept as-is; their parsing is already correct.
- Live readout of the resulting clip length.
- Selecting an out-point before the in-point swaps them rather than erroring.
- **Errors render in the panel.** The `console.warn` swallow at `VideoPreview.jsx:31-44`
  is replaced by a visible message with a retry action.

---

## 7. Format picker

New command `list_formats(url) -> Vec<FormatRow>`, parsing `formats[]` from
`--dump-json`:

```rust
pub struct FormatRow {
    pub format_id: String,
    pub ext: String,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub vcodec: String,
    pub acodec: String,
    pub tbr: Option<f64>,
    pub filesize: Option<u64>,          // or filesize_approx
    pub is_muxed: bool,                 // vcodec != none && acodec != none
}
```

Two modes, mirroring Stacher:

- **Quick** (default) — the existing 360/480/720/1080/best presets, unchanged behaviour.
- **Advanced** — the full sortable table.

An `Exact { format_id }` choice that is video-only is automatically paired with the
best available audio (`<id>+bestaudio`) so the user cannot accidentally produce a
silent file.

### 7.1 URL validation is relaxed

`isValidYouTubeUrl` (`App.jsx:152-155`) currently hard-blocks every non-YouTube host.
A format table across yt-dlp's 1000+ supported sites is incompatible with that.
Validation becomes a well-formed `http(s)` URL check; the extractor decides whether it
is supported, and reports back through the normal error path.

The Android share-intent handler (`App.jsx:81-86`) uses the same relaxed check.

---

## 8. Metadata robustness

Independent of the preview fix, `get_video_metadata` (`lib.rs:201-249`) is hardened:

- Use `--dump-single-json`, which emits **one** object for playlists instead of the
  concatenated stream that breaks `serde_json::from_str` at line 220 (§2.4).
- Detect a playlist payload (`_type == "playlist"`) and expand entries into individual
  queued jobs rather than failing.
- `duration` becomes `Option<f64>` — `None`, not a silently wrong `0.0` (line 228).
- Metadata errors propagate to the UI with the yt-dlp stderr attached.
- Probing runs off the UI path so the interface never blocks on a slow `--dump-json`.

---

## 9. History and settings

Completed and failed jobs persist via `@tauri-apps/plugin-store` — already a declared
dependency (`package.json`) and permitted capability
(`src-tauri/capabilities/default.json`), currently unused for this purpose. Records
retain url, title, format, trim range, output path, size, and timestamp, with actions
to re-download, reveal in file manager, and clear.

Settings migrate from scattered `localStorage` keys (`App.jsx:96-108`) into the same
store: output folder, concurrency, default format mode, theme.

---

## 10. Testing

**The repository has no test infrastructure today.** This is stated plainly rather
than implied otherwise.

**Added — Rust unit tests over pure functions.** These are precisely where the reported
defects originated, and all are testable without network or binaries:

| Target | Why |
|---|---|
| Trim argument construction | Directly regression-tests §2.1. Asserts `-c copy` never appears on a trim path and that `-ss` precedes `-i` in the fallback. |
| Format-selector construction | Video-only ids get `+bestaudio`; presets map correctly. |
| Progress-line parsing | Existing regex logic, currently untested. |
| Time parse/format round-trip | Covers SS, MM:SS, HH:MM:SS, and the new `H:MM:SS` output. |
| Section-timestamp formatting | `HH:MM:SS.mmm` boundaries and fractional seconds. |

**Not automated.** Real downloads, extractor behaviour, and UI interaction require
network and the LFS binaries. These are covered by a written manual checklist:

1. Untrimmed mp4 download completes with aria2c acceleration.
2. Trimmed mp4 — **verify cut lands within 0.5s of the selection** (the §2.1 regression).
3. Trimmed mp3.
4. Preview streams directly on a standard video.
5. Preview falls back to proxy on a DASH-only/4K video.
6. Playlist URL expands to multiple queued jobs (the §2.4 parse failure).
7. Three queued jobs respect the concurrency limit of 2.
8. Cancel mid-download kills the process and removes partial files.
9. Non-YouTube URL is accepted and reports extractor errors cleanly.
10. History survives an app restart.

No claim of automated coverage over the download path will be made.

---

## 11. Prerequisite

**`git lfs pull` must run before any local build or verification.**

The bundled binaries are unpulled LFS pointers. `src-tauri/binaries/macos-arm64/yt-dlp`
is a 3-line text stub:

```
version https://git-lfs.github.com/spec/v1
oid sha256:bb3a68c1c1397f4fe8b373970148239e2d546b246711a50c0ca71264bfec5988
size 35709776
```

Executing it produces `line 1: version: command not found`. Until the pull completes,
§5.2 cannot be verified and nothing runs locally.

---

## 12. Migration and risk

| Risk | Mitigation |
|---|---|
| `--download-sections` behaves differently than expected | Verified as Task 0 against the real binary before code depends on it; §5.3 fallback exists regardless |
| Stream preview blocked or throttled | Proxy fallback (§5.3) is a required part of the design, not an optional extra |
| Bundled yt-dlp too old for `--force-keyframes-at-cuts` | Version-check in Task 0; bump the bundled binary if needed |
| Queue rewrite regresses single-download behaviour | A single job is just a queue of length one; manual checklist item 1 covers it |
| Existing users lose `localStorage` settings | One-time migration read from `localStorage` into the store on first v3 launch |

Version target **v3.0.0** — the event contract (`download-progress` → `job-updated`)
and the command surface both change incompatibly.

---

## 13. Implementation phasing

This milestone is large enough that a single undifferentiated plan would be difficult
to review or land safely. The implementation plan must be phased, with the app left
working and verifiable at the end of each phase.

| Phase | Delivers | Verifiable at end of phase |
|---|---|---|
| **0** | `git lfs pull`; verify §5.2 constraints against the real yt-dlp binary; record observed behaviour | Documented answer on aria2c/sections compatibility and `--force-keyframes-at-cuts` support |
| **1** | Job registry, queue, scheduler, job-scoped events; existing UI rewired to a queue of length one | Checklist items 1, 7, 8 |
| **2** | Trim via `--download-sections`; `perform_trimming` deleted; Rust unit tests | Checklist items 2, 3 — **including the ≤0.5s cut-accuracy assertion** |
| **3** | Preview workbench: `<video>`, hybrid stream/proxy, in/out handles, duration from element | Checklist items 4, 5 |
| **4** | Format picker, relaxed URL validation, metadata hardening, playlist expansion | Checklist items 6, 9 |
| **5** | Sidebar layout, history, settings migration | Checklist item 10 |

Phases 2 and 3 together close all three reported trim defects; phase 2 fixes off-target
cuts, phase 3 fixes both "cannot see" and "cannot select". Neither depends on phases 4–5,
so the trim feature can be exercised before the parity work completes.
