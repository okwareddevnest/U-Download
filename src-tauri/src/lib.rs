// use regex::Regex; // Only used on non-Android platforms
#[cfg(not(target_os = "android"))]
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::sync::{Arc, Mutex};
#[cfg(not(target_os = "android"))]
use tauri::menu::{Menu, MenuItem};
#[cfg(not(target_os = "android"))]
use tauri::tray::TrayIconBuilder;
#[cfg(not(target_os = "android"))]
use tauri::Manager;
use tauri::{AppHandle, Emitter, State, Window, Runtime};
#[cfg(not(target_os = "android"))]
use tauri_plugin_dialog::DialogExt;

mod binary_manager;


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

type ProgressState = Arc<Mutex<DownloadProgress>>;

fn format_speed(bytes_per_sec: u64) -> String {
    if bytes_per_sec == 0 {
        return "Calculating...".to_string();
    }
    
    if bytes_per_sec < 10 {
        return "Starting...".to_string();
    }
    
    const UNITS: &[&str] = &["B/s", "kB/s", "MB/s", "GB/s"];
    let mut speed = bytes_per_sec as f64;
    let mut unit_index = 0;
    
    while speed >= 1024.0 && unit_index < UNITS.len() - 1 {
        speed /= 1024.0;
        unit_index += 1;
    }
    
    let formatted = if speed >= 100.0 {
        format!("{:.0}", speed)
    } else if speed >= 10.0 {
        format!("{:.1}", speed)
    } else if speed >= 1.0 {
        format!("{:.2}", speed)
    } else {
        format!("{:.3}", speed)
    };
    
    format!("{} {}", formatted, UNITS[unit_index])
}

fn parse_bytes_from_yt_dlp_size(size_str: &str) -> u64 {
    let size_str = size_str.trim().replace(",", ""); // Remove commas
    eprintln!("Parsing size string: '{}'", size_str);
    
    // Handle "Unknown" or empty strings
    if size_str.is_empty() || size_str.to_lowercase() == "unknown" {
        return 0;
    }
    
    // Find the position where unit starts (first alphabetic character)
    let (number_part, unit_part) = if let Some(pos) = size_str.find(char::is_alphabetic) {
        (&size_str[..pos], &size_str[pos..])
    } else {
        (size_str.as_str(), "")
    };
    
    let number: f64 = number_part.parse().unwrap_or_else(|_| {
        eprintln!("Failed to parse number: '{}'", number_part);
        0.0
    });
    
    let multiplier = match unit_part.to_uppercase().as_str() {
        "B" | "BYTES" => 1.0,
        "K" | "KB" | "KIB" => 1024.0,
        "M" | "MB" | "MIB" | "MBYTES" => 1024.0 * 1024.0,
        "G" | "GB" | "GIB" | "GBYTES" => 1024.0 * 1024.0 * 1024.0,
        "T" | "TB" | "TIB" | "TBYTES" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        // Handle speed units (remove /s)
        "KB/S" | "KIB/S" => 1024.0,
        "MB/S" | "MIB/S" => 1024.0 * 1024.0,
        "GB/S" | "GIB/S" => 1024.0 * 1024.0 * 1024.0,
        "" => 1.0, // assume bytes if no unit
        _ => {
            eprintln!("Unknown unit: '{}', assuming bytes", unit_part);
            1.0
        }
    };
    
    let result = (number * multiplier) as u64;
    eprintln!("Parsed '{}' as {} bytes", size_str, result);
    result
}

fn calculate_eta(bytes_downloaded: u64, total_bytes: u64, speed_bytes_per_sec: u64) -> String {
    if speed_bytes_per_sec == 0 {
        return "Calculating...".to_string();
    }
    
    if total_bytes == 0 || bytes_downloaded >= total_bytes {
        return "Complete".to_string();
    }
    
    if speed_bytes_per_sec < 10 {
        return "Starting...".to_string();
    }
    
    let remaining_bytes = total_bytes.saturating_sub(bytes_downloaded);
    if remaining_bytes == 0 {
        return "Complete".to_string();
    }
    
    let eta_seconds = remaining_bytes / speed_bytes_per_sec;
    
    // Handle very long ETAs (more than 24 hours)
    if eta_seconds > 86400 {
        let days = eta_seconds / 86400;
        return format!("{}d+", days);
    }
    
    let hours = eta_seconds / 3600;
    let minutes = (eta_seconds % 3600) / 60;
    let seconds = eta_seconds % 60;
    
    if hours > 0 {
        format!("{}:{:02}:{:02}", hours, minutes, seconds)
    } else if minutes > 0 {
        format!("{}:{:02}", minutes, seconds)
    } else {
        format!("{}s", seconds.max(1))
    }
}

