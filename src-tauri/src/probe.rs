use serde::{Deserialize, Serialize};
use std::process::Command;
use tauri::{AppHandle, Runtime, State};

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
    /// The sprite-sheet track the frontend paints as scrub-bar hover frames,
    /// or `None` when the extractor publishes none — which is the common case
    /// outside YouTube. See `pick_storyboard`.
    pub storyboard: Option<Storyboard>,
}

/// One sprite sheet of storyboard tiles, and the slice of the timeline it
/// covers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoryboardFragment {
    pub url: String,
    /// Seconds of video this sheet spans. Reported per fragment because it is
    /// *not* uniform: on a real 9-hour recording the 135 sheets of `sb1` run
    /// 249.90s each except the last, which covers the leftover 39.98s.
    pub duration: f64,
    /// Seconds from the start of the video to this sheet's first tile — the
    /// running sum of the durations before it. Precomputed here so the
    /// frontend maps a hovered timestamp to a sheet with a lookup instead of
    /// re-accumulating floats on every pointer move.
    pub start: f64,
}

/// A storyboard track: an ordered run of sprite sheets, each a `rows` x
/// `columns` grid of `tile_width` x `tile_height` frames.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Storyboard {
    pub format_id: String,
    pub rows: u32,
    pub columns: u32,
    pub tile_width: u32,
    pub tile_height: u32,
    /// Seconds of video one tile advances.
    ///
    /// Derived from the *first* fragment (`duration / (rows * columns)`), never
    /// from the fragment being displayed. The final sheet is short and only
    /// partly filled — dividing its own duration by the full grid size would
    /// place its four real frames as if they were twenty-five, and every hover
    /// near the end of a long video would show a blank tile.
    pub tile_duration: f64,
    pub fragments: Vec<StoryboardFragment>,
}

/// The `sb*` ids to take a storyboard from, best first.
///
/// `sb1` leads on the measured shape of a real YouTube track: 160x90 tiles,
/// which stay legible at scrub size, spread over 135 sheets for a nine-hour
/// recording. `sb0` is the same legibility at 320x180 but 373 sheets, so it is
/// the second choice rather than the first. `sb2` (80x45) and `sb3` (48x27) are
/// last: cheap to fetch and barely readable, worth having only when nothing
/// better is published.
const STORYBOARD_PREFERENCE: [&str; 4] = ["sb1", "sb0", "sb2", "sb3"];

/// The largest a preview track may be, in bytes.
///
/// The preview exists to find a moment, so the only quality that matters is how
/// fast it answers a seek — and both halves of that cost scale with the size of
/// the file, not with its resolution. Before the first frame the webview must
/// pull the whole mp4 index, which grows with the sample count; every seek then
/// pulls a chunk whose size grows with the bitrate. Total bytes captures both
/// well enough to decide on, and it is the one figure yt-dlp reports directly.
///
/// 500 MB is set where the gradient it produces is the one a person would
/// choose by hand: a three-minute clip previews at 480p (a few tens of MB), an
/// hour-long talk still previews at 480p (~324 MB at the 720 kbit/s YouTube
/// serves itag 135 at), a two-hour one steps down to 360p, and the nine-hour
/// livestream that prompted this — 2.9 GB at 480p, 793 MB at 240p — lands on
/// the 368 MB 144p track. Resolution is spent only where it is affordable.
const PREVIEW_BYTE_BUDGET: u64 = 500 * 1024 * 1024;

/// Whether a format is a plain range-served file rather than a manifest.
///
/// A `<video src>` is handed one URL and expects one file behind it. yt-dlp's
/// `m3u8_native`, `http_dash_segments`, `mhtml` and friends name a playlist or
/// an index that only a media-source player can assemble; pointing the element
/// at one is unreliable at best. Only `https` (and plain `http`, the same thing
/// without TLS) qualify. A format with no `protocol` field at all is taken to
/// be progressive — that is the ordinary shape of a bare media URL, and many
/// extractors outside YouTube omit the field entirely.
fn is_progressive(f: &serde_json::Value) -> bool {
    matches!(f["protocol"].as_str().unwrap_or("https"), "https" | "http")
}

/// The size of a format in bytes, as reported or as estimated from its bitrate.
///
/// `tbr` is in kbit/s, so a second of it is `tbr * 1000 / 8` bytes. `None` when
/// the format reports neither a size nor a bitrate-and-duration pair to derive
/// one from — a real gap for some extractors, and one the caller treats as
/// "unknown", never as "free".
fn estimated_bytes(f: &serde_json::Value, duration: Option<f64>) -> Option<u64> {
    if let Some(bytes) = f["filesize"].as_f64().or_else(|| f["filesize_approx"].as_f64()) {
        if bytes > 0.0 {
            return Some(bytes as u64);
        }
    }
    let tbr = f["tbr"].as_f64()?;
    let secs = duration?;
    if tbr <= 0.0 || secs <= 0.0 {
        return None;
    }
    Some((tbr * 125.0 * secs) as u64)
}

