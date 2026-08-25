use serde::{Deserialize, Serialize};
use std::process::Command;
use tauri::{AppHandle, Runtime};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewSource {
    /// "stream" when a directly playable URL was found, "needs_proxy" otherwise.
    ///
    /// Deliberately still only those two values: the frontend branches on this
    /// string, and a video-only stream is still a stream as far as loading it
    /// goes. What differs is `has_audio`.
    pub kind: String,
    pub url: Option<String>,
    /// False when the chosen stream carries picture but no sound. A video-only
    /// H.264 stream is perfectly good for placing cut points, and the frontend
    /// says so rather than letting silence read as a fault.
    ///
    /// Meaningless when `kind` is "needs_proxy" (no stream was chosen); reported
    /// as true there because the proxy the frontend then fetches is muxed.
    pub has_audio: bool,
    /// URL of a compatible audio-only format to play alongside a silent
    /// video-only `url`, so the frontend can sync a separate `<audio>` element
    /// to the `<video>` instead of waiting on a proxy download for sound.
    ///
    /// Always `None` when `has_audio` is true (a muxed pick already carries
    /// its own sound) and when `kind` is "needs_proxy" (no stream was chosen
    /// at all). See `pick_preview_audio_url`.
    pub audio_url: Option<String>,
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

/// Selects a video-only H.264 format the webview can decode on its own.
///
/// Modern YouTube has largely retired muxed formats — a typical video now
/// exposes dozens of formats and not one with both codecs — so demanding audio
/// pushed every preview onto the slow full-download proxy path. A `<video>`
/// element will happily play a video-only, range-served mp4: no sound, but
/// instant and seekable, which is all placing a cut point needs.
///
/// H.264 specifically, not merely the best video-only format: the preview plays
/// in a WKWebView on macOS, which does not reliably decode VP9. A stream that
/// downloads but will not play is worth nothing here.
///
/// Prefers the highest height at or below 480p, and at equal height a
/// progressive stream over an HLS manifest — a single range-served file seeks
/// more predictably than a playlist. A format with no `protocol` field is taken
/// to be progressive, that being the ordinary case for a plain media URL.
pub fn pick_video_only_format(formats: &serde_json::Value) -> Option<serde_json::Value> {
    let arr = formats.as_array()?;

    let mut candidates: Vec<&serde_json::Value> = arr
        .iter()
        .filter(|f| {
            let v = f["vcodec"].as_str().unwrap_or("none");
            let a = f["acodec"].as_str().unwrap_or("none");
            v.starts_with("avc1") && a == "none" && f["url"].as_str().is_some()
        })
        .collect();

    if candidates.is_empty() {
        return None;
    }

    // Ascending, so the reverse scan below finds the tallest qualifying format,
    // and among equal heights the progressive one (sorted last).
    candidates.sort_by_key(|f| {
        let height = f["height"].as_u64().unwrap_or(0);
        let progressive = !f["protocol"].as_str().unwrap_or("https").contains("m3u8");
        (height, progressive)
    });

    let best = candidates
        .iter()
        .rev()
        .find(|f| f["height"].as_u64().unwrap_or(0) <= 480)
        .or_else(|| candidates.first())?;

    Some((*best).clone())
}

/// The preview source, in order of preference: a muxed stream (picture and
/// sound), then a video-only H.264 stream (picture alone), then nothing — for
/// which the caller falls back to downloading a proxy.
///
/// The bool is whether the chosen format carries audio.
pub fn pick_preview_format(formats: &serde_json::Value) -> Option<(serde_json::Value, bool)> {
    if let Some(f) = pick_muxed_format(formats) {
        return Some((f, true));
    }
    pick_video_only_format(formats).map(|f| (f, false))
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
    //
    // Run on a blocking thread: yt-dlp's network fetch + extraction can take
    // several seconds, and this is an async fn — calling Command::output()
    // inline would block whichever runtime thread services this future,
    // stalling other concurrent Tauri commands (the job queue now runs
    // downloads concurrently on that same runtime).
    let yt_dlp = paths.yt_dlp.clone();
    // yt-dlp deprecates runtime-less YouTube extraction and warns loudly about
    // it, so a runtime is passed when one is available. It is not what makes
    // previews work: measured on a real YouTube URL the muxed-format count is
    // zero with a runtime and zero without — see `pick_video_only_format`, which
    // is the reason this path now resolves at all. `None` here means the machine
    // has no runtime; the invocation is then exactly what it was before.
    let js_runtime = crate::binary_manager::resolve_js_runtime(&paths);
    // The directory the bundled tools live in, for `augment_path_env` below.
    let binaries_dir = paths.dir.clone();
    let output = tokio::task::spawn_blocking(move || {
        let mut args: Vec<String> = Vec::new();
        crate::binary_manager::push_js_runtime_args(&mut args, js_runtime.as_ref());
        let mut cmd = Command::new(&yt_dlp);
        // Puts the bundled tools on PATH and, crucially, SSL_CERT_FILE in the
        // environment: yt-dlp may reach for ffmpeg here, and the bundled
        // ffmpeg cannot verify a single HTTPS certificate without it.
        crate::binary_manager::augment_path_env(&mut cmd, &binaries_dir);
        cmd.args(&args)
            .arg("--dump-single-json")
            .arg("--no-playlist")
            .arg("--no-download")
            .arg(&url)
            .output()
    })
    .await
    .map_err(|e| format!("Probe task failed: {e}"))?
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

    // A URL that turns out not to play is not a dead end: the frontend's
    // <video onError> falls back to `fetch_preview_proxy`. That matters most for
    // the video-only branch, where playability is likely but not guaranteed for
    // every itag.
    match pick_preview_format(&meta["formats"]) {
        Some((f, has_audio)) => Ok(PreviewSource {
            kind: "stream".to_string(),
            url: f["url"].as_str().map(|s| s.to_string()),
            has_audio,
            duration,
            title,
            thumbnail,
        }),
        None => Ok(PreviewSource {
            kind: "needs_proxy".to_string(),
            url: None,
            // No stream was chosen; the proxy the frontend fetches next is muxed.
            has_audio: true,
            duration,
            title,
            thumbnail,
        }),
    }
}

/// Format selector for the preview proxy download.
///
/// `best[height<=360]` — what this used to be — asks for a *muxed* stream at
/// that height, and modern YouTube has essentially none: on a real URL that
/// selector fails outright with "Requested format is not available", which is
/// what made the fallback path fail alongside the primary one. Asking for
/// separate streams and letting ffmpeg mux them resolves.
///
/// avc1 + mp4a first because the result is played back in a WKWebView, which
/// does not reliably decode VP9. That is a *preference*: the later branches are
/// what keeps an avc1-less source working at all, so they stay.
///
/// Written as one continued literal; `\` at a line end swallows the newline and
/// the following indentation, and `selector_is_a_single_unbroken_token` holds it
/// to that.
const PROXY_FORMAT_SELECTOR: &str = "bestvideo[height<=360][vcodec^=avc1]+bestaudio[acodec^=mp4a]/\
     bestvideo[height<=360]+bestaudio/best[height<=360]/worst";

/// Downloads a small copy of the video for local scrubbing, for sources with no
/// directly playable stream. Cached by URL hash under the app cache directory.
///
/// The last resort, reached only when neither a muxed nor a video-only H.264
/// stream could be resolved, or when a resolved stream failed to play in the
/// webview. See `pick_preview_format`.
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
    let hash = hasher.finish();
    let target = cache_dir.join(format!("{:x}.mp4", hash));
    let tmp = cache_dir.join(format!("{:x}.partial.mp4", hash));

    if target.exists() {
        return Ok(target.to_string_lossy().to_string());
    }

    // A previous run may have been killed mid-download, leaving a partial
    // file at `tmp`. yt-dlp would otherwise resume/append to it; remove it
    // so every attempt starts clean.
    let _ = std::fs::remove_file(&tmp);

    // Run on a blocking thread: this shells out to yt-dlp/ffmpeg to actually
    // download a short clip, which can take several seconds. Calling
    // Command::output() inline on this async fn would stall the runtime
    // thread and, with it, other concurrent Tauri commands (the job queue
    // runs downloads concurrently on the same runtime).
    let yt_dlp = paths.yt_dlp.clone();
    let ffmpeg = paths.ffmpeg.clone();
    // Same reason as in `resolve_preview`: with no runtime the format selector
    // below matches nothing and this download fails outright.
    let js_runtime = crate::binary_manager::resolve_js_runtime(&paths);
    let binaries_dir = paths.dir.clone();
    let tmp_for_task = tmp.clone();
    let url_for_task = url.clone();
    let output = tokio::task::spawn_blocking(move || {
        let mut args: Vec<String> = Vec::new();
        crate::binary_manager::push_js_runtime_args(&mut args, js_runtime.as_ref());
        let mut cmd = Command::new(&yt_dlp);
        // This one merges with ffmpeg (`--merge-output-format mp4`) and can
        // have it fetch over HTTPS, so it needs the CA bundle in its
        // environment just as much as the download runner does.
        crate::binary_manager::augment_path_env(&mut cmd, &binaries_dir);
        cmd.args(&args)
            .arg("--no-playlist")
            .arg("-f")
            .arg(PROXY_FORMAT_SELECTOR)
            .arg("--merge-output-format")
            .arg("mp4")
            .arg("--ffmpeg-location")
            .arg(&ffmpeg)
            .arg("-o")
            .arg(&tmp_for_task)
            .arg(&url_for_task);
        cmd.output()
    })
    .await
    .map_err(|e| format!("Preview download task failed: {e}"))?
    .map_err(|e| format!("Failed to fetch preview: {e}"))?;

    if !output.status.success() {
        // Never leave a partial file behind for a later call to mistake for
        // a complete proxy — that is what turned a transient failure into a
        // permanently poisoned cache entry.
        let _ = std::fs::remove_file(&tmp);
        return Err(format!(
            "Preview download failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    // Atomic within one directory on every platform this app targets: after
    // this, `target` either does not exist or is a complete file — never a
    // half-written one.
    std::fs::rename(&tmp, &target).map_err(|e| format!("Could not finalise preview: {e}"))?;

    Ok(target.to_string_lossy().to_string())
}

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
        assert!(pick_video_only_format(&json!([])).is_none());
        assert!(pick_preview_format(&json!([])).is_none());
    }

    // --- video-only selection ------------------------------------------------
    //
    // Measured against a real YouTube URL: 47 formats, 0 muxed, 12 avc1
    // video-only. Without this branch every such video paid for a full proxy
    // download before a single cut point could be placed.

    #[test]
    fn picks_the_best_avc1_video_only_format_at_or_below_480p() {
        let formats = json!([
            { "format_id": "160", "vcodec": "avc1.4d400c", "acodec": "none", "height": 144,  "url": "https://cdn/160" },
            { "format_id": "135", "vcodec": "avc1.4d401f", "acodec": "none", "height": 480,  "url": "https://cdn/135" },
            { "format_id": "137", "vcodec": "avc1.640028", "acodec": "none", "height": 1080, "url": "https://cdn/137" }
        ]);
        let picked = pick_video_only_format(&formats).expect("an avc1 video-only format exists");
        assert_eq!(picked["format_id"], "135");
    }

    // VP9 decodes unreliably in the macOS webview, so a stream that would
    // download fine is still no use as a preview.
    #[test]
    fn ignores_video_only_formats_that_are_not_avc1() {
        let formats = json!([
            { "format_id": "248", "vcodec": "vp9",       "acodec": "none", "height": 480, "url": "https://cdn/248" },
            { "format_id": "401", "vcodec": "av01.0.08M", "acodec": "none", "height": 480, "url": "https://cdn/401" }
        ]);
        assert!(pick_video_only_format(&formats).is_none());
    }

    #[test]
    fn ignores_video_only_entries_that_carry_no_url() {
        let formats = json!([
            { "format_id": "134", "vcodec": "avc1.4d401e", "acodec": "none", "height": 360 }
        ]);
        assert!(pick_video_only_format(&formats).is_none());
        assert!(pick_preview_format(&formats).is_none());
    }

    // Both exist at 480p on real YouTube (135 progressive, 231 HLS). A single
    // range-served file seeks more predictably in a <video> element than a
    // playlist does.
    #[test]
    fn prefers_a_progressive_stream_over_an_hls_manifest_at_the_same_height() {
        let formats = json!([
            { "format_id": "231", "vcodec": "avc1.4D401F", "acodec": "none", "height": 480, "protocol": "m3u8_native", "url": "https://manifest/231" },
            { "format_id": "135", "vcodec": "avc1.4d401f", "acodec": "none", "height": 480, "protocol": "https",       "url": "https://cdn/135" }
        ]);
        let picked = pick_video_only_format(&formats).expect("an avc1 video-only format exists");
        assert_eq!(picked["format_id"], "135");
    }

    // Nothing at or below 480p: a taller stream still beats a full proxy
    // download, so the shortest available is taken rather than nothing.
    #[test]
    fn falls_back_to_the_smallest_video_only_format_above_480p() {
        let formats = json!([
            { "format_id": "137", "vcodec": "avc1.640028", "acodec": "none", "height": 1080, "url": "https://cdn/137" },
            { "format_id": "136", "vcodec": "avc1.4d401f", "acodec": "none", "height": 720,  "url": "https://cdn/136" }
        ]);
        let picked = pick_video_only_format(&formats).expect("an avc1 video-only format exists");
        assert_eq!(picked["format_id"], "136");
    }

    // --- combined preference -------------------------------------------------

    #[test]
    fn prefers_a_muxed_format_and_reports_that_it_has_audio() {
        let formats = json!([
            { "format_id": "18",  "vcodec": "avc1", "acodec": "mp4a", "height": 360, "url": "https://cdn/18" },
            { "format_id": "135", "vcodec": "avc1.4d401f", "acodec": "none", "height": 480, "url": "https://cdn/135" }
        ]);
        let (picked, has_audio) = pick_preview_format(&formats).expect("a playable format exists");
        assert_eq!(picked["format_id"], "18");
        assert!(has_audio);
    }

    #[test]
    fn falls_back_to_video_only_and_reports_that_it_has_no_audio() {
        let formats = json!([
            { "format_id": "140", "vcodec": "none", "acodec": "mp4a" },
            { "format_id": "135", "vcodec": "avc1.4d401f", "acodec": "none", "height": 480, "url": "https://cdn/135" }
        ]);
        let (picked, has_audio) = pick_preview_format(&formats).expect("a playable format exists");
        assert_eq!(picked["format_id"], "135");
        assert!(!has_audio);
    }

    // The proxy signal, still reachable: audio-only plus a VP9 video stream
    // leaves nothing the webview can play.
    #[test]
    fn returns_none_when_no_muxed_or_avc1_video_only_format_exists() {
        let formats = json!([
            { "format_id": "140", "vcodec": "none", "acodec": "mp4a", "url": "https://cdn/140" },
            { "format_id": "248", "vcodec": "vp9",  "acodec": "none", "height": 480, "url": "https://cdn/248" }
        ]);
        assert!(pick_preview_format(&formats).is_none());
    }

    // --- the proxy selector --------------------------------------------------

    // The literal is line-continued for readability; a mangled continuation
    // would smuggle spaces into the selector and yt-dlp would reject it.
    #[test]
    fn selector_is_a_single_unbroken_token() {
        assert!(!PROXY_FORMAT_SELECTOR.contains(char::is_whitespace));
        assert_eq!(
            PROXY_FORMAT_SELECTOR,
            "bestvideo[height<=360][vcodec^=avc1]+bestaudio[acodec^=mp4a]/bestvideo[height<=360]+bestaudio/best[height<=360]/worst"
        );
    }

    // The old selector's failure mode, stated as a test so it cannot come back:
    // a bare `best[...]` implies a muxed stream, and the fallbacks that make an
    // avc1-less source still work must survive any future edit.
    #[test]
    fn selector_prefers_avc1_but_keeps_its_fallbacks() {
        let branches: Vec<&str> = PROXY_FORMAT_SELECTOR.split('/').collect();
        assert_eq!(
            branches[0],
            "bestvideo[height<=360][vcodec^=avc1]+bestaudio[acodec^=mp4a]"
        );
        assert!(branches.contains(&"bestvideo[height<=360]+bestaudio"));
        assert!(branches.contains(&"best[height<=360]"));
        assert!(branches.contains(&"worst"));
    }
}
