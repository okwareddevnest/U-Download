use serde::{Deserialize, Serialize};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(target_os = "android")]
use tauri::Emitter;
#[cfg(not(target_os = "android"))]
use tauri::menu::{Menu, MenuItem};
#[cfg(not(target_os = "android"))]
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Runtime, State, Window};
#[cfg(not(target_os = "android"))]
use tauri_plugin_dialog::DialogExt;

mod binary_manager;
mod jobs;
mod probe;
mod queue;
mod runner;
mod ytdlp;


#[cfg(target_os = "android")]
#[derive(Debug, Serialize, Deserialize, Clone)]
struct DownloadProgress {
    percentage: f64,
    speed: String,
    speed_bytes_per_sec: u64,
    eta: String,
    status: String,
    bytes_downloaded: u64,
    total_bytes: u64,
    download_start_time: std::time::SystemTime,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct VideoMetadata {
    title: String,
    duration: f64, // Duration in seconds
    thumbnail_url: String,
    uploader: String,
    view_count: Option<u64>,
    upload_date: Option<String>,
}

#[cfg(target_os = "android")]
type ProgressState = Arc<Mutex<DownloadProgress>>;

#[cfg(target_os = "android")]
fn send_download_complete_notification(_filename: &str) -> Result<(), String> { Ok(()) }
#[cfg(target_os = "android")]
fn send_download_error_notification(_error: &str) -> Result<(), String> { Ok(()) }

#[tauri::command]
async fn get_shared_url() -> Result<String, String> {
    #[cfg(target_os = "android")]
    {
        use std::fs;
        use std::path::PathBuf;
        let base = std::env::var("UDL_FILES_DIR").unwrap_or_default();
        if base.is_empty() { return Err("not-android".into()); }
        let path = PathBuf::from(base).join("shared_url.txt");
        match fs::read_to_string(&path) {
            Ok(s) => {
                let _ = fs::remove_file(&path);
                let trimmed = s.trim().to_string();
                if trimmed.is_empty() { Err("empty".into()) } else { Ok(trimmed) }
            }
            Err(e) => Err(format!("no-shared-url: {}", e)),
        }
    }
    #[cfg(not(target_os = "android"))]
    { Err("unsupported".into()) }
}

#[tauri::command]
async fn get_android_videos_dir() -> Result<String, String> {
    #[cfg(target_os = "android")]
    {
        use std::fs;
        use std::path::PathBuf;
        let base = std::env::var("UDL_FILES_DIR").unwrap_or_default();
        if base.is_empty() { return Err("not-android".into()); }
        let path = PathBuf::from(base).join("udownload_movies_dir.txt");
        match fs::read_to_string(&path) {
            Ok(s) => Ok(s.trim().to_string()),
            Err(e) => Err(format!("no-videos-dir: {}", e)),
        }
    }
    #[cfg(not(target_os = "android"))]
    { Err("unsupported".into()) }
}

#[tauri::command]
async fn get_video_metadata<R: Runtime>(app_handle: AppHandle<R>, url: String) -> Result<VideoMetadata, String> {
    let paths = binary_manager::resolve_paths(&app_handle)?;
    binary_manager::ensure_executable(&paths)?;

    // Get video information using bundled yt-dlp --dump-json
    let output = Command::new(&paths.yt_dlp)
        .arg("--dump-json")
        .arg("--no-download")
        .arg(&url)
        .output()
        .map_err(|e| format!("Failed to get video info: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to get video metadata: {}", stderr));
    }

    let json_output = String::from_utf8_lossy(&output.stdout);
    let metadata: serde_json::Value = serde_json::from_str(&json_output)
        .map_err(|e| format!("Failed to parse video metadata: {}", e))?;

    let title = metadata["title"]
        .as_str()
        .unwrap_or("Unknown Title")
        .to_string();

    let duration = metadata["duration"].as_f64().unwrap_or(0.0);

    let thumbnail_url = metadata["thumbnail"].as_str().unwrap_or("").to_string();

    let uploader = metadata["uploader"]
        .as_str()
        .unwrap_or("Unknown Uploader")
        .to_string();

    let view_count = metadata["view_count"].as_u64();

    let upload_date = metadata["upload_date"].as_str().map(|s| s.to_string());

    Ok(VideoMetadata {
        title,
        duration,
        thumbnail_url,
        uploader,
        view_count,
        upload_date,
    })
}

// Android-specific HTTP downloader removed; use unified yt-dlp/ffmpeg flow on all platforms.

#[tauri::command]
async fn check_ffmpeg<R: Runtime>(app_handle: AppHandle<R>) -> Result<String, String> {
    let paths = binary_manager::resolve_paths(&app_handle)?;
    binary_manager::ensure_executable(&paths)?;

    match Command::new(&paths.ffmpeg).arg("-version").output() {
        Ok(output) => {
            let version = String::from_utf8_lossy(&output.stdout);
            Ok(format!(
                "✅ FFmpeg: {}",
                version.lines().next().unwrap_or("unknown")
            ))
        }
        Err(e) => Err(format!("❌ FFmpeg: Bundled binary not found or not executable ({})", e)),
    }
}