/// Picks the preview stream out of an already-filtered candidate pool.
///
/// Two rules, in this order:
///
/// 1. **Affordable first.** Anything over `PREVIEW_BYTE_BUDGET` is set aside;
///    a format whose size cannot be determined stays in, because refusing it
///    would push every extractor that reports neither `filesize` nor `tbr`
///    onto the full-download proxy path for no measured reason.
/// 2. **Then the tallest that is still not too tall.** Within what is
///    affordable, the highest height at or below 480p — preview detail past
///    that is spent on nothing.
///
/// If nothing at all fits the budget the smallest candidate is returned rather
/// than none: a slow preview still beats downloading a proxy copy of the whole
/// video, and the frontend's `onError` path is still behind it.
fn choose_streamable<'a>(
    candidates: &[&'a serde_json::Value],
    duration: Option<f64>,
) -> Option<&'a serde_json::Value> {
    if candidates.is_empty() {
        return None;
    }

    let mut affordable: Vec<&serde_json::Value> = candidates
        .iter()
        .copied()
        .filter(|f| estimated_bytes(f, duration).is_none_or(|b| b <= PREVIEW_BYTE_BUDGET))
        .collect();

    if !affordable.is_empty() {
        // Ascending by height, so the reverse scan finds the tallest.
        affordable.sort_by_key(|f| f["height"].as_u64().unwrap_or(0));
        return affordable
            .iter()
            .rev()
            .find(|f| f["height"].as_u64().unwrap_or(0) <= 480)
            .or_else(|| affordable.first())
            .copied();
    }

    // Every candidate is over budget: take the least bad one. An unknown size
    // cannot reach here — it was affordable by definition — so `u64::MAX` is
    // an unreachable default rather than a silent winner.
    candidates
        .iter()
        .copied()
        .min_by_key(|f| estimated_bytes(f, duration).unwrap_or(u64::MAX))
}

