use regex::Regex;
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::Manager;
use tauri::{AppHandle, Emitter, State, Window};
use tauri_plugin_dialog::DialogExt;
use notify_rust::Notification;

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

fn send_notification(title: &str, body: &str) -> Result<(), String> {
    Notification::new()
        .summary(title)
        .body(body)
        .icon("u-download") // Will fallback to default if icon not found
        .timeout(0) // Use system default timeout
        .show()
        .map_err(|e| format!("Failed to show notification: {}", e))?;
    
    Ok(())
}

fn send_download_complete_notification(filename: &str) -> Result<(), String> {
    send_notification(
        "Download Complete! 🎉",
        &format!("Successfully downloaded: {}", filename),
    )
}

fn send_download_error_notification(error: &str) -> Result<(), String> {
    send_notification(
        "Download Failed ❌",
        &format!("Download error: {}", error),
    )
}

fn send_download_started_notification(filename: &str) -> Result<(), String> {
    send_notification(
        "Download Started 🚀",
        &format!("Started downloading: {}", filename),
    )
}

#[tauri::command]
async fn get_video_metadata(url: String) -> Result<VideoMetadata, String> {
    // Test if yt-dlp is available
    match Command::new("yt-dlp").arg("--version").output() {
        Ok(_) => {}
        Err(e) => {
            return Err(format!("yt-dlp not found: {}. Please install yt-dlp.", e));
        }
    }

    // Get video information using yt-dlp --dump-json
    let output = Command::new("yt-dlp")
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

#[tauri::command]
async fn check_ffmpeg() -> Result<String, String> {
    match Command::new("ffmpeg").arg("-version").output() {
        Ok(output) => {
            let version = String::from_utf8_lossy(&output.stdout);
            Ok(format!(
                "✅ FFmpeg: {}",
                version.lines().next().unwrap_or("unknown")
            ))
        }
        Err(e) => Err(format!(
            "❌ FFmpeg: Not found ({}). Please install with: sudo apt install ffmpeg",
            e
        )),
    }
}

#[tauri::command]
async fn select_output_folder(app_handle: AppHandle) -> Result<String, String> {
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

#[tauri::command]
async fn start_download(
    window: Window,
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
async fn test_dependencies() -> Result<String, String> {
    let mut results = Vec::new();

    // Test yt-dlp
    match Command::new("yt-dlp").arg("--version").output() {
        Ok(output) => {
            let version = String::from_utf8_lossy(&output.stdout);
            results.push(format!("✅ yt-dlp: {}", version.trim()));
        }
        Err(e) => {
            results.push(format!("❌ yt-dlp: Not found ({})", e));
        }
    }

    // Test aria2c
    match Command::new("aria2c").arg("--version").output() {
        Ok(output) => {
            let version = String::from_utf8_lossy(&output.stdout);
            results.push(format!(
                "✅ aria2c: {}",
                version.lines().next().unwrap_or("unknown")
            ));
        }
        Err(e) => {
            results.push(format!("❌ aria2c: Not found ({})", e));
        }
    }

    Ok(results.join("\n"))
}

async fn perform_download(
    window: &Window,
    progress_state: ProgressState,
    url: &str,
    download_type: &str,
    quality: &str,
    output_folder: &str,
    start_time: Option<f64>,
    end_time: Option<f64>,
) -> Result<String, String> {
    // First, test if yt-dlp is available
    match Command::new("yt-dlp").arg("--version").output() {
        Ok(output) => {
            let version = String::from_utf8_lossy(&output.stdout);
            eprintln!("yt-dlp version: {}", version.trim());
        }
        Err(e) => {
            return Err(format!("yt-dlp not found or not executable: {}. Please install with: sudo apt install yt-dlp", e));
        }
    }

    // Test if aria2c is available
    match Command::new("aria2c").arg("--version").output() {
        Ok(output) => {
            let version = String::from_utf8_lossy(&output.stdout);
            eprintln!(
                "aria2c version: {}",
                version.lines().next().unwrap_or("unknown")
            );
        }
        Err(e) => {
            return Err(format!("aria2c not found or not executable: {}. Please install with: sudo apt install aria2", e));
        }
    }

    // Check if FFmpeg is available for trimming
    let trimming_enabled = start_time.is_some() || end_time.is_some();
    if trimming_enabled {
        match Command::new("ffmpeg").arg("-version").output() {
            Ok(_) => {
                eprintln!("FFmpeg is available for trimming");
            }
            Err(e) => {
                return Err(format!("FFmpeg not found or not executable: {}. Please install with: sudo apt install ffmpeg", e));
            }
        }
    }

    let mut cmd = Command::new("yt-dlp");

    // Basic arguments for better quality and performance
    cmd.arg("--external-downloader")
        .arg("aria2c")
        .arg("--external-downloader-args")
        .arg("-x 16 -s 16 -k 1M")
        .arg("--progress")
        .arg("--newline")
        .arg("--merge-output-format")
        .arg("mp4")
        .arg("--prefer-free-formats");

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
                "Failed to start yt-dlp: {}. Make sure yt-dlp and aria2c are installed.",
                e
            )
        })?;

    // Get video title for notification
    let video_title = match get_video_metadata(url.to_string()).await {
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
            perform_trimming(window, progress_state, output_folder, start_time, end_time).await?;
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
}

async fn perform_trimming(
    window: &Window,
    progress_state: ProgressState,
    output_folder: &str,
    start_time: Option<f64>,
    end_time: Option<f64>,
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

    let mut ffmpeg_cmd = Command::new("ffmpeg");

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
        .manage(progress_state)
        .invoke_handler(tauri::generate_handler![
            select_output_folder,
            start_download,
            test_dependencies,
            get_video_metadata,
            check_ffmpeg
        ])
        .setup(move |app| {
            let show_item = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .tooltip("U-Download")
                .on_menu_event(|app, event| {
                    if event.id.as_ref() == "show" {
                        println!("Show menu item clicked");
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    } else if event.id.as_ref() == "quit" {
                        println!("Quit menu item clicked");

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
                                    println!("User confirmed quit");
                                    std::thread::spawn(move || {
                                        app_handle.exit(0);
                                    });
                                } else {
                                    println!("User canceled quit");
                                }
                            });
                    }
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                println!("Close button clicked: hiding window instead of quitting");
                window.hide().unwrap();
                api.prevent_close();
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