#[tauri::command]
async fn select_output_folder<R: Runtime>(app_handle: AppHandle<R>) -> Result<String, String> {
    #[cfg(target_os = "android")]
    {
        use std::path::PathBuf;
        let base = std::env::var("UDL_FILES_DIR").unwrap_or_default();
        if base.is_empty() {
            return Err("unsupported".into());
        }
        let p = PathBuf::from(base).join("udownload_movies_dir.txt");
        match std::fs::read_to_string(&p) {
            Ok(s) => Ok(s.trim().to_string()),
            Err(_) => Err("No folder selected".into()),
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        use tauri_plugin_dialog::DialogExt;
        // Use blocking approach for folder selection
        let (tx, rx) = std::sync::mpsc::channel();
        app_handle.dialog().file().pick_folder(move |folder_path| {
            let _ = tx.send(folder_path);
        });
        // Wait for the dialog result with timeout
        match rx.recv_timeout(std::time::Duration::from_secs(30)) {
            Ok(Some(path)) => Ok(path.to_string()),
            Ok(None) => Err("No folder selected".to_string()),
            Err(_) => Err("Dialog timeout".to_string()),
        }
    }
}

/// Legacy single-download entry point, retained for Android only.
///
/// Desktop downloads go through `enqueue_job` and `runner`. Android has its
/// own downloader (`perform_download_android`) with no queue and no job
/// registry; porting it is out of scope here, so it keeps this command and the
/// global `download-progress` event.
#[cfg(target_os = "android")]
#[tauri::command]
async fn start_download<R: Runtime>(
    window: Window<R>,
    progress_state: State<'_, ProgressState>,
    url: String,
    downloadType: String,
    quality: String,
    outputFolder: String,
    startTime: Option<f64>,
    endTime: Option<f64>,
) -> Result<(), String> {
    let window_clone = window.clone();
    let progress_arc = progress_state.inner().clone();
    let url_clone = url.clone();
    let download_type_clone = downloadType.clone();
    let quality_clone = quality.clone();
    let output_folder_clone = outputFolder.clone();
    let start_time_clone = startTime;
    let end_time_clone = endTime;

    tokio::spawn(async move {
        let result = perform_download_android(
            &window_clone,
            progress_arc.clone(),
            &url_clone,
            &download_type_clone,
            &quality_clone,
            &output_folder_clone,
            start_time_clone,
            end_time_clone,
        )
        .await;

        match result {
            Ok(filename) => {
                let mut progress = progress_arc.lock().unwrap();
                progress.status = "completed".to_string();
                progress.percentage = 100.0;
                let progress_copy = progress.clone();
                let _ = window_clone.emit("download-progress", progress_copy);

                // Send completion notification
                let _ = send_download_complete_notification(&filename);
                let _ = window_clone.emit("download-complete", filename);
            }
            Err(e) => {
                let mut progress = progress_arc.lock().unwrap();
                progress.status = "error".to_string();
                eprintln!("Download error: {}", e);

                // Send error notification
                let _ = send_download_error_notification(&e);
                let _ = window_clone.emit("download-error", format!("Download failed: {}", e));
            }
        }
    });

    Ok(())
}

#[tauri::command]
async fn test_dependencies<R: Runtime>(app_handle: AppHandle<R>) -> Result<String, String> {
    let paths = binary_manager::resolve_paths(&app_handle)?;
    binary_manager::ensure_executable(&paths)?;
    let mut results = Vec::new();

    // Test yt-dlp (bundled)
    match Command::new(&paths.yt_dlp).arg("--version").output() {
        Ok(output) => {
            let version = String::from_utf8_lossy(&output.stdout);
            results.push(format!("✅ yt-dlp: {}", version.trim()));
        }
        Err(e) => {
            results.push(format!("❌ yt-dlp: Bundled binary error ({})", e));
        }
    }

    // Test aria2c (bundled)
    match Command::new(&paths.aria2c).arg("--version").output() {
        Ok(output) => {
            let version = String::from_utf8_lossy(&output.stdout);
            results.push(format!(
                "✅ aria2c: {}",
                version.lines().next().unwrap_or("unknown")
            ));
        }
        Err(e) => {
            results.push(format!("❌ aria2c: Bundled binary error ({})", e));
        }
    }

    Ok(results.join("\n"))
}

/// The concurrency the UI last asked for.
///
/// Only `enqueue_job` carries a concurrency argument, but cancelling, pausing
/// and resuming all change how many slots are free and therefore all need to
/// re-run the scheduler. They read the last requested value from here rather
/// than guessing a default that could under-dispatch.
type ConcurrencyState = Arc<AtomicU32>;

const DEFAULT_CONCURRENCY: u32 = 2;

fn runner_context<R: Runtime>(
    window: &Window<R>,
    concurrency: u32,
) -> Result<runner::RunnerContext, String> {
    let paths = binary_manager::resolve_paths(window.app_handle())?;
    binary_manager::ensure_executable(&paths)?;
    Ok(runner::RunnerContext {
        yt_dlp: paths.yt_dlp,
        ffmpeg: paths.ffmpeg,
        binaries_dir: paths.dir,
        concurrency: concurrency.max(1),
    })
}

#[tauri::command]
async fn enqueue_job<R: Runtime>(
    window: Window<R>,
    registry: State<'_, jobs::SharedJobs>,
    concurrency_state: State<'_, ConcurrencyState>,
    url: String,
    format: ytdlp::FormatChoice,
    trim: Option<ytdlp::TrimRange>,
    output_folder: String,
    concurrency: Option<u32>,
) -> Result<jobs::JobId, String> {
    // Resolved before the job is inserted, so a missing binary is reported to
    // the caller instead of leaving a job queued that can never start.
    let ctx = runner_context(&window, concurrency.unwrap_or(DEFAULT_CONCURRENCY))?;
    concurrency_state.store(ctx.concurrency, Ordering::Relaxed);

    // Lock scope: one insert.
    let id = {
        let mut reg = registry.lock().unwrap();
        reg.insert(jobs::Job::new(url, format, trim, output_folder))
    };

    // Announced before dispatch, because `pump` emits only for the jobs it can
    // actually claim. The frontend populates its map from `list_jobs` once at
    // mount and from events thereafter, so an enqueue beyond the concurrency
    // limit would otherwise not appear in the UI at all until a slot freed up.
    runner::emit_job(&window, registry.inner(), &id);

    runner::pump(window.clone(), registry.inner().clone(), ctx);
    Ok(id)
}

#[tauri::command]
fn list_jobs(registry: State<'_, jobs::SharedJobs>) -> Vec<jobs::Job> {
    // Lock scope: one snapshot of the registry.
    registry.lock().unwrap().list()
}

#[tauri::command]
async fn cancel_job<R: Runtime>(
    window: Window<R>,
    registry: State<'_, jobs::SharedJobs>,
    concurrency_state: State<'_, ConcurrencyState>,
    job_id: jobs::JobId,
) -> Result<(), String> {
    {
        // Lock scope: map writes only — `cancel` hands the kill and the reap
        // to a detached thread precisely so this does not block.
        registry.lock().unwrap().cancel(&job_id);
    }
    runner::emit_job(&window, registry.inner(), &job_id);
    repump(&window, &registry, &concurrency_state);
    Ok(())
}

#[tauri::command]
async fn pause_job<R: Runtime>(
    window: Window<R>,
    registry: State<'_, jobs::SharedJobs>,
    concurrency_state: State<'_, ConcurrencyState>,
    job_id: jobs::JobId,
) -> Result<(), String> {
    {
        // Lock scope: map writes only. Pausing an in-flight job goes through
        // the same non-blocking `cancel` as above.
        let mut reg = registry.lock().unwrap();
        queue::pause(&mut reg, &job_id);
    }
    runner::emit_job(&window, registry.inner(), &job_id);
    repump(&window, &registry, &concurrency_state);
    Ok(())
}

#[tauri::command]
async fn resume_job<R: Runtime>(
    window: Window<R>,
    registry: State<'_, jobs::SharedJobs>,
    concurrency_state: State<'_, ConcurrencyState>,
    job_id: jobs::JobId,
) -> Result<(), String> {
    {
        // Lock scope: one status write.
        let mut reg = registry.lock().unwrap();
        queue::resume(&mut reg, &job_id);
    }
    runner::emit_job(&window, registry.inner(), &job_id);
    // Without this the resumed job would sit in `Queued` until some unrelated
    // job finished or another was enqueued.
    repump(&window, &registry, &concurrency_state);
    Ok(())
}

/// Re-runs the scheduler after a command changed how many slots are free.
///
/// A binary that cannot be resolved must not turn a successful cancel, pause
/// or resume into an error — the state change already happened — so a failed
/// context simply means nothing is dispatched.
fn repump<R: Runtime>(
    window: &Window<R>,
    registry: &State<'_, jobs::SharedJobs>,
    concurrency_state: &State<'_, ConcurrencyState>,
) {
    if let Ok(ctx) = runner_context(window, concurrency_state.load(Ordering::Relaxed)) {
        runner::pump(window.clone(), registry.inner().clone(), ctx);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let job_registry: jobs::SharedJobs = Arc::new(Mutex::new(jobs::JobRegistry::new()));
    let concurrency_state: ConcurrencyState = Arc::new(AtomicU32::new(DEFAULT_CONCURRENCY));

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(job_registry)
        .manage(concurrency_state);

    // Android is still served by the legacy single-download command and its
    // global progress state; the desktop path is the job queue alone. The two
    // command lists are spelled out separately because `generate_handler!`
    // cannot take a `#[cfg]` on an individual entry.
    #[cfg(target_os = "android")]
    let builder = {
        let progress_state: ProgressState = Arc::new(Mutex::new(DownloadProgress {
            percentage: 0.0,
            speed: String::new(),
            speed_bytes_per_sec: 0,
            eta: String::new(),
            status: "idle".to_string(),
            bytes_downloaded: 0,
            total_bytes: 0,
            download_start_time: std::time::SystemTime::now(),
        }));
        builder
            .manage(progress_state)
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
                resume_job,
                start_download,
                probe::resolve_preview
            ])
    };

    #[cfg(not(target_os = "android"))]
    let builder = builder.invoke_handler(tauri::generate_handler![
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
        resume_job,
        probe::resolve_preview
    ]);

    builder
        .setup(move |_app| {
            #[cfg(not(target_os = "android"))]
            let app = _app;
            #[cfg(not(target_os = "android"))]
            {
                let show_item = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
                let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

                let _tray = TrayIconBuilder::new()
                    .icon(app.default_window_icon().unwrap().clone())
                    .menu(&menu)
                    .tooltip("U-Download")
                    .on_menu_event(|app, event| {
                        if event.id.as_ref() == "show" {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        } else if event.id.as_ref() == "quit" {
                            let app_handle = app.clone();
                            app.dialog()
                                .message("Are you sure you want to quit U-Download?")
                                .title("Quit Confirmation")
                                .kind(tauri_plugin_dialog::MessageDialogKind::Info)
                                .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancelCustom(
                                    "Yes".to_owned(),
                                    "No".to_owned(),
                                ))
                                .show(move |answer| {
                                    if answer {
                                        std::thread::spawn(move || {
                                            app_handle.exit(0);
                                        });
                                    }
                                });
                        }
                    })
                    .build(app)?;
            }
            Ok(())
        })
        .on_window_event(|_window, event| match event {
            tauri::WindowEvent::CloseRequested { .. } => {
                #[cfg(not(target_os = "android"))]
                {
                    let _ = _window.hide();
                }
                #[cfg(target_os = "android")]
                {
                    // Let Android handle back/close normally
                }
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
#[cfg(target_os = "android")]
async fn perform_download_android<R: Runtime>(
    window: &Window<R>,
    progress_state: ProgressState,
    url: &str,
    download_type: &str,
    quality: &str,
    output_folder: &str,
    _start_time: Option<f64>,
    _end_time: Option<f64>,
) -> Result<String, String> {
    use std::path::Path;
    use tokio::fs;

    eprintln!("Android YouTube download starting for URL: {}", url);

    // Set initial progress
    {
        let mut p = progress_state.lock().unwrap();
        p.status = "initializing".into();
        p.percentage = 0.0;
        p.bytes_downloaded = 0;
        p.total_bytes = 0;
        p.download_start_time = std::time::SystemTime::now();
        let _ = window.emit("download-progress", p.clone());
    }

    // Method 1: Advanced YouTube API extraction using multiple endpoints
    async fn try_youtube_api_extraction(
        url: &str,
        download_type: &str,
        quality: &str,
    ) -> Result<(String, String, Vec<u8>), String> {
        eprintln!("Attempting YouTube API extraction...");
        
        use regex::Regex;
        use rand::Rng;
        use rand::rngs::StdRng;
        use rand::SeedFromEntropy;
        
        // Extract video ID
        let video_id_regex = Regex::new(r"(?:youtube\.com/watch\?v=|youtu\.be/|youtube\.com/embed/|youtube\.com/v/)([a-zA-Z0-9_-]+)")
            .map_err(|e| format!("Video ID regex failed: {}", e))?;
        
        let video_id = video_id_regex
            .captures(url)
            .and_then(|caps| caps.get(1))
            .ok_or_else(|| "Could not extract video ID from URL".to_string())?
            .as_str();
        
        eprintln!("Extracted video ID: {}", video_id);
        
        // Advanced user agent rotation with real Android devices
        let user_agents = vec![
            "Mozilla/5.0 (Linux; Android 13; SM-S918B) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Mobile Safari/537.36",
            "Mozilla/5.0 (Linux; Android 12; SM-G998B) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36",
            "Mozilla/5.0 (Linux; Android 11; Pixel 6) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/119.0.0.0 Mobile Safari/537.36",
            "Mozilla/5.0 (Linux; Android 14; SM-A546B) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Mobile Safari/537.36",
            "Mozilla/5.0 (Linux; Android 12; OnePlus 9 Pro) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/118.0.0.0 Mobile Safari/537.36"
        ];
        
        let mut rng = StdRng::from_entropy();
        let user_agent = user_agents[rng.gen_range(0..user_agents.len())];
        
        // Create HTTP client with anti-bot headers
        let client = reqwest::Client::builder()
            .user_agent(user_agent)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?;
        
        // Method 1a: Try YouTube embed endpoint (often less protected)
        let embed_url = format!("https://www.youtube.com/embed/{}?autoplay=1", video_id);
        
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8".parse().unwrap());
        headers.insert("Accept-Language", "en-US,en;q=0.5".parse().unwrap());
        headers.insert("Accept-Encoding", "gzip, deflate, br".parse().unwrap());
        headers.insert("DNT", "1".parse().unwrap());
        headers.insert("Connection", "keep-alive".parse().unwrap());
        headers.insert("Sec-Fetch-Dest", "document".parse().unwrap());
        headers.insert("Sec-Fetch-Mode", "navigate".parse().unwrap());
        headers.insert("Sec-Fetch-Site", "none".parse().unwrap());
        
        // Add random delay to avoid detection
        let delay_ms = rng.gen_range(1000..3000);
        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
        
        let response = client
            .get(&embed_url)
            .headers(headers.clone())
            .send()
            .await
            .map_err(|e| format!("Failed to fetch embed page: {}", e))?;
        
        if !response.status().is_success() {
            return Err(format!("Embed request failed: {}", response.status()));
        }
        
        let html_content = response
            .text()
            .await
            .map_err(|e| format!("Failed to read embed content: {}", e))?;
        
        eprintln!("Fetched embed page, extracting streams...");
        
        // Modern extraction patterns - YouTube uses multiple variable names
        let extraction_patterns = vec![
            r#"ytInitialPlayerResponse"\s*=\s*(\{.*?\});"#,
            r#"var ytInitialPlayerResponse = (\{.*?\});"#,
            r#"window\[""ytInitialPlayerResponse""\]\s*=\s*(\{.*?\});"#,
            r#"ytcfg\.set\(\{""EXPERIMENT_FLAGS"".*?""PLAYER_CONFIG"":(\{.*?\})"#,
            r#"""player_response"":\s*""(.*?)"""#,
        ];
        
        let mut player_response: Option<serde_json::Value> = None;
        
        for pattern in &extraction_patterns {
            let regex = Regex::new(pattern)
                .map_err(|e| format!("Pattern regex failed: {}", e))?;
            
            if let Some(captures) = regex.captures(&html_content) {
                if let Some(json_match) = captures.get(1) {
                    let json_str = json_match.as_str();
                    
                    // Handle escaped JSON
                    let cleaned_json = json_str
                        .replace(r#"\"#, r#""#)
                        .replace(r#"\\"#, r#"\"#);
                    
                    match serde_json::from_str::<serde_json::Value>(&cleaned_json) {
                        Ok(parsed) => {
                            player_response = Some(parsed);
                            eprintln!("Successfully parsed player response with pattern: {}", pattern);
                            break;
                        }
                        Err(e) => {
                            eprintln!("JSON parse failed for pattern {}: {}", pattern, e);
                            continue;
                        }
                    }
                }
            }
        }
        
        let player_data = player_response
            .ok_or_else(|| "Could not extract player response from any pattern".to_string())?;
        
        // Extract video title
        let title = player_data
            .get("videoDetails")
            .and_then(|vd| vd.get("title"))
            .and_then(|t| t.as_str())
            .unwrap_or("Unknown Video")
            .to_string();
        
        eprintln!("Extracted title: {}", title);
        
        // Extract streaming data
        let streaming_data = player_data
            .get("streamingData")
            .ok_or_else(|| "No streamingData found in player response".to_string())?;
        
        // Select appropriate streams based on download type and quality
        let (stream_url, is_audio_only) = if download_type == "mp3" {
            // Extract audio streams
            let audio_formats = streaming_data
                .get("adaptiveFormats")
                .and_then(|f| f.as_array())
                .ok_or_else(|| "No adaptive formats found".to_string())?
                .iter()
                .filter(|stream| {
                    stream.get("mimeType")
                        .and_then(|mime| mime.as_str())
                        .map(|mime| mime.contains("audio"))
                        .unwrap_or(false)
                })
                .collect::<Vec<_>>();
            
            if audio_formats.is_empty() {
                return Err("No audio streams found".to_string());
            }
            
            // Select best quality audio stream
            let best_audio = audio_formats
                .iter()
                .max_by_key(|stream| {
                    stream.get("bitrate")
                        .and_then(|br| br.as_u64())
                        .unwrap_or(0)
                })
                .ok_or_else(|| "Could not select best audio stream".to_string())?;
            
            let url = best_audio
                .get("url")
                .and_then(|u| u.as_str())
                .ok_or_else(|| "No URL found in audio stream".to_string())?
                .to_string();
            
            (url, true)
        } else {
            // Extract video streams for specified quality
            let video_formats = streaming_data
                .get("formats")
                .and_then(|f| f.as_array())
                .or_else(|| {
                    streaming_data
                        .get("adaptiveFormats")
                        .and_then(|f| f.as_array())
                })
                .ok_or_else(|| "No video formats found".to_string())?
                .iter()
                .filter(|stream| {
                    stream.get("mimeType")
                        .and_then(|mime| mime.as_str())
                        .map(|mime| mime.contains("video"))
                        .unwrap_or(false)
                })
                .collect::<Vec<_>>();
            
            if video_formats.is_empty() {
                return Err("No video streams found".to_string());
            }
            
            // Filter by quality if specified
            let filtered_streams: Vec<_> = if quality != "best" {
                let target_height: u32 = quality.parse().unwrap_or(720);
                video_formats
                    .iter()
                    .filter(|stream| {
                        stream.get("height")
                            .and_then(|h| h.as_u64())
                            .map(|h| h as u32 <= target_height)
                            .unwrap_or(true)
                    })
                    .cloned()
                    .collect()
            } else {
                video_formats
            };
            
            let best_video = filtered_streams
                .iter()
                .max_by_key(|stream| {
                    let bitrate = stream.get("bitrate")
                        .and_then(|br| br.as_u64())
                        .unwrap_or(0);
                    let height = stream.get("height")
                        .and_then(|h| h.as_u64())
                        .unwrap_or(0);
                    bitrate + height * 1000 // Prioritize higher resolution with good bitrate
                })
                .ok_or_else(|| "Could not select best video stream".to_string())?;
            
            let url = best_video
                .get("url")
                .and_then(|u| u.as_str())
                .ok_or_else(|| "No URL found in video stream".to_string())?
                .to_string();
            
            (url, false)
        };
        
        eprintln!("Successfully extracted stream URL for {} (audio_only: {})", download_type, is_audio_only);
        
        // Download the content with progress tracking
        let download_response = client
            .get(&stream_url)
            .headers(headers)
            .send()
            .await
            .map_err(|e| format!("Failed to download stream: {}", e))?;
        
        if !download_response.status().is_success() {
            return Err(format!("Stream download failed: {}", download_response.status()));
        }
        
        let content_bytes = download_response
            .bytes()
            .await
            .map_err(|e| format!("Failed to read stream content: {}", e))?
            .to_vec();
        
        eprintln!("Successfully downloaded {} bytes", content_bytes.len());
        
        Ok((title, stream_url, content_bytes))
    }

    // Method 2: Fallback direct extraction with modern patterns
    async fn try_fallback_extraction(
        url: &str,
        download_type: &str,
    ) -> Result<(String, String), String> {
        eprintln!("Attempting fallback extraction...");
        
        use regex::Regex;
        use rand::Rng;
        use rand::rngs::StdRng;
        use rand::SeedFromEntropy;
        
        // Extract video ID with enhanced regex
        let video_id_regex = Regex::new(r"(?:youtube\.com/(?:[^/]+/.+/|(?:v|e(?:mbed)?|watch)/|.*[?&]v=)|youtu\.be/|youtube\.com/embed/)([^'&?/\s]{11})")
            .map_err(|e| format!("Video ID regex failed: {}", e))?;
        
        let video_id = video_id_regex
            .captures(url)
            .and_then(|caps| caps.get(1))
            .ok_or_else(|| "Could not extract video ID from URL".to_string())?
            .as_str();
        
        eprintln!("Extracted video ID: {}", video_id);
        
        // Try multiple endpoints with different approaches
        let mut rng = StdRng::from_entropy();
        let endpoints = vec![
            (format!("https://www.youtube.com/oembed?url=https://youtube.com/watch?v={}&format=json", video_id), "oembed"),
            (format!("https://m.youtube.com/watch?v={}", video_id), "mobile"),
            (format!("https://www.youtube.com/youtubei/v1/player?videoId={}&key=AIzaSyA8eiZmM1FaDVjRy-df2KTyQ_vz_yYM39w", video_id), "youtubei"),
        ];
        
        for (endpoint_url, endpoint_type) in &endpoints {
            eprintln!("Trying {} endpoint: {}", endpoint_type, endpoint_url);
            
            let user_agents = vec![
                "Mozilla/5.0 (Linux; Android 13; SM-S918B) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Mobile Safari/537.36",
                "Mozilla/5.0 (Linux; Android 12; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36",
                "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1",
            ];
            
            let user_agent = user_agents[rng.gen_range(0..user_agents.len())];
            
            let client = reqwest::Client::builder()
                .user_agent(user_agent)
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .map_err(|e| format!("Failed to create HTTP client: {}", e))?;
            
            // Add delay between requests
            let delay_ms = rng.gen_range(500..2000);
            tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
            
            match client.get(endpoint_url).send().await {
                Ok(response) if response.status().is_success() => {
                    match response.text().await {
                        Ok(content) => {
                            match *endpoint_type {
                                "oembed" => {
                                    if let Ok(oembed_data) = serde_json::from_str::<serde_json::Value>(&content) {
                                        if let Some(title) = oembed_data.get("title").and_then(|t| t.as_str()) {
                                            eprintln!("Found title via oembed: {}", title);
                                            // For oembed, we still need to get the actual stream URL
                                            // This is primarily used for title extraction
                                            continue;
                                        }
                                    }
                                }
                                "mobile" => {
                                    // Parse mobile page for stream URLs
                                    if let Ok(stream_info) = extract_from_mobile_page(&content, download_type) {
                                        return Ok(stream_info);
                                    }
                                }
                                "youtubei" => {
                                    // Parse YouTube internal API response
                                    if let Ok(api_data) = serde_json::from_str::<serde_json::Value>(&content) {
                                        if let Ok(stream_info) = extract_from_api_response(&api_data, download_type) {
                                            return Ok(stream_info);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to read {} response: {}", endpoint_type, e);
                            continue;
                        }
                    }
                }
                Ok(response) => {
                    eprintln!("{} endpoint returned status: {}", endpoint_type, response.status());
                    continue;
                }
                Err(e) => {
                    eprintln!("{} endpoint request failed: {}", endpoint_type, e);
                    continue;
                }
            }
        }
        
        Err("All fallback extraction methods failed".to_string())
    }
    
    fn extract_from_mobile_page(html: &str, download_type: &str) -> Result<(String, String), String> {
        use scraper::{Html, Selector};
        use regex::Regex;
        
        let document = Html::parse_document(html);
        
        // Extract title
        let title_selector = Selector::parse("title, meta[property='og:title'], meta[name='title']").unwrap();
        let title = document
            .select(&title_selector)
            .next()
            .and_then(|el| {
                if el.value().name() == "title" {
                    Some(el.text().collect::<String>())
                } else {
                    el.value().attr("content").map(|s| s.to_string())
                }
            })
            .unwrap_or_else(|| "Unknown Video".to_string())
            .replace(" - YouTube", "");
        
        // Look for stream URLs in various script tags and data attributes
        let url_patterns = vec![
            r#""url"":\s*""([^""]+)""#,
            r#"streamingData.*?url.*?""([^""]+)""#,
            r#"adaptiveFormats.*?url.*?""([^""]+)""#,
        ];
        
        for pattern in &url_patterns {
            let regex = Regex::new(pattern).map_err(|e| format!("URL pattern regex failed: {}", e))?;
            
            if let Some(captures) = regex.captures(html) {
                if let Some(url_match) = captures.get(1) {
                    let stream_url = url_match.as_str().to_string();
                    if stream_url.starts_with("https://") {
                        eprintln!("Found stream URL in mobile page: {}", &stream_url[..50.min(stream_url.len())]);
                        return Ok((title, stream_url));
                    }
                }
            }
        }
        
        Err("No stream URLs found in mobile page".to_string())
    }
    
    fn extract_from_api_response(data: &serde_json::Value, download_type: &str) -> Result<(String, String), String> {
        // Extract title
        let title = data
            .get("videoDetails")
            .and_then(|vd| vd.get("title"))
            .and_then(|t| t.as_str())
            .unwrap_or("Unknown Video")
            .to_string();
        
        // Extract stream URL based on download type
        let streaming_data = data
            .get("streamingData")
            .ok_or_else(|| "No streaming data in API response".to_string())?;
        
        let formats = if download_type == "mp3" {
            streaming_data.get("adaptiveFormats")
        } else {
            streaming_data.get("formats")
                .or_else(|| streaming_data.get("adaptiveFormats"))
        };
        
        let formats_array = formats
            .and_then(|f| f.as_array())
            .ok_or_else(|| "No formats array found".to_string())?;
        
        for format in formats_array {
            if let Some(url) = format.get("url").and_then(|u| u.as_str()) {
                let mime_type = format.get("mimeType")
                    .and_then(|m| m.as_str())
                    .unwrap_or("");
                
                let is_suitable = if download_type == "mp3" {
                    mime_type.contains("audio")
                } else {
                    mime_type.contains("video")
                };
                
                if is_suitable {
                    eprintln!("Found suitable stream in API response");
                    return Ok((title, url.to_string()));
                }
            }
        }
        
        Err("No suitable streams found in API response".to_string())
    }

    // Method 3: Enhanced Rustube with sophisticated retry logic and error handling
    async fn try_rustube_download(url: &str, download_type: &str) -> Result<(String, String), String> {
        eprintln!("Attempting enhanced Rustube extraction...");
        
        use rand::Rng;
        use rand::rngs::StdRng;
        use rand::SeedFromEntropy;
        
        // Multiple video ID extraction methods for robustness
        let video_id = match rustube::Id::from_raw(url) {
            Ok(id) => id,
            Err(_) => {
                // Fallback: extract manually
                use regex::Regex;
                let video_id_regex = Regex::new(r"(?:youtube\.com/(?:[^/]+/.+/|(?:v|e(?:mbed)?|watch)/|.*[?&]v=)|youtu\.be/|youtube\.com/embed/)([^'&?/\s]{11})")
                    .map_err(|e| format!("Video ID regex failed: {}", e))?;
                
                let video_id_str = video_id_regex
                    .captures(url)
                    .and_then(|caps| caps.get(1))
                    .ok_or_else(|| "Could not extract video ID from URL".to_string())?
                    .as_str();
                
                rustube::Id::from_raw(&format!("https://www.youtube.com/watch?v={}", video_id_str))
                    .map_err(|e| format!("Failed to create video ID: {}", e))?
            }
        };
        
        let mut rng = StdRng::from_entropy();
        
        // Enhanced retry with jitter and different strategies
        for attempt in 1..=5 {
            eprintln!("Enhanced Rustube attempt {} of 5", attempt);
            
            // Create fetcher with error handling
            let fetcher = rustube::VideoFetcher::from_id(video_id.clone().into_owned())
                .map_err(|e| format!("Create enhanced fetcher: {}", e))?;
            
            // Intelligent delay with jitter to avoid rate limiting patterns
            if attempt > 1 {
                let base_delay = (1000 * (2_u64.pow(attempt - 2))).min(10000); // Exponential with cap
                let jitter = rng.gen_range(0..1000); // Add randomness
                let delay = std::time::Duration::from_millis(base_delay + jitter);
                eprintln!("Waiting {:?} before enhanced retry...", delay);
                tokio::time::sleep(delay).await;
            }
            
            // Enhanced fetch with timeout
            let fetch_result = tokio::time::timeout(
                std::time::Duration::from_secs(20),
                fetcher.fetch()
            ).await;
            
            match fetch_result {
                Ok(Ok(video_descrambler)) => {
                    eprintln!("Enhanced Rustube fetch successful on attempt {}", attempt);
                    
                    let video_details = video_descrambler.video_details();
                    let video_title = video_details.title.clone();
                    
                    // Enhanced descrambling with timeout
                    let descramble_result = tokio::time::timeout(
                        std::time::Duration::from_secs(15),
                        async {
                            video_descrambler.descramble()
                        }
                    ).await;
                    
                    match descramble_result {
                        Ok(Ok(stream_data)) => {
                            eprintln!("Enhanced Rustube descramble successful");
                            
                            let streams = stream_data.streams();
                            eprintln!("Found {} streams", streams.len());
                            
                            // Enhanced stream selection with quality preferences
                            let selected_stream = if download_type == "mp3" {
                                // Prefer audio streams with highest bitrate
                                let audio_streams: Vec<_> = streams.iter()
                                    .filter(|s| s.mime.type_() == "audio")
                                    .collect();
                                
                                eprintln!("Found {} audio streams", audio_streams.len());
                                
                                audio_streams.iter()
                                    .max_by_key(|s| {
                                        let bitrate = s.bitrate.unwrap_or(0);
                                        let audio_quality = s.audio_quality.as_ref().map(|aq| format!("{:?}", aq)).unwrap_or_default();
                                        eprintln!("Audio stream: bitrate={}, quality={}", bitrate, audio_quality);
                                        bitrate
                                    })
                                    .copied()
                            } else {
                                // Prefer video streams with good balance of quality and bitrate
                                let video_streams: Vec<_> = streams.iter()
                                    .filter(|s| s.mime.type_() == "video" && s.includes_video_track)
                                    .collect();
                                
                                eprintln!("Found {} video streams", video_streams.len());
                                
                                video_streams.iter()
                                    .max_by_key(|s| {
                                        let bitrate = s.bitrate.unwrap_or(0);
                                        let quality_score = s.quality_label.as_ref()
                                            .and_then(|ql| {
                                                let ql_str = format!("{:?}", ql);
                                                ql_str.chars().take_while(|c| c.is_numeric()).collect::<String>().parse::<u64>().ok()
                                            })
                                            .unwrap_or(0);
                                        eprintln!("Video stream: bitrate={}, quality={}", bitrate, quality_score);
                                        bitrate / 1000 + quality_score * 100 // Balance bitrate and resolution
                                    })
                                    .copied()
                            };
                            
                            if let Some(stream) = selected_stream {
                                // Enhanced URL extraction with validation
                                let stream_url = stream.signature_cipher.url.to_string();
                                
                                // Validate URL format
                                if stream_url.starts_with("https://") && (stream_url.contains("googlevideo.com") || stream_url.contains("youtube.com")) {
                                    eprintln!("Enhanced Rustube extraction successful with URL: {}...", &stream_url[..50.min(stream_url.len())]);
                                    return Ok((video_title, stream_url));
                                } else {
                                    eprintln!("Invalid stream URL format: {}...", &stream_url[..30.min(stream_url.len())]);
                                    continue;
                                }
                            } else {
                                eprintln!("No suitable {} stream found in enhanced rustube (available: {})", 
                                         download_type, 
                                         streams.iter().map(|s| format!("{}:{}", s.mime.type_(), s.bitrate.unwrap_or(0))).collect::<Vec<_>>().join(", "));
                            }
                        }
                        Ok(Err(e)) => {
                            eprintln!("Enhanced Rustube descramble failed on attempt {}: {}", attempt, e);
                            continue;
                        }
                        Err(_) => {
                            eprintln!("Enhanced Rustube descramble timeout on attempt {}", attempt);
                            continue;
                        }
                    }
                }
                Ok(Err(e)) => {
                    eprintln!("Enhanced Rustube fetch failed on attempt {}: {}", attempt, e);
                    continue;
                }
                Err(_) => {
                    eprintln!("Enhanced Rustube fetch timeout on attempt {}", attempt);
                    continue;
                }
            }
        }
        
        Err("All enhanced Rustube download attempts failed after 5 tries with sophisticated retry logic".to_string())
    }

    // Cascading fallback system implementation
    {
        let mut p = progress_state.lock().unwrap();
        p.status = "extracting".into();
        p.percentage = 10.0;
        let _ = window.emit("download-progress", p.clone());
    }

    // Method 1: Advanced YouTube API extraction (Primary)
    let (video_title, download_url, content_bytes) =
    match try_youtube_api_extraction(url, download_type, quality).await {
        Ok((title, url, bytes)) => {
            eprintln!("✅ Advanced API extraction successful");
            (title, url, Some(bytes))
        }
        Err(api_error) => {
            eprintln!("❌ Advanced API extraction failed: {}", api_error);
            
            // Method 2: Fallback extraction (Secondary)
            match try_fallback_extraction(url, download_type).await {
                Ok((title, stream_url)) => {
                    eprintln!("✅ Fallback extraction successful");
                    (title, stream_url, None)
                }
                Err(fallback_error) => {
                    eprintln!("❌ Fallback extraction failed: {}", fallback_error);
                    
                    // Method 3: Enhanced Rustube (Tertiary)
                    match try_rustube_download(url, download_type).await {
                        Ok((title, stream_url)) => {
                            eprintln!("✅ Enhanced Rustube extraction successful");
                            (title, stream_url, None)
                        }
                        Err(rustube_error) => {
                            eprintln!("❌ All extraction methods failed");
                            return Err(format!(
                                "All YouTube extraction methods failed:\n\
                                1. Advanced API extraction: {}\n\
                                2. Fallback extraction: {}\n\
                                3. Enhanced Rustube: {}\n\
                                \n\
                                YouTube may have updated their anti-bot measures. The app will be updated to handle these changes.",
                                api_error, fallback_error, rustube_error
                            ));
                        }
                    }
                }
            }
        }
    };

    // Update progress for download phase
    {
        let mut p = progress_state.lock().unwrap();
        p.status = "downloading".into();
        p.percentage = 25.0;
        let _ = window.emit("download-progress", p.clone());
    }

    // Check if content was already downloaded by yt-dlp crate
    let has_content_bytes = content_bytes.is_some();
    
    // Download the content (if not already downloaded by yt-dlp crate)
    let file_content = if let Some(bytes) = content_bytes {
        bytes
    } else {
        eprintln!("Downloading content from extracted URL...");
        
        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (Linux; Android 10; SM-G975F) AppleWebKit/537.36")
            .build()
            .map_err(|e| format!("Failed to create download client: {}", e))?;
        
        let response = client
            .get(&download_url)
            .send()
            .await
            .map_err(|e| format!("Failed to download content: {}", e))?;
        
        if !response.status().is_success() {
            return Err(format!("Download failed with status: {}", response.status()));
        }
        
        response
            .bytes()
            .await
            .map_err(|e| format!("Failed to read download content: {}", e))?
            .to_vec()
    };

    // Update progress for file writing
    {
        let mut p = progress_state.lock().unwrap();
        p.status = "saving".into();
        p.percentage = 80.0;
        let _ = window.emit("download-progress", p.clone());
    }

    // Save the file
    let out_dir = Path::new(output_folder);
    let extension = if download_type == "mp3" { 
        if has_content_bytes { "mp3" } else { "m4a" }
    } else { 
        "mp4" 
    };
    
    let sanitized_title = video_title
        .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_")
        .chars()
        .take(100)  // Limit filename length
        .collect::<String>();
    
    let filename = format!("{}.{}", sanitized_title, extension);
    let file_path = out_dir.join(&filename);
    
    eprintln!("Saving file: {}", file_path.display());
    
    fs::write(&file_path, &file_content)
        .await
        .map_err(|e| format!("Failed to write file {}: {}", file_path.display(), e))?;
    
    // Final progress update
    {
        let mut p = progress_state.lock().unwrap();
        p.status = "completed".into();
        p.percentage = 100.0;
        p.bytes_downloaded = file_content.len() as u64;
        p.total_bytes = file_content.len() as u64;
        let _ = window.emit("download-progress", p.clone());
    }

    eprintln!("✅ Android download completed successfully: {}", filename);

    Ok(filename)
}