/// Selects a format the webview's <video> element can play on its own: both a
/// video and an audio codec in one stream, served progressively, with a
/// resolvable URL. See `choose_streamable` for how one is picked from those.
pub fn pick_muxed_format(
    formats: &serde_json::Value,
    duration: Option<f64>,
) -> Option<serde_json::Value> {
    let arr = formats.as_array()?;

    let candidates: Vec<&serde_json::Value> = arr
        .iter()
        .filter(|f| {
            let v = f["vcodec"].as_str().unwrap_or("none");
            let a = f["acodec"].as_str().unwrap_or("none");
            v != "none" && a != "none" && f["url"].as_str().is_some() && is_progressive(f)
        })
        .collect();

    choose_streamable(&candidates, duration).cloned()
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
/// Progressive only, and inside a byte budget before resolution is considered
/// at all — see `is_progressive` and `choose_streamable`. Choosing on height
/// alone is what handed a nine-hour livestream a 2.9 GB 480p track and left the
/// user staring at a stage that never painted a frame.
pub fn pick_video_only_format(
    formats: &serde_json::Value,
    duration: Option<f64>,
) -> Option<serde_json::Value> {
    let arr = formats.as_array()?;

    let candidates: Vec<&serde_json::Value> = arr
        .iter()
        .filter(|f| {
            let v = f["vcodec"].as_str().unwrap_or("none");
            let a = f["acodec"].as_str().unwrap_or("none");
            v.starts_with("avc1") && a == "none" && f["url"].as_str().is_some() && is_progressive(f)
        })
        .collect();

    choose_streamable(&candidates, duration).cloned()
}

/// Builds the storyboard track the scrub bar paints hover frames from.
///
/// yt-dlp publishes storyboards as `sb*` formats whose `fragments` are the
/// sprite-sheet URLs, with `rows`/`columns` giving the grid and `width`/`height`
/// the size of one tile. Everything the frontend needs to turn a timestamp into
/// a tile is resolved here, including the cumulative start of each sheet.
///
/// `None` whenever no usable track exists — no `sb*` format, no fragments, a
/// degenerate grid, or a first fragment with no duration to derive the tile
/// step from. The hover preview is an enhancement; the scrub bar must work
/// exactly as before without it.
pub fn pick_storyboard(formats: &serde_json::Value) -> Option<Storyboard> {
    let arr = formats.as_array()?;

    let usable = |f: &&serde_json::Value| -> bool {
        let id = f["format_id"].as_str().unwrap_or("");
        id.starts_with("sb")
            && f["rows"].as_u64().unwrap_or(0) > 0
            && f["columns"].as_u64().unwrap_or(0) > 0
            && f["fragments"].as_array().is_some_and(|v| !v.is_empty())
    };

    // Preference order first; anything else with the right shape after it, so a
    // site that names its storyboards differently still gets hover frames.
    let chosen = STORYBOARD_PREFERENCE
        .iter()
        .find_map(|want| {
            arr.iter()
                .find(|f| f["format_id"].as_str() == Some(*want) && usable(f))
        })
        .or_else(|| arr.iter().find(usable))?;

    let rows = chosen["rows"].as_u64()? as u32;
    let columns = chosen["columns"].as_u64()? as u32;
    let per_sheet = f64::from(rows) * f64::from(columns);

    let raw = chosen["fragments"].as_array()?;
    let mut fragments: Vec<StoryboardFragment> = Vec::with_capacity(raw.len());
    let mut start = 0.0_f64;
    for frag in raw {
        // A fragment with no URL is not skippable: dropping it would slide every
        // later sheet earlier in the timeline and mis-time the whole track.
        let url = frag["url"].as_str()?.to_string();
        let duration = frag["duration"].as_f64().unwrap_or(0.0);
        fragments.push(StoryboardFragment {
            url,
            duration,
            start,
        });
        start += duration;
    }

    let tile_duration = fragments.first().map(|f| f.duration).unwrap_or(0.0) / per_sheet;
    if !(tile_duration.is_finite() && tile_duration > 0.0) {
        return None;
    }

    Some(Storyboard {
        format_id: chosen["format_id"].as_str().unwrap_or("").to_string(),
        rows,
        columns,
        tile_width: chosen["width"].as_u64().unwrap_or(0) as u32,
        tile_height: chosen["height"].as_u64().unwrap_or(0) as u32,
        tile_duration,
        fragments,
    })
}

/// Selects an audio-only format to pair with a video-only preview pick, so the
/// frontend can play a separate `<audio>` element in sync with the silent
/// `<video>` instead of waiting on a full proxy download for sound.
///
/// Prefers `mp4a` (AAC) over any other codec: the preview plays in a WKWebView
/// on macOS, which decodes AAC reliably and Opus/WebM far less so. Falls back
/// to any audio-only format with a resolvable `url` if no AAC track exists.
///
/// Progressive only, for the same reason the video pick is: an `<audio>`
/// element is handed one URL and cannot assemble a playlist.
///
/// Among same-preference candidates, prefers the lowest present bitrate
/// (`abr`, falling back to `tbr`) — this is a preview, not the download, so a
/// modest bitrate is plenty. A format reporting neither field sorts last
/// rather than winning by default.
pub fn pick_audio_only_format(formats: &serde_json::Value) -> Option<serde_json::Value> {
    let arr = formats.as_array()?;

    let candidates: Vec<&serde_json::Value> = arr
        .iter()
        .filter(|f| {
            let v = f["vcodec"].as_str().unwrap_or("none");
            let a = f["acodec"].as_str().unwrap_or("none");
            a != "none" && v == "none" && f["url"].as_str().is_some() && is_progressive(f)
        })
        .collect();

    if candidates.is_empty() {
        return None;
    }

    let is_aac = |f: &&serde_json::Value| f["acodec"].as_str().unwrap_or("").starts_with("mp4a");
    let mut pool: Vec<&serde_json::Value> = candidates.iter().copied().filter(is_aac).collect();
    if pool.is_empty() {
        pool = candidates;
    }

    let bitrate = |f: &&serde_json::Value| -> f64 {
        f["abr"]
            .as_f64()
            .or_else(|| f["tbr"].as_f64())
            .unwrap_or(f64::MAX)
    };

    pool.sort_by(|a, b| {
        bitrate(a)
            .partial_cmp(&bitrate(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    pool.first().map(|f| (**f).clone())
}

/// The `audio_url` companion for a preview pick: `None` whenever the picked
/// stream already carries its own sound (`has_audio`), otherwise the URL of
/// the best match from `pick_audio_only_format`.
pub fn pick_preview_audio_url(formats: &serde_json::Value, has_audio: bool) -> Option<String> {
    if has_audio {
        return None;
    }
    pick_audio_only_format(formats).and_then(|f| f["url"].as_str().map(|s| s.to_string()))
}

/// The preview source, in order of preference: a muxed stream (picture and
/// sound), then a video-only H.264 stream (picture alone), then nothing — for
/// which the caller falls back to downloading a proxy.
///
/// The bool is whether the chosen format carries audio.
///
/// `duration` is the video's length in seconds, used to turn a bitrate into an
/// estimated size when a format reports no `filesize`. `None` simply means one
/// fewer signal — see `estimated_bytes`.
pub fn pick_preview_format(
    formats: &serde_json::Value,
    duration: Option<f64>,
) -> Option<(serde_json::Value, bool)> {
    if let Some(f) = pick_muxed_format(formats, duration) {
        return Some((f, true));
    }
    pick_video_only_format(formats, duration).map(|f| (f, false))
}

/// Shells out to `yt-dlp --dump-single-json` for `url` and returns the parsed
/// metadata document, with no interpretation of its fields.
///
/// The one place that runs this command and parses its output — `resolve_preview`
/// (which also needs `formats`) and `fetch_job_metadata` (which needs only
/// `title`/`thumbnail`) both build on this rather than each shelling out and
/// parsing JSON on their own.
///
/// `--dump-single-json` emits exactly one object even for playlists, unlike
/// `--dump-json` which emits one per entry and breaks JSON parsing.
///
/// Synchronous and blocking on the network + yt-dlp's extraction, which was
/// measured at 33–47 seconds per call on a real YouTube URL — every caller MUST
/// run this via `tokio::task::spawn_blocking` rather than inline on an async
/// fn, or it will stall whichever runtime thread services that future.
///
/// At that cost, neither caller reaches this directly: both go through
/// `metadata_cache::get_or_probe`, which runs it at most once per URL and makes
/// a second caller for a URL already being probed wait on the first rather than
/// start its own.
fn dump_single_json(
    yt_dlp: &std::path::Path,
    binaries_dir: &std::path::Path,
    js_runtime: Option<&crate::binary_manager::JsRuntime>,
    url: &str,
) -> Result<serde_json::Value, String> {
    let mut args: Vec<String> = Vec::new();
    crate::binary_manager::push_js_runtime_args(&mut args, js_runtime);
    let mut cmd = Command::new(yt_dlp);
    // Puts the bundled tools on PATH and, crucially, SSL_CERT_FILE in the
    // environment: yt-dlp may reach for ffmpeg here, and the bundled
    // ffmpeg cannot verify a single HTTPS certificate without it.
    crate::binary_manager::augment_path_env(&mut cmd, binaries_dir);
    let output = cmd
        .args(&args)
        .arg("--dump-single-json")
        .arg("--no-playlist")
        .arg("--no-download")
        .arg(url)
        .output()
        .map_err(|e| format!("Failed to probe video: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "Could not read video info: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    serde_json::from_str(&String::from_utf8_lossy(&output.stdout))
        .map_err(|e| format!("Failed to parse video info: {e}"))
}

/// The title/thumbnail pair the job queue needs to make a freshly queued job
/// recognisable before its download starts. Both are empty when nothing could
/// be learned — never a placeholder string — so the caller's precedence
/// logic (and the frontend's URL fallback) can tell "no metadata yet" from
/// "metadata said so".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JobMetadata {
    pub title: String,
    pub thumbnail: String,
}

/// Fetches `title`/`thumbnail` for a job, via the same `--dump-single-json`
/// call `resolve_preview` uses — and through the same `cache`, so a URL the
/// trim panel has already previewed costs nothing here, and one it is still
/// probing is awaited rather than probed a second time.
///
/// Deliberately not a `#[tauri::command]`: this is called from the Rust side
/// (the job queue, off the `enqueue_job` path) rather than invoked by the
/// frontend. Runs its blocking work on `spawn_blocking` internally, so it is
/// safe to `.await` from an async context without stalling the runtime.
///
/// Returns `Err` for a probe failure (age-gated video, offline, unsupported
/// site, ...) rather than a partially-filled `JobMetadata` — the caller
/// decides what "no metadata" means for the job; see `jobs::JobRegistry::set_metadata`.
pub async fn fetch_job_metadata<R: Runtime>(
    app_handle: &AppHandle<R>,
    cache: &crate::metadata_cache::SharedMetadataCache,
    url: &str,
) -> Result<JobMetadata, String> {
    let paths = crate::binary_manager::resolve_paths(app_handle)?;
    crate::binary_manager::ensure_executable(&paths)?;

    let yt_dlp = paths.yt_dlp.clone();
    let js_runtime = crate::binary_manager::resolve_js_runtime(&paths);
    let binaries_dir = paths.dir.clone();
    let url_for_task = url.to_string();
    // `METADATA_MAX_AGE`, not `PLAYBACK_MAX_AGE`: only `title` and `thumbnail`
    // are read below, and neither expires the way a format URL does. Enqueueing
    // a URL the trim panel probed a while ago therefore still costs nothing.
    let meta = crate::metadata_cache::get_or_probe(
        cache,
        url,
        crate::metadata_cache::METADATA_MAX_AGE,
        move || async move {
            tokio::task::spawn_blocking(move || {
                dump_single_json(&yt_dlp, &binaries_dir, js_runtime.as_ref(), &url_for_task)
            })
            .await
            .map_err(|e| format!("Metadata task failed: {e}"))?
        },
    )
    .await?;

    Ok(JobMetadata {
        title: meta["title"].as_str().unwrap_or("").to_string(),
        thumbnail: meta["thumbnail"].as_str().unwrap_or("").to_string(),
    })
}

#[tauri::command]
pub async fn resolve_preview<R: Runtime>(
    app_handle: AppHandle<R>,
    cache: State<'_, crate::metadata_cache::SharedMetadataCache>,
    url: String,
) -> Result<PreviewSource, String> {
    let paths = crate::binary_manager::resolve_paths(&app_handle)?;
    crate::binary_manager::ensure_executable(&paths)?;

    // Run on a blocking thread: yt-dlp's network fetch + extraction can take
    // several seconds, and this is an async fn — calling it inline would
    // block whichever runtime thread services this future, stalling other
    // concurrent Tauri commands (the job queue now runs downloads
    // concurrently on that same runtime).
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
    let url_for_task = url.clone();
    // Through the shared cache, so that re-opening the trim panel on a URL just
    // probed — and the metadata fetch `enqueue_job` spawns for that same URL —
    // cost nothing rather than another full extraction. `PLAYBACK_MAX_AGE`
    // because the format URLs picked below are what gets played, and those are
    // signed and time-limited. A probe already in flight for this URL is awaited
    // rather than duplicated; see `metadata_cache`.
    let meta = crate::metadata_cache::get_or_probe(
        cache.inner(),
        &url,
        crate::metadata_cache::PLAYBACK_MAX_AGE,
        move || async move {
            tokio::task::spawn_blocking(move || {
                dump_single_json(&yt_dlp, &binaries_dir, js_runtime.as_ref(), &url_for_task)
            })
            .await
            .map_err(|e| format!("Probe task failed: {e}"))?
        },
    )
    .await?;

    let title = meta["title"].as_str().unwrap_or("Unknown Title").to_string();
    let thumbnail = meta["thumbnail"].as_str().unwrap_or("").to_string();
    // Absent duration stays None. Substituting 0.0 here is what previously
    // collapsed the frontend scrub control to two positions.
    let duration = meta["duration"].as_f64();

    // A URL that turns out not to play is not a dead end: the frontend's
    // <video onError> falls back to `fetch_preview_proxy`. That matters most for
    // the video-only branch, where playability is likely but not guaranteed for
    // every itag.
    // Reported in both branches. Hover frames are how a nine-hour recording is
    // navigated at all, and they do not depend on a stream having been found —
    // a source on the proxy path still gets them while the copy downloads.
    let storyboard = pick_storyboard(&meta["formats"]);

    match pick_preview_format(&meta["formats"], duration) {
        Some((f, has_audio)) => {
            let audio_url = pick_preview_audio_url(&meta["formats"], has_audio);
            Ok(PreviewSource {
                kind: "stream".to_string(),
                url: f["url"].as_str().map(|s| s.to_string()),
                has_audio,
                audio_url,
                duration,
                title,
                thumbnail,
                storyboard,
            })
        }
        None => Ok(PreviewSource {
            kind: "needs_proxy".to_string(),
            url: None,
            // No stream was chosen; the proxy the frontend fetches next is muxed.
            has_audio: true,
            audio_url: None,
            duration,
            title,
            thumbnail,
            storyboard,
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
        let picked = pick_muxed_format(&formats, None).expect("a muxed format exists");
        assert_eq!(picked["format_id"], "18");
    }

    #[test]
    fn prefers_the_highest_muxed_format_at_or_below_480p() {
        let formats = json!([
            { "format_id": "18",  "vcodec": "avc1", "acodec": "mp4a", "height": 360, "url": "https://cdn/18" },
            { "format_id": "22",  "vcodec": "avc1", "acodec": "mp4a", "height": 720, "url": "https://cdn/22" },
            { "format_id": "59",  "vcodec": "avc1", "acodec": "mp4a", "height": 480, "url": "https://cdn/59" }
        ]);
        let picked = pick_muxed_format(&formats, None).expect("a muxed format exists");
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
        assert!(pick_muxed_format(&formats, None).is_none());
    }

    #[test]
    fn ignores_muxed_entries_that_carry_no_url() {
        let formats = json!([
            { "format_id": "18", "vcodec": "avc1", "acodec": "mp4a", "height": 360 }
        ]);
        assert!(pick_muxed_format(&formats, None).is_none());
    }

    #[test]
    fn handles_an_empty_format_list() {
        assert!(pick_muxed_format(&json!([]), None).is_none());
        assert!(pick_video_only_format(&json!([]), None).is_none());
        assert!(pick_preview_format(&json!([]), None).is_none());
        assert!(pick_audio_only_format(&json!([])).is_none());
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
        let picked = pick_video_only_format(&formats, None).expect("an avc1 video-only format exists");
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
        assert!(pick_video_only_format(&formats, None).is_none());
    }

    #[test]
    fn ignores_video_only_entries_that_carry_no_url() {
        let formats = json!([
            { "format_id": "134", "vcodec": "avc1.4d401e", "acodec": "none", "height": 360 }
        ]);
        assert!(pick_video_only_format(&formats, None).is_none());
        assert!(pick_preview_format(&formats, None).is_none());
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
        let picked = pick_video_only_format(&formats, None).expect("an avc1 video-only format exists");
        assert_eq!(picked["format_id"], "135");
    }

    // --- protocol: manifests are not streams --------------------------------
    //
    // An `m3u8_native` URL is an HLS playlist, not a media file. Handing one to
    // a plain <video src> is unreliable, and the old height-only sort could
    // reach for one whenever it was the tallest thing on offer — which on real
    // YouTube it is, since the HLS ladder runs higher than the progressive one.

    #[test]
    fn never_chooses_an_hls_video_only_candidate_even_when_it_is_the_only_480p() {
        let formats = json!([
            { "format_id": "231", "vcodec": "avc1.4D401F", "acodec": "none", "height": 480, "protocol": "m3u8_native", "url": "https://manifest/231" },
            { "format_id": "134", "vcodec": "avc1.4d401e", "acodec": "none", "height": 360, "protocol": "https",       "url": "https://cdn/134" }
        ]);
        let picked = pick_video_only_format(&formats, None).expect("a progressive format exists");
        assert_eq!(picked["format_id"], "134");
    }

    #[test]
    fn returns_none_when_every_video_only_candidate_is_a_manifest() {
        let formats = json!([
            { "format_id": "231", "vcodec": "avc1.4D401F", "acodec": "none", "height": 480, "protocol": "m3u8_native",       "url": "https://manifest/231" },
            { "format_id": "269", "vcodec": "avc1.4D400C", "acodec": "none", "height": 144, "protocol": "m3u8_native",       "url": "https://manifest/269" },
            { "format_id": "dash","vcodec": "avc1.4d401e", "acodec": "none", "height": 360, "protocol": "http_dash_segments","url": "https://manifest/dash" }
        ]);
        assert!(pick_video_only_format(&formats, None).is_none());
        assert!(pick_preview_format(&formats, None).is_none());
    }

    #[test]
    fn never_chooses_an_hls_muxed_candidate() {
        let formats = json!([
            { "format_id": "hls-720", "vcodec": "avc1", "acodec": "mp4a", "height": 720, "protocol": "m3u8_native", "url": "https://manifest/hls" },
            { "format_id": "18",      "vcodec": "avc1", "acodec": "mp4a", "height": 360, "protocol": "https",       "url": "https://cdn/18" }
        ]);
        let picked = pick_muxed_format(&formats, None).expect("a progressive muxed format exists");
        assert_eq!(picked["format_id"], "18");
    }

    #[test]
    fn never_chooses_an_hls_audio_companion() {
        let formats = json!([
            { "format_id": "233", "vcodec": "none", "acodec": "mp4a.40.2", "abr": 32.0,  "protocol": "m3u8_native", "url": "https://manifest/233" },
            { "format_id": "140", "vcodec": "none", "acodec": "mp4a.40.2", "abr": 128.0, "protocol": "https",       "url": "https://cdn/140" }
        ]);
        let picked = pick_audio_only_format(&formats).expect("a progressive audio format exists");
        assert_eq!(picked["format_id"], "140");
    }

    // --- size: a preview is chosen to be seekable, not to be tall -----------
    //
    // The reported bug. Real figures from the nine-hour livestream at
    // youtube.com/watch?v=B29b3uKrsrk (duration 33526s): itag 135 is 480p and
    // 2.9 GB, itag 134 is 360p and 1.6 GB, itag 133 is 240p and 793 MB, itag
    // 160 is 144p and 368 MB. Only the last is under the budget.

    #[test]
    fn a_long_video_drops_to_the_one_track_inside_the_byte_budget() {
        let formats = json!([
            { "format_id": "160", "vcodec": "avc1.4d400c", "acodec": "none", "height": 144, "protocol": "https", "filesize": 385877994u64,  "url": "https://cdn/160" },
            { "format_id": "133", "vcodec": "avc1.4d4015", "acodec": "none", "height": 240, "protocol": "https", "filesize": 831850995u64,  "url": "https://cdn/133" },
            { "format_id": "134", "vcodec": "avc1.4d401e", "acodec": "none", "height": 360, "protocol": "https", "filesize": 1676570475u64, "url": "https://cdn/134" },
            { "format_id": "135", "vcodec": "avc1.4d401f", "acodec": "none", "height": 480, "protocol": "https", "filesize": 3019847034u64, "url": "https://cdn/135" }
        ]);
        let picked = pick_video_only_format(&formats, Some(33526.0)).expect("a format exists");
        assert_eq!(picked["format_id"], "160");
    }

    // The same ladder for a short clip: everything is affordable, so the
    // resolution preference decides and the preview is not needlessly coarse.
    #[test]
    fn a_short_video_still_previews_at_the_best_affordable_resolution() {
        let formats = json!([
            { "format_id": "160", "vcodec": "avc1.4d400c", "acodec": "none", "height": 144, "protocol": "https", "filesize": 195278u64,  "url": "https://cdn/160" },
            { "format_id": "133", "vcodec": "avc1.4d4015", "acodec": "none", "height": 240, "protocol": "https", "filesize": 433081u64,  "url": "https://cdn/133" },
            { "format_id": "135", "vcodec": "avc1.4d401f", "acodec": "none", "height": 480, "protocol": "https", "filesize": 2100000u64, "url": "https://cdn/135" }
        ]);
        let picked = pick_video_only_format(&formats, Some(19.0)).expect("a format exists");
        assert_eq!(picked["format_id"], "135");
    }

    // A taller track that is also lighter is not passed over: the budget is a
    // filter, not a penalty, and inside it height still wins.
    #[test]
    fn height_still_decides_between_two_affordable_tracks() {
        let formats = json!([
            { "format_id": "160", "vcodec": "avc1.4d400c", "acodec": "none", "height": 144, "protocol": "https", "filesize": 40_000_000u64, "url": "https://cdn/160" },
            { "format_id": "135", "vcodec": "avc1.4d401f", "acodec": "none", "height": 480, "protocol": "https", "filesize": 90_000_000u64, "url": "https://cdn/135" }
        ]);
        let picked = pick_video_only_format(&formats, Some(600.0)).expect("a format exists");
        assert_eq!(picked["format_id"], "135");
    }

    // No filesize at all: `tbr` (kbit/s) times the duration says the same thing.
    // 720 kbit/s across nine hours is 3.0 GB; 92 kbit/s is 386 MB.
    #[test]
    fn estimates_size_from_bitrate_when_no_filesize_is_reported() {
        let formats = json!([
            { "format_id": "160", "vcodec": "avc1.4d400c", "acodec": "none", "height": 144, "protocol": "https", "tbr": 92.077,  "url": "https://cdn/160" },
            { "format_id": "135", "vcodec": "avc1.4d401f", "acodec": "none", "height": 480, "protocol": "https", "tbr": 720.591, "url": "https://cdn/135" }
        ]);
        let picked = pick_video_only_format(&formats, Some(33526.0)).expect("a format exists");
        assert_eq!(picked["format_id"], "160");

        // Same bitrates, three minutes of them: both fit, so height decides.
        let picked_short = pick_video_only_format(&formats, Some(180.0)).expect("a format exists");
        assert_eq!(picked_short["format_id"], "135");
    }

    // A size that cannot be determined must not be read as free — but nor may
    // it be excluded, or every extractor that reports neither field would fall
    // through to a full proxy download. It stays eligible.
    #[test]
    fn a_format_of_unknown_size_remains_eligible() {
        let formats = json!([
            { "format_id": "unknown", "vcodec": "avc1.4d401e", "acodec": "none", "height": 360, "protocol": "https", "url": "https://cdn/unknown" }
        ]);
        let picked = pick_video_only_format(&formats, Some(33526.0)).expect("a format exists");
        assert_eq!(picked["format_id"], "unknown");
    }

    // Nothing is affordable — a very long video whose lightest track is still
    // over budget. A slow preview beats no preview and beats a proxy download
    // of the whole thing, so the smallest is taken.
    #[test]
    fn falls_back_to_the_lightest_track_when_nothing_fits_the_budget() {
        let formats = json!([
            { "format_id": "135", "vcodec": "avc1.4d401f", "acodec": "none", "height": 480, "protocol": "https", "filesize": 3019847034u64, "url": "https://cdn/135" },
            { "format_id": "134", "vcodec": "avc1.4d401e", "acodec": "none", "height": 360, "protocol": "https", "filesize": 1676570475u64, "url": "https://cdn/134" }
        ]);
        let picked = pick_video_only_format(&formats, Some(33526.0)).expect("a format exists");
        assert_eq!(picked["format_id"], "134");
    }

    // --- storyboards ---------------------------------------------------------
    //
    // Measured on the reported nine-hour livestream: sb0 = 320x180 3x3 over 373
    // sheets, sb1 = 160x90 5x5 over 135 sheets, sb2 = 80x45 10x10 over 34,
    // sb3 = 48x27 10x10 in a single sheet. Fragment durations are 249.8956s for
    // every sb1 sheet but the last, which covers 39.98s.

    fn storyboard_formats() -> serde_json::Value {
        json!([
            { "format_id": "sb3", "rows": 10, "columns": 10, "width": 48,  "height": 27,
              "fragments": [ { "url": "https://sb/L0/default.jpg", "duration": 33526.0 } ] },
            { "format_id": "sb1", "rows": 5, "columns": 5, "width": 160, "height": 90,
              "fragments": [
                { "url": "https://sb/L2/M0.jpg", "duration": 249.89564698867025 },
                { "url": "https://sb/L2/M1.jpg", "duration": 249.89564698867025 },
                { "url": "https://sb/L2/M2.jpg", "duration": 39.98330351818731 }
              ] },
            { "format_id": "sb0", "rows": 3, "columns": 3, "width": 320, "height": 180,
              "fragments": [ { "url": "https://sb/L3/M0.jpg", "duration": 89.96243291592128 } ] },
            { "format_id": "135", "vcodec": "avc1.4d401f", "acodec": "none", "height": 480, "protocol": "https", "url": "https://cdn/135" }
        ])
    }

    #[test]
    fn prefers_sb1_and_reports_its_grid_and_tile_size() {
        let sb = pick_storyboard(&storyboard_formats()).expect("a storyboard exists");
        assert_eq!(sb.format_id, "sb1");
        assert_eq!((sb.rows, sb.columns), (5, 5));
        assert_eq!((sb.tile_width, sb.tile_height), (160, 90));
        // 249.8956 / 25 tiles: one frame roughly every ten seconds.
        assert!((sb.tile_duration - 9.99582587954681).abs() < 1e-9);
    }

    // The cumulative starts are what turn a hovered timestamp into a sheet, and
    // they must come from the per-fragment durations: the last sheet is short,
    // so assuming a uniform 249.9s stride would misplace the whole tail.
    #[test]
    fn reports_each_fragment_with_its_own_duration_and_cumulative_start() {
        let sb = pick_storyboard(&storyboard_formats()).expect("a storyboard exists");
        assert_eq!(sb.fragments.len(), 3);
        assert_eq!(sb.fragments[0].start, 0.0);
        assert!((sb.fragments[1].start - 249.89564698867025).abs() < 1e-9);
        assert!((sb.fragments[2].start - 499.7912939773405).abs() < 1e-9);
        assert!((sb.fragments[2].duration - 39.98330351818731).abs() < 1e-9);
        assert_eq!(sb.fragments[2].url, "https://sb/L2/M2.jpg");
    }

    #[test]
    fn falls_back_to_sb0_when_sb1_is_absent() {
        let formats = json!([
            { "format_id": "sb2", "rows": 10, "columns": 10, "width": 80, "height": 45,
              "fragments": [ { "url": "https://sb/L1/M0.jpg", "duration": 999.58 } ] },
            { "format_id": "sb0", "rows": 3, "columns": 3, "width": 320, "height": 180,
              "fragments": [ { "url": "https://sb/L3/M0.jpg", "duration": 89.96 } ] }
        ]);
        let sb = pick_storyboard(&formats).expect("a storyboard exists");
        assert_eq!(sb.format_id, "sb0");
    }

    // Most extractors publish no storyboard at all. That is not a failure — the
    // frontend simply shows no hover frames.
    #[test]
    fn reports_no_storyboard_when_the_source_publishes_none() {
        let formats = json!([
            { "format_id": "135", "vcodec": "avc1.4d401f", "acodec": "none", "height": 480, "protocol": "https", "url": "https://cdn/135" }
        ]);
        assert!(pick_storyboard(&formats).is_none());
        assert!(pick_storyboard(&json!([])).is_none());
    }

    #[test]
    fn rejects_a_storyboard_with_no_usable_geometry_or_fragments() {
        // No fragments to fetch.
        assert!(pick_storyboard(&json!([
            { "format_id": "sb1", "rows": 5, "columns": 5, "width": 160, "height": 90, "fragments": [] }
        ]))
        .is_none());
        // A degenerate grid would divide the tile step by zero.
        assert!(pick_storyboard(&json!([
            { "format_id": "sb1", "rows": 0, "columns": 5, "width": 160, "height": 90,
              "fragments": [ { "url": "https://sb/x.jpg", "duration": 10.0 } ] }
        ]))
        .is_none());
        // A first fragment with no duration leaves no tile step to derive.
        assert!(pick_storyboard(&json!([
            { "format_id": "sb1", "rows": 5, "columns": 5, "width": 160, "height": 90,
              "fragments": [ { "url": "https://sb/x.jpg" } ] }
        ]))
        .is_none());
    }

    // A storyboard under an unexpected id is still a storyboard.
    #[test]
    fn accepts_any_sb_id_when_none_of_the_preferred_ones_exist() {
        let formats = json!([
            { "format_id": "sb9", "rows": 4, "columns": 4, "width": 128, "height": 72,
              "fragments": [ { "url": "https://sb/L9/M0.jpg", "duration": 160.0 } ] }
        ]);
        let sb = pick_storyboard(&formats).expect("a storyboard exists");
        assert_eq!(sb.format_id, "sb9");
        assert_eq!(sb.tile_duration, 10.0);
    }

    // Nothing at or below 480p: a taller stream still beats a full proxy
    // download, so the shortest available is taken rather than nothing.
    #[test]
    fn falls_back_to_the_smallest_video_only_format_above_480p() {
        let formats = json!([
            { "format_id": "137", "vcodec": "avc1.640028", "acodec": "none", "height": 1080, "url": "https://cdn/137" },
            { "format_id": "136", "vcodec": "avc1.4d401f", "acodec": "none", "height": 720,  "url": "https://cdn/136" }
        ]);
        let picked = pick_video_only_format(&formats, None).expect("an avc1 video-only format exists");
        assert_eq!(picked["format_id"], "136");
    }

    // --- combined preference -------------------------------------------------

    #[test]
    fn prefers_a_muxed_format_and_reports_that_it_has_audio() {
        let formats = json!([
            { "format_id": "18",  "vcodec": "avc1", "acodec": "mp4a", "height": 360, "url": "https://cdn/18" },
            { "format_id": "135", "vcodec": "avc1.4d401f", "acodec": "none", "height": 480, "url": "https://cdn/135" }
        ]);
        let (picked, has_audio) = pick_preview_format(&formats, None).expect("a playable format exists");
        assert_eq!(picked["format_id"], "18");
        assert!(has_audio);
    }

    #[test]
    fn falls_back_to_video_only_and_reports_that_it_has_no_audio() {
        let formats = json!([
            { "format_id": "140", "vcodec": "none", "acodec": "mp4a" },
            { "format_id": "135", "vcodec": "avc1.4d401f", "acodec": "none", "height": 480, "url": "https://cdn/135" }
        ]);
        let (picked, has_audio) = pick_preview_format(&formats, None).expect("a playable format exists");
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
        assert!(pick_preview_format(&formats, None).is_none());
    }

    // --- audio-only selection for the video-only preview ---------------------
    //
    // Measured against a real YouTube URL: of 47 formats, exactly 2 are mp4a
    // audio-only (itags 139 and 140); itag 139 is the lower-bitrate one.

    #[test]
    fn picks_mp4a_audio_over_other_codecs() {
        let formats = json!([
            { "format_id": "251", "vcodec": "none", "acodec": "opus",      "abr": 128.0, "url": "https://cdn/251" },
            { "format_id": "140", "vcodec": "none", "acodec": "mp4a.40.2", "abr": 128.0, "url": "https://cdn/140" }
        ]);
        let picked = pick_audio_only_format(&formats).expect("an audio-only format exists");
        assert_eq!(picked["format_id"], "140");
    }

    #[test]
    fn prefers_the_lower_bitrate_aac_track() {
        let formats = json!([
            { "format_id": "140", "vcodec": "none", "acodec": "mp4a.40.2", "abr": 128.0, "url": "https://cdn/140" },
            { "format_id": "139", "vcodec": "none", "acodec": "mp4a.40.2", "abr": 48.0,  "url": "https://cdn/139" }
        ]);
        let picked = pick_audio_only_format(&formats).expect("an audio-only format exists");
        assert_eq!(picked["format_id"], "139");
    }

    #[test]
    fn falls_back_to_any_audio_only_format_when_no_aac_exists() {
        let formats = json!([
            { "format_id": "251", "vcodec": "none", "acodec": "opus", "abr": 160.0, "url": "https://cdn/251" }
        ]);
        let picked = pick_audio_only_format(&formats).expect("an audio-only format exists");
        assert_eq!(picked["format_id"], "251");
    }

    #[test]
    fn ignores_audio_only_formats_with_no_url() {
        let formats = json!([
            { "format_id": "140", "vcodec": "none", "acodec": "mp4a.40.2", "abr": 128.0 }
        ]);
        assert!(pick_audio_only_format(&formats).is_none());
    }

    #[test]
    fn returns_none_when_no_audio_only_format_exists() {
        let formats = json!([
            { "format_id": "135", "vcodec": "avc1.4d401f", "acodec": "none", "height": 480, "url": "https://cdn/135" }
        ]);
        assert!(pick_audio_only_format(&formats).is_none());
    }

    #[test]
    fn preview_audio_url_is_none_for_a_muxed_pick() {
        let formats = json!([
            { "format_id": "140", "vcodec": "none", "acodec": "mp4a.40.2", "abr": 128.0, "url": "https://cdn/140" }
        ]);
        assert_eq!(pick_preview_audio_url(&formats, true), None);
    }

    #[test]
    fn preview_audio_url_is_populated_for_a_video_only_pick() {
        let formats = json!([
            { "format_id": "140", "vcodec": "none", "acodec": "mp4a.40.2", "abr": 128.0, "url": "https://cdn/140" }
        ]);
        assert_eq!(
            pick_preview_audio_url(&formats, false),
            Some("https://cdn/140".to_string())
        );
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
