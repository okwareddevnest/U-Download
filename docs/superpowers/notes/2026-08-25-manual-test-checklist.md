# Manual test checklist — trim rework

Run with `npm run tauri:dev`. Record the actual result, including failures.

Output folder: pick an empty one you can inspect. Binaries on disk are real
(yt-dlp 2026.08.19, ffmpeg 9.0.1).

---

## The three symptoms you originally reported

### 1. Cuts land where you set them  ← THE headline fix

- [ ] Paste a video URL, open trim, set a range of about 10 seconds, download.
- [ ] Check the result objectively, not by eye:

```bash
src-tauri/binaries/macos-arm64/ffmpeg -i "<downloaded file>" 2>&1 | grep Duration
```

**Pass:** duration within ~0.5 s of your selection. Automated verification measured
8 ms on a 5 s request; the old code was 2-10 **seconds** off.

### 2. You can see what you are cutting

- [ ] Video actually plays and scrubs in the panel.
- [ ] If it says "Preparing preview…" instead, that is the proxy fallback — expected,
      and now the *common* path for YouTube. It should finish and then scrub.

### 3. You can select start and end

- [ ] The scrub track spans the whole video, not 2 positions.
- [ ] Drag the green (in) and red (out) handles; the selection band follows.
- [ ] `[` and `]` set in/out at the playhead; arrows nudge 1 s, Shift+arrows 0.1 s.
- [ ] Type into the HH:MM:SS boxes; values apply.
- [ ] "Clip length" updates live.

---

## Fixes made after the final review — worth targeting

### 4. Trimmed downloads show real progress

- [ ] Start a trimmed download of something long enough to watch.

**Pass:** the bar climbs steadily. **Fail:** it sits at 0% then jumps to 100% —
that was the bug; yt-dlp routes section downloads through ffmpeg and its progress
was previously unparsed.

### 5. Cancel actually cancels

- [ ] Start a trimmed download, cancel mid-flight.
- [ ] Check the output folder.

**Pass:** no file appears, then or later. **Fail:** a file shows up seconds after you
cancelled — that means ffmpeg outlived the cancel.

### 6. Changing the URL mid-preview does not swap the video

- [ ] Paste URL A, let "Preparing preview…" start, then change the URL to B before it
      finishes.

**Pass:** the player ends up showing B, matching B's title. **Fail:** the player shows
A while the header says B — that would recreate symptom 1 by a different route.

### 7. Trim still possible when preview fails

- [ ] Find a URL whose preview fails (or disconnect briefly to force it).

**Pass:** the HH:MM:SS inputs remain usable so you can type timestamps.
**Fail:** everything is greyed out and you cannot trim at all.

---

## Queue behaviour

- [ ] Untrimmed mp4 downloads and completes.
- [ ] Trimmed mp3 produces audio matching the selection.
- [ ] Queue three downloads: at most **2** run at once; the third starts as one finishes.
- [ ] Completion plays a sound and shows a desktop notification.
- [ ] A failing job shows its error inline in the queue row (not a modal).
- [ ] A non-YouTube URL is accepted and either downloads or reports a real extractor error.

---

## Known limitations — not bugs, do not chase

- **Pause/resume are not wired to any UI control.** The backend supports them; the
  buttons were never added.
- **Trimmed jobs show a percentage but no ETA or speed.** ffmpeg reports a realtime
  ratio, which does not map honestly onto byte counters.
- **Cancel is Unix-only.** On macOS/Linux the whole process group is killed. On Windows
  the old behaviour remains and ffmpeg can outlive a cancel.
- **A cancelled trim may leave an unfinalised fragment** rather than nothing, since the
  kill is SIGKILL. Still better than the previous complete file.
- **"Processing" status is never shown**, so the final re-encode looks like a stalled
  100%.
- **The queue list never clears.** No dismiss control yet.
- **Android is not functional.** See the note in the session summary.
- **yt-dlp warns about a missing JS runtime** for YouTube and some formats vanish. This
  affects the shipped app generally, not just trimming.
