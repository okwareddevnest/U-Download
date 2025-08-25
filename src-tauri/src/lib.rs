use std::process::Command;
use std::sync::{Arc, Mutex};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Window, State, AppHandle};

#[derive(Debug, Serialize, Deserialize, Clone)]
struct DownloadProgress {
    percentage: f64,
    speed: String,
    eta: String,
    status: String,
}

type ProgressState = Arc<Mutex<DownloadProgress>>;

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
) -> Result<(), String> {
    let window_clone = window.clone();
    let progress_arc = progress_state.inner().clone();
    let url_clone = url.clone();
    let download_type_clone = downloadType.clone();
    let quality_clone = quality.clone();
    let output_folder_clone = outputFolder.clone();

    tokio::spawn(async move {
        let result = perform_download(
            &window_clone,
            progress_arc.clone(),
            &url_clone,
            &download_type_clone,
            &quality_clone,
            &output_folder_clone,
        ).await;

        match result {
            Ok(_) => {
                let mut progress = progress_arc.lock().unwrap();
                progress.status = "completed".to_string();
                progress.percentage = 100.0;
                let progress_copy = progress.clone();
                let _ = window_clone.emit("download-progress", progress_copy);
            }
            Err(e) => {
                let mut progress = progress_arc.lock().unwrap();
                progress.status = "error".to_string();
                eprintln!("Download error: {}", e);
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
            results.push(format!("✅ aria2c: {}", version.lines().next().unwrap_or("unknown")));
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
) -> Result<(), String> {
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
            eprintln!("aria2c version: {}", version.lines().next().unwrap_or("unknown"));
        }
        Err(e) => {
            return Err(format!("aria2c not found or not executable: {}. Please install with: sudo apt install aria2", e));
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
       .arg("--prefer-free-formats")
       .arg("-o")
       .arg(format!("{}/%(title)s.%(ext)s", output_folder));

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

    cmd.arg(url);

    // Log the full command for debugging
    eprintln!("Executing command: {:?}", cmd);
    
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start yt-dlp: {}. Make sure yt-dlp and aria2c are installed.", e))?;

    // Monitor the process output
    if let Some(stdout) = child.stdout.take() {
        use std::io::{BufRead, BufReader};
        let reader = BufReader::new(stdout);
        
        let progress_regex = Regex::new(r"\[download\]\s+(\d+\.?\d*)%.*?(\S+/s).*?ETA\s+(\S+)").unwrap();

        for line in reader.lines() {
            if let Ok(line) = line {
                if let Some(captures) = progress_regex.captures(&line) {
                    let percentage: f64 = captures.get(1).unwrap().as_str().parse().unwrap_or(0.0);
                    let speed = captures.get(2).unwrap().as_str().to_string();
                    let eta = captures.get(3).unwrap().as_str().to_string();

                    {
                        let mut progress = progress_state.lock().unwrap();
                        progress.percentage = percentage;
                        progress.speed = speed;
                        progress.eta = eta;
                        progress.status = "downloading".to_string();
                    }

                    let progress_copy = {
                        let progress = progress_state.lock().unwrap();
                        progress.clone()
                    };

                    let _ = window.emit("download-progress", progress_copy);
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
        Ok(())
    } else {
        let exit_code = output.code().unwrap_or(-1);
        let error_msg = if !stderr_output.is_empty() {
            format!("yt-dlp failed (exit code {}): {}", exit_code, stderr_output.trim())
        } else {
            format!("yt-dlp failed with exit code {}", exit_code)
        };
        eprintln!("Download failed: {}", error_msg);
        Err(error_msg)
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let progress_state: ProgressState = Arc::new(Mutex::new(DownloadProgress {
        percentage: 0.0,
        speed: String::new(),
        eta: String::new(),
        status: "idle".to_string(),
    }));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .manage(progress_state)
        .invoke_handler(tauri::generate_handler![select_output_folder, start_download, test_dependencies])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
