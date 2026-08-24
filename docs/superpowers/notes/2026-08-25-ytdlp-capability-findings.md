# Task 0 — yt-dlp capability findings (empirical)

**Date:** 2026-08-25
**Status:** COMPLETE. Both release blockers closed, one new required fix identified.

## How this was run

`git lfs pull` failed — the repository's LFS budget is exhausted:

```
batch response: This repository exceeded its LFS budget.
```

So the bundled binaries in `src-tauri/binaries/` remain 3-line pointer stubs and could
not be executed. Verification instead used:

- **yt-dlp 2026.08.19**, the official `yt-dlp_macos` release asset, downloaded to a
  scratch directory. Not from LFS, and the repository was never touched.
- **ffmpeg 8.0.1** (system, Homebrew).
- A **stub `aria2c`** placed ahead on `PATH`, which reports its arguments and exits —
  this reveals exactly what yt-dlp passes without needing a real aria2c.

Caveat: the bundled yt-dlp may differ in version from 2026.08.19. Re-run against the
real binary once the LFS situation is resolved.

## Q1 — Do `--download-sections` and `--force-keyframes-at-cuts` produce an accurate cut?

**YES. Verified to 8 milliseconds.**

```
yt-dlp --download-sections "*00:00:05.000-00:00:10.000" --force-keyframes-at-cuts \
       -f "bestvideo[height<=480]+bestaudio/best[height<=480]" ...
```

| | |
|---|---|
| Requested | 5.000 s |
| Actual (`ffprobe`) | **5.008 s** |
| Deviation | **0.008 s** |
| Threshold | ≤ 0.5 s → **PASS** |

For comparison, the implementation this replaces (`-ss` after `-i` plus `-c copy`) snapped
cuts to the nearest keyframe, typically **2-10 seconds** off. The core premise of the
whole rework is confirmed.

## Q2 — Is `--download-sections` compatible with `--external-downloader aria2c`?

**The question is moot: with `--download-sections`, yt-dlp never uses the external
downloader at all.** It routes the ranged fetch through ffmpeg.

Evidence: the section download above completed successfully on a machine with **no
aria2c installed**, while `--external-downloader aria2c` was passed. No error, no
warning. ffmpeg performed the fetch (its `frame=`/`Lsize=` progress is in the output).

**Consequence for the code:** omitting the aria2c flags on trimmed jobs (as
`build_download_args` does) is correct and harmless — but so would keeping them be.
The `trimmed_job_omits_aria2c` test is not wrong, merely stricter than yt-dlp requires.
**No code change needed.**

## Q3 — Does aria2c emit progress output that can be parsed?

**YES, but redrawn with `\r` and never newline-terminated. This makes the deferred
CR-flattening fix REQUIRED, not optional.**

The stub captured yt-dlp's exact aria2c invocation:

```
--no-conf --auto-save-interval=10 --console-log-level=warn
--summary-interval=0 --download-result=hide --http-accept-gzip=true
--file-allocation=none -x16 -j16 -s16 --min-split-size 1M
... --check-certificate=true --show-console-readout=true ...
-x 16 -s 16 -k 1M        <- user --external-downloader-args, appended LAST
```

Two things follow:

1. **`--summary-interval=0` disables the periodic summary block, but
   `--show-console-readout=true` keeps the live single-line readout.** So aria2c does
   emit `[#gid 1.2MiB/10MiB(12%) CN:16 DL:2.1MiB ETA:4s]`. The aria2c branch added to
   `parse_progress_line` is correct and necessary.
2. **aria2c redraws that readout with a carriage return, not a newline.** Rust's
   `BufReader::lines()` yields nothing until it sees `\n`, so every update buffers and
   replays at once when the pipe closes — progress would still appear frozen at 0%
   and then jump.

**Required fix:** read the child's stdout at byte level and split on **both** `\r` and
`\n`. The current `flat_map(|line| line.split('\r'))` is insufficient because it only
runs after `lines()` has already yielded.

**Also note:** user `--external-downloader-args` are appended AFTER yt-dlp's defaults,
so `--summary-interval=1` could be forced if a periodic summary is ever wanted. Not
needed given `--show-console-readout=true` is already on.

## Q4 (unplanned) — YouTube extraction now warns about a missing JS runtime

```
WARNING: [youtube] No supported JavaScript runtime could be found. Only deno is
enabled by default ... YouTube extraction without a JS runtime has been deprecated,
and some formats may be missing.
```

This is **not** a defect in this rework, but it affects the shipped app: the bundled
yt-dlp will hit the same warning on end users' machines, and "some formats may be
missing" can mean the requested quality silently degrades. A concrete symptom was seen
during testing: `-f "best[height<=480]"` failed with "Requested format is not
available", while `-f "bestvideo[height<=480]+bestaudio/best[height<=480]"` succeeded —
muxed formats are among those that go missing.

Worth tracking separately. See https://github.com/yt-dlp/yt-dlp/wiki/EJS
