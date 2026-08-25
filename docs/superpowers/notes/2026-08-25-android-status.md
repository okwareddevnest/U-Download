# Android status — read this before resuming mobile work

**Decision (2026-08-25):** desktop first. Android code is **retained deliberately**,
not abandoned, and not deleted. This note records exactly what state it is in so the
next person does not have to rediscover it.

## Current state: retained, compiles, unreachable

The desktop trim/queue rework replaced the app's single global download state with a
job registry, and deleted the desktop download path. The Android path was kept behind
`#[cfg(target_os = "android")]` rather than ported or removed.

What that means concretely:

| Thing | State |
|---|---|
| `perform_download_android` | **776 lines**, present, compiles, **unreachable** |
| `start_download` command | registered in the Android `generate_handler!` list, never invoked |
| `DownloadProgress` / `ProgressState` | retained Android-only; the desktop equivalents are gone |
| `download-progress` / `-complete` / `-error` events | emitted by Android code only |
| Frontend references to any of the above | **zero** (`grep` across `src/` returns nothing) |
| `tauri.conf.json` bundle targets | `deb, rpm, nsis, dmg, appimage` — **no Android target** |
| `capabilities/mobile.json` | still present and valid |

**Why it is unreachable:** the React frontend is shared between desktop and Android.
It was rewired to the job queue (`enqueue_job`, `job-updated`/`job-done`/`job-failed`)
and no longer calls `start_download` or listens for the legacy events at all. So on
Android the app would call `enqueue_job`, which resolves binaries via
`binary_manager::resolve_paths` into an `android-arm64/` directory that
`scripts/fetch-binaries.sh` never populates and CI never builds.

**Net: Android would launch and then fail at the first download**, while 776 lines of
working legacy downloader sit right there, unable to be called. This is recorded
honestly because an earlier assessment of mine claimed Android would "keep the legacy
path" — it does not, and that was wrong.

## When you resume mobile, pick one

**Option A — port Android onto the job queue (recommended).** The queue is
platform-agnostic; what Android actually lacks is binaries.
1. Add `android-arm64` (and `android-arm` / `android-x64` if you want emulators) to
   `scripts/fetch-binaries.sh`. yt-dlp is a Python zipapp and will not run on Android as-is
   — this is the real work. Options: a termux-style static build, a Rust-native extractor,
   or a server-side extraction API.
2. Then delete `perform_download_android`, `start_download`, `DownloadProgress` and
   `ProgressState`, and drop `start_download` from the Android handler list. Android
   would use the same runner as desktop.
3. Add an Android leg to the release matrix.

**Option B — re-wire the frontend to the legacy path under `isAndroid`.** Cheaper, but
you would maintain two download implementations permanently, and Android would keep the
old broken trim (the `-c copy` keyframe-snapping bug this whole rework removed).

**Option C — delete the Android code now.** Only if mobile is genuinely off the roadmap.
It is recoverable from git history either way.

Option A is the right shape. Option B is a trap: it re-introduces the exact bug the
desktop work exists to fix.

## Do not be misled by

- **`capabilities/mobile.json` existing** — that is permissions config, not a working build.
- **The code compiling** — every cfg-gated symbol is defined and the gating is correct
  (`serde` and `Emitter` are properly Android-scoped). Compiling was the goal of retaining
  it; reachability was not achieved and was never claimed after the correction.
- **`android-arm64` appearing in `binary_manager.rs`'s platform matcher** — the resolver
  knows the name; nothing ever puts binaries there.