fn send_download_complete_notification(_filename: &str) -> Result<(), String> { Ok(()) }
fn send_download_error_notification(_error: &str) -> Result<(), String> { Ok(()) }
fn send_download_started_notification(_filename: &str) -> Result<(), String> { Ok(()) }

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
        let result = perform_download(
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

async fn perform_download<R: Runtime>(
    window: &Window<R>,
    progress_state: ProgressState,
    url: &str,
    download_type: &str,
    quality: &str,
    output_folder: &str,
    start_time: Option<f64>,
    end_time: Option<f64>,
) -> Result<String, String> {
    #[cfg(target_os = "android")]
    {
        return perform_download_android(
            window,
            progress_state,
            url,
            download_type,
            quality,
            output_folder,
            start_time,
            end_time,
        )
        .await;
    }

    #[cfg(not(target_os = "android"))]
    {
        // Unified flow for desktop platforms
        let app_handle = window.app_handle();
    let paths = binary_manager::resolve_paths(&app_handle)?;
    binary_manager::ensure_executable(&paths)?;

    // First, test if yt-dlp is available
    match Command::new(&paths.yt_dlp).arg("--version").output() {
        Ok(output) => {
            let version = String::from_utf8_lossy(&output.stdout);
            eprintln!("yt-dlp version: {}", version.trim());
        }
        Err(e) => {
            return Err(format!("Bundled yt-dlp not found or not executable: {}", e));
        }
    }

    // Test if aria2c is available (skip on Android)
    #[cfg(not(target_os = "android"))]
    {
        match Command::new(&paths.aria2c).arg("--version").output() {
            Ok(output) => {
                let version = String::from_utf8_lossy(&output.stdout);
                eprintln!(
                    "aria2c version: {}",
                    version.lines().next().unwrap_or("unknown")
                );
            }
            Err(e) => {
                return Err(format!("Bundled aria2c not found or not executable: {}", e));
            }
        }
    }

    // Check if FFmpeg is available for trimming
    let trimming_enabled = start_time.is_some() || end_time.is_some();
    if trimming_enabled {
        match Command::new(&paths.ffmpeg).arg("-version").output() {
            Ok(_) => {
                eprintln!("FFmpeg is available for trimming");
            }
            Err(e) => {
                return Err(format!("Bundled FFmpeg not found or not executable: {}", e));
            }
        }
    }

    let mut cmd = Command::new(&paths.yt_dlp);
    // Ensure yt-dlp can find bundled aria2c and ffmpeg
    binary_manager::augment_path_env(&mut cmd, &paths.dir);

    // Basic arguments for better quality and performance
    #[cfg(not(target_os = "android"))]
    {
        cmd.arg("--external-downloader")
            .arg("aria2c")
            .arg("--external-downloader-args")
            .arg("-x 16 -s 16 -k 1M");
    }
    cmd.arg("--progress")
        .arg("--newline")
        .arg("--merge-output-format")
        .arg("mp4")
        .arg("--prefer-free-formats")
        .arg("--ffmpeg-location")
        .arg(&paths.ffmpeg);

    // Format selection based on type and quality
    match download_type {
        "mp3" => {
            cmd.arg("-x")
                .arg("--audio-format")
                .arg("mp3")
                .arg("--audio-quality")
                .arg("192K");
        }
        "mp4" => {
            // Improved format selection for better video quality
            let format_selector = match quality {
                "360" => "bestvideo[height<=360]+bestaudio/best[height<=360]",
                "480" => "bestvideo[height<=480]+bestaudio/best[height<=480]",
                "720" => "bestvideo[height<=720]+bestaudio/best[height<=720]",
                "1080" => "bestvideo[height<=1080]+bestaudio/best[height<=1080]",
                "best" => "bestvideo+bestaudio/best",
                _ => "bestvideo+bestaudio/best",
            };
            cmd.arg("-f").arg(format_selector);
        }
        _ => return Err("Invalid download type".to_string()),
    }

    // For trimming, we'll download the full video first, then trim with FFmpeg
    // Set a temporary output pattern that we can identify later
    let temp_output_pattern = if trimming_enabled {
        format!("{}/%(title)s_temp.%(ext)s", output_folder)
    } else {
        format!("{}/%(title)s.%(ext)s", output_folder)
    };

    cmd.arg("-o").arg(&temp_output_pattern);

    cmd.arg(url);

    // Log the full command for debugging
    eprintln!("Executing command: {:?}", cmd);

    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            format!(
                "Failed to start bundled yt-dlp: {}. This is an application error; please reinstall or report a bug.",
                e
            )
        })?;

    // Get video title for notification
    let video_title = match get_video_metadata(app_handle.clone(), url.to_string()).await {
        Ok(metadata) => metadata.title,
        Err(_) => "Unknown Video".to_string(),
    };

    // Send download start notification
    let _ = send_download_started_notification(&video_title);

    // Initialize download start time and periodic update task
    {
        let mut progress = progress_state.lock().unwrap();
        progress.download_start_time = std::time::SystemTime::now();
        progress.status = "downloading".to_string();
        progress.percentage = 0.0;
        progress.bytes_downloaded = 0;
        progress.total_bytes = 0;
    }

    // Start periodic progress update task
    let periodic_progress_state = progress_state.clone();
    let periodic_window = window.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
        let mut last_percentage = 0.0;
        let mut last_update_time = std::time::SystemTime::now();
        
        loop {
            interval.tick().await;
            
            let now = std::time::SystemTime::now();
            let should_update = {
                let mut progress = periodic_progress_state.lock().unwrap();
                
                if progress.status != "downloading" {
                    break; // Exit if download is no longer active
                }
                
                let elapsed_since_last = now.duration_since(last_update_time).unwrap_or_default();
                
                // Calculate speed based on percentage change if no real speed data
                if progress.speed_bytes_per_sec == 0 && progress.percentage > last_percentage {
                    let percentage_change = progress.percentage - last_percentage;
                    let elapsed_secs = elapsed_since_last.as_secs_f64().max(0.1);
                    
                    if percentage_change > 0.0 {
                        // Estimate speed based on percentage progress over time
                        let estimated_total_bytes = if progress.total_bytes > 0 {
                            progress.total_bytes
                        } else {
                            100_000_000 // 100MB default estimate
                        };
                        
                        let bytes_for_percentage = ((percentage_change / 100.0) * estimated_total_bytes as f64) as u64;
                        let estimated_speed = (bytes_for_percentage as f64 / elapsed_secs) as u64;
                        
                        progress.speed_bytes_per_sec = estimated_speed;
                        progress.speed = format_speed(estimated_speed);
                        
                        // Update ETA
                        let remaining_percentage = 100.0 - progress.percentage;
                        if remaining_percentage > 0.0 && estimated_speed > 0 {
                            progress.eta = calculate_eta(progress.bytes_downloaded, progress.total_bytes, estimated_speed);
                        }
                    }
                }
                
                last_percentage = progress.percentage;
                last_update_time = now;
                
                progress.clone()
            };
            
            // Send periodic update to frontend
            let _ = periodic_window.emit("download-progress", should_update);
        }
    });

    // Monitor the process output with comprehensive parsing
    if let Some(stdout) = child.stdout.take() {
        use std::io::{BufRead, BufReader};
        let reader = BufReader::new(stdout);

        // Regex patterns for different output formats
        let dl_status_regex = Regex::new(r"\[DL:([\d.]+)([GMK]?)iB\]").unwrap(); // aria2c download status
        let fragment_regex = Regex::new(r"\[hlsnative\]\s+Total fragments:\s+(\d+)").unwrap(); // HLS fragment count
        let standard_progress_patterns = vec![
            // Standard yt-dlp progress patterns
            Regex::new(r"\[download\]\s+(\d+\.?\d*)%\s+of\s+(\S+)\s+at\s+(\S+/s)\s+ETA\s+(\S+)").unwrap(),
            Regex::new(r"\[download\]\s+(\d+\.?\d*)%\s+of\s+(\S+)\s+at\s+(\S+/s).*?ETA\s+(\S+)").unwrap(),
            Regex::new(r"\[download\]\s+(\d+\.?\d*)%.*?at\s+(\S+/s).*?ETA\s+(\S+)").unwrap(),
            Regex::new(r"\[download\]\s+(\d+\.?\d*)%.*?at\s+(\S+/s)").unwrap(),
            Regex::new(r"\[download\]\s+(\d+\.?\d*)%\s+of\s+(\S+)").unwrap(),
            Regex::new(r"\[download\]\s+(\d+\.?\d*)%").unwrap(),
        ];

        let mut total_fragments = 0u32;
        let mut current_fragments = 0u32;
        let mut last_dl_size = 0u64;
        let mut accumulated_size = 0u64;

        for line in reader.lines() {
            if let Ok(line) = line {
                eprintln!("yt-dlp output: {}", line);
                let now = std::time::SystemTime::now();
                let mut progress_updated = false;

                // 1. Check for total fragments count (HLS streams)
                if let Some(captures) = fragment_regex.captures(&line) {
                    if let Ok(fragments) = captures.get(1).unwrap().as_str().parse::<u32>() {
                        total_fragments = fragments;
                        eprintln!("Found total fragments: {}", total_fragments);
                    }
                }

                // 2. Parse aria2c download status lines: [DL:4.1MiB][#hash size/totalsize][...]
                if let Some(captures) = dl_status_regex.captures(&line) {
                    let size_num: f64 = captures.get(1).unwrap().as_str().parse().unwrap_or(0.0);
                    let size_unit = captures.get(2).map(|m| m.as_str()).unwrap_or("");
                    
                    // Convert to bytes
                    let current_size = match size_unit {
                        "G" => (size_num * 1024.0 * 1024.0 * 1024.0) as u64,
                        "M" => (size_num * 1024.0 * 1024.0) as u64,
                        "K" => (size_num * 1024.0) as u64,
                        _ => size_num as u64,
                    };

                    eprintln!("aria2c DL status: {} {} = {} bytes", size_num, size_unit, current_size);
                    
                    // Update accumulated size
                    if current_size > last_dl_size {
                        accumulated_size += current_size - last_dl_size;
                    } else {
                        accumulated_size += current_size; // New fragment started
                    }
                    last_dl_size = current_size;

                    // Calculate progress based on fragments if we know the total
                    let (percentage, estimated_speed) = if total_fragments > 0 {
                        // Count completed fragments by counting how many times we see repeated sizes
                        current_fragments += 1;
                        let progress = (current_fragments as f64 / total_fragments as f64) * 100.0;
                        
                        // Calculate speed based on accumulated data
                        let elapsed = now.duration_since({
                            let progress = progress_state.lock().unwrap();
                            progress.download_start_time
                        }).unwrap_or_default();
                        let elapsed_secs = elapsed.as_secs_f64().max(0.1);
                        let speed = (accumulated_size as f64 / elapsed_secs) as u64;
                        
                        (progress.min(100.0), speed)
                    } else {
                        // Estimate progress based on download size (rough estimation)
                        // Assume an average video is around 100MB to 1GB
                        let estimated_total = 500_000_000u64; // 500MB estimate
                        let progress = ((accumulated_size as f64 / estimated_total as f64) * 100.0).min(95.0);
                        
                        let elapsed = now.duration_since({
                            let progress = progress_state.lock().unwrap();
                            progress.download_start_time
                        }).unwrap_or_default();
                        let elapsed_secs = elapsed.as_secs_f64().max(0.1);
                        let speed = (accumulated_size as f64 / elapsed_secs) as u64;
                        
                        (progress, speed)
                    };

                    // Update progress state
                    {
                        let mut progress = progress_state.lock().unwrap();
                        progress.percentage = percentage;
                        progress.bytes_downloaded = accumulated_size;
                        
                        if total_fragments > 0 {
                            // For HLS streams, estimate total size based on average fragment size
                            let avg_fragment_size = if current_fragments > 0 {
                                accumulated_size / current_fragments as u64
                            } else {
                                current_size
                            };
                            progress.total_bytes = avg_fragment_size * total_fragments as u64;
                        } else {
                            progress.total_bytes = (accumulated_size as f64 / (percentage / 100.0).max(0.01)) as u64;
                        }
                        
                        progress.speed_bytes_per_sec = estimated_speed;
                        progress.speed = format_speed(estimated_speed);
                        progress.eta = calculate_eta(accumulated_size, progress.total_bytes, estimated_speed);
                        progress.status = "downloading".to_string();
                        
                        eprintln!("aria2c Progress: {:.1}% | {} | bytes: {} | fragments: {}/{}", 
                                 percentage, progress.speed, accumulated_size, current_fragments, total_fragments);
                    }

                    let progress_copy = {
                        let progress = progress_state.lock().unwrap();
                        progress.clone()
                    };

                    let _ = window.emit("download-progress", progress_copy);
                    progress_updated = true;
                }

                // 3. Try standard yt-dlp progress patterns as fallback
                if !progress_updated {
                    for (pattern_index, pattern) in standard_progress_patterns.iter().enumerate() {
                        if let Some(captures) = pattern.captures(&line) {
                            eprintln!("Matched standard pattern {}: {:?}", pattern_index, captures);
                            
                            let percentage: f64 = captures.get(1)
                                .and_then(|m| m.as_str().parse().ok())
                                .unwrap_or(0.0);
                            
                            let total_size_str = match pattern_index {
                                0 | 1 | 4 => captures.get(2).map(|m| m.as_str()),
                                _ => None,
                            };
                            
                            let speed_str = match pattern_index {
                                0 | 1 => captures.get(3).map(|m| m.as_str()),
                                2 | 3 => captures.get(2).map(|m| m.as_str()),
                                _ => None,
                            };
                            
                            let eta_str = match pattern_index {
                                0 | 1 => captures.get(4).map(|m| m.as_str()),
                                2 => captures.get(3).map(|m| m.as_str()),
                                _ => None,
                            };

                            let total_bytes = total_size_str
                                .map(|s| parse_bytes_from_yt_dlp_size(s))
                                .unwrap_or(0);
                            
                            let bytes_downloaded = if total_bytes > 0 {
                                ((percentage / 100.0) * total_bytes as f64) as u64
                            } else {
                                0
                            };
                            
                            let parsed_speed_bytes = speed_str
                                .map(|s| parse_bytes_from_yt_dlp_size(&s.replace("/s", "")))
                                .unwrap_or(0);

                            {
                                let mut progress = progress_state.lock().unwrap();
                                progress.percentage = percentage;
                                
                                if total_bytes > 0 {
                                    progress.bytes_downloaded = bytes_downloaded;
                                    progress.total_bytes = total_bytes;
                                }
                                
                                if parsed_speed_bytes > 0 {
                                    progress.speed_bytes_per_sec = parsed_speed_bytes;
                                    progress.speed = format_speed(parsed_speed_bytes);
                                }
                                
                                progress.eta = eta_str.map(|s| s.to_string())
                                    .unwrap_or_else(|| calculate_eta(bytes_downloaded, total_bytes, progress.speed_bytes_per_sec));
                                
                                progress.status = "downloading".to_string();
                                
                                eprintln!("Standard progress: {}% | {} | ETA: {}", 
                                         progress.percentage, progress.speed, progress.eta);
                            }

                            let progress_copy = {
                                let progress = progress_state.lock().unwrap();
                                progress.clone()
                            };

                            let _ = window.emit("download-progress", progress_copy);
                            progress_updated = true;
                            break;
                        }
                    }
                }

                // 4. Final fallback: look for any percentage in download-related lines
                if !progress_updated && (line.contains("[download]") || line.contains("DL:")) {
                    if let Some(percent_match) = Regex::new(r"(\d+\.?\d*)%").unwrap().find(&line) {
                        if let Ok(percentage) = percent_match.as_str().trim_end_matches('%').parse::<f64>() {
                            eprintln!("Fallback percentage: {}%", percentage);
                            
                            let mut progress = progress_state.lock().unwrap();
                            if percentage > progress.percentage {
                                progress.percentage = percentage;
                                
                                // Estimate speed from percentage change
                                let elapsed = now.duration_since(progress.download_start_time).unwrap_or_default();
                                let elapsed_secs = elapsed.as_secs_f64().max(0.1);
                                
                                if progress.speed_bytes_per_sec == 0 && percentage > 0.0 {
                                    let estimated_total = 200_000_000_u64; // 200MB estimate
                                    let estimated_downloaded = ((percentage / 100.0) * estimated_total as f64) as u64;
                                    let estimated_speed = (estimated_downloaded as f64 / elapsed_secs) as u64;
                                    
                                    progress.speed_bytes_per_sec = estimated_speed;
                                    progress.speed = format_speed(estimated_speed);
                                    progress.eta = calculate_eta(estimated_downloaded, estimated_total, estimated_speed);
                                }
                                
                                let progress_copy = progress.clone();
                                drop(progress);
                                let _ = window.emit("download-progress", progress_copy);
                            }
                        }
                    }
                }
            }
        }
    }

    // Also capture stderr for error details
    let stderr_output = if let Some(stderr) = child.stderr.take() {
        use std::io::Read;
        let mut error_msg = String::new();
        let mut stderr_reader = stderr;
        let _ = stderr_reader.read_to_string(&mut error_msg);
        error_msg
    } else {
        String::new()
    };

    let output = child.wait().map_err(|e| format!("Process error: {}", e))?;

    if output.success() {
        // If trimming is enabled, perform FFmpeg trimming
        if trimming_enabled {
            perform_trimming(window, progress_state, output_folder, start_time, end_time, paths.ffmpeg.clone()).await?;
        }
        Ok(video_title)
    } else {
        let exit_code = output.code().unwrap_or(-1);
        let error_msg = if !stderr_output.is_empty() {
            format!(
                "yt-dlp failed (exit code {}): {}",
                exit_code,
                stderr_output.trim()
            )
        } else {
            format!("yt-dlp failed with exit code {}", exit_code)
        };
        eprintln!("Download failed: {}", error_msg);
        Err(error_msg)
    }
    } // Close #[cfg(not(target_os = "android"))] block
}

