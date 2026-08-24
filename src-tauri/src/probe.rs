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
    //
    // Run on a blocking thread: yt-dlp's network fetch + extraction can take
    // several seconds, and this is an async fn — calling Command::output()
    // inline would block whichever runtime thread services this future,
    // stalling other concurrent Tauri commands (the job queue now runs
    // downloads concurrently on that same runtime).
    let yt_dlp = paths.yt_dlp;
    let output = tokio::task::spawn_blocking(move || {
        Command::new(&yt_dlp)
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

/// Downloads a small copy of the video for local scrubbing, for sources with no
/// directly playable stream. Cached by URL hash under the app cache directory.
///
/// The proxy path is the common one, not a rare edge case: current yt-dlp
/// extraction against YouTube frequently drops muxed formats entirely (see the
/// module-level notes on `pick_muxed_format`), so most previews land here.
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
    let yt_dlp = paths.yt_dlp;
    let ffmpeg = paths.ffmpeg;
    let tmp_for_task = tmp.clone();
    let url_for_task = url.clone();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(&yt_dlp)
            .arg("--no-playlist")
            .arg("-f")
            .arg("best[height<=360]/worst")
            .arg("--merge-output-format")
            .arg("mp4")
            .arg("--ffmpeg-location")
            .arg(&ffmpeg)
            .arg("-o")
            .arg(&tmp_for_task)
            .arg(&url_for_task)
            .output()
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
    }
}