async fn perform_trimming<R: Runtime>(
    window: &Window<R>,
    progress_state: ProgressState,
    output_folder: &str,
    start_time: Option<f64>,
    end_time: Option<f64>,
    ffmpeg_path: std::path::PathBuf,
) -> Result<(), String> {
    use std::fs;
    use std::path::Path;

    // Find the downloaded file (it should have "_temp" in the name)
    let folder_path = Path::new(output_folder);
    let temp_files: Vec<_> = fs::read_dir(folder_path)
        .map_err(|e| format!("Failed to read output directory: {}", e))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().contains("_temp"))
        .collect();

    if temp_files.is_empty() {
        return Err("No temporary file found for trimming".to_string());
    }

    let temp_file = &temp_files[0];
    let temp_path = temp_file.path();
    let file_name_str = temp_file.file_name().to_string_lossy().to_string();

    // Create the final output filename (remove "_temp")
    let final_name = file_name_str.replace("_temp", "");
    let final_path = folder_path.join(final_name);

    let mut ffmpeg_cmd = Command::new(&ffmpeg_path);

    // Add input file
    ffmpeg_cmd.arg("-i").arg(&temp_path);

    // Add trimming parameters
    if let Some(start) = start_time {
        ffmpeg_cmd.arg("-ss").arg(format!("{}", start));
    }

    if let Some(end) = end_time {
        ffmpeg_cmd
            .arg("-t")
            .arg(format!("{}", end - start_time.unwrap_or(0.0)));
    }

    // Copy codecs and avoid re-encoding for speed
    ffmpeg_cmd.arg("-c").arg("copy");

    // Set output file
    ffmpeg_cmd.arg(&final_path);

    // Hide FFmpeg output for cleaner logs
    ffmpeg_cmd.arg("-hide_banner").arg("-loglevel").arg("error");

    eprintln!("Executing FFmpeg trimming: {:?}", ffmpeg_cmd);

    {
        let mut progress = progress_state.lock().unwrap();
        progress.status = "trimming".to_string();
        progress.percentage = 0.0;
        let progress_copy = progress.clone();
        let _ = window.emit("download-progress", progress_copy);
    }

    let ffmpeg_output = ffmpeg_cmd
        .output()
        .map_err(|e| format!("Failed to run FFmpeg: {}", e))?;

    if ffmpeg_output.status.success() {
        // Remove the temporary file
        if let Err(e) = fs::remove_file(&temp_path) {
            eprintln!("Warning: Failed to remove temporary file: {}", e);
        }

        {
            let mut progress = progress_state.lock().unwrap();
            progress.status = "completed".to_string();
            progress.percentage = 100.0;
            let progress_copy = progress.clone();
            let _ = window.emit("download-progress", progress_copy);
        }

        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&ffmpeg_output.stderr);
        Err(format!("FFmpeg trimming failed: {}", stderr))
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(progress_state)
        .invoke_handler(tauri::generate_handler![
            select_output_folder,
            start_download,
            test_dependencies,
            get_video_metadata,
            check_ffmpeg,
            get_shared_url,
            get_android_videos_dir
        ])
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
